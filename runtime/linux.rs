// What the runtime asks of Linux: system calls made directly (no C library), its own entry point
// `_start`, and its own memory allocator. The executable is then static: it runs on any Linux.
// On x86-64 and on arm64: only the system calls' numbers, how they are made and `_start` differ.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::ptr::{null, null_mut};
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use crate::http::{Fail, Network, Serve};

use crate::{Global, Locked, c_text};

pub const SLASH: char = '/';

#[cfg(target_arch = "x86_64")]
unsafe fn call(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let r;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b, in("rdx") c, in("r10") d, in("r8") e, in("r9") f, lateout("rcx") _, lateout("r11") _, options(nostack))
    };
    r
}

#[cfg(target_arch = "aarch64")]
unsafe fn call(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let r;
    unsafe { asm!("svc 0", in("x8") n, inlateout("x0") a as isize => r, in("x1") b, in("x2") c, in("x3") d, in("x4") e, in("x5") f, options(nostack)) };
    r
}

#[cfg(target_arch = "x86_64")]
mod numbers {
    pub const READ: usize = 0;
    pub const WRITE: usize = 1;
    pub const CLOSE: usize = 3;
    pub const MMAP: usize = 9;
    pub const MUNMAP: usize = 11;
    pub const IOCTL: usize = 16;
    pub const MREMAP: usize = 25;
    pub const NANOSLEEP: usize = 35;
    pub const GETPID: usize = 39;
    pub const GETDENTS64: usize = 217;
    pub const CLOCK_GETTIME: usize = 228;
    pub const EXIT_GROUP: usize = 231;
    pub const OPENAT: usize = 257;
    pub const MKDIRAT: usize = 258;
    pub const UNLINKAT: usize = 263;
    pub const STATX: usize = 332;
    pub const GETRANDOM: usize = 318;
    pub const RENAMEAT2: usize = 316;
    pub const PIPE2: usize = 293;
    pub const DUP3: usize = 292;
    pub const EXECVE: usize = 59;
    pub const WAIT4: usize = 61;
    pub const SOCKET: usize = 41;
    pub const SETSOCKOPT: usize = 54;
    pub const CONNECT: usize = 42;
    pub const SENDTO: usize = 44;
    pub const RECVFROM: usize = 45;
    pub const BIND: usize = 49;
    pub const SHUTDOWN: usize = 48;
    pub const GETSOCKNAME: usize = 51;
    pub const LISTEN: usize = 50;
    pub const ACCEPT4: usize = 288;
    pub const PPOLL: usize = 271;
    pub const MPROTECT: usize = 10;
    pub const CLONE: usize = 56;
    pub const EXIT: usize = 60;
    pub const FUTEX: usize = 202;
    pub const SCHED_GETAFFINITY: usize = 204;
    pub const O_DIRECTORY: usize = 0o200000;
}

// Linux's own numbering for the newer systems (`asm-generic/unistd.h`).
#[cfg(target_arch = "aarch64")]
mod numbers {
    pub const READ: usize = 63;
    pub const WRITE: usize = 64;
    pub const CLOSE: usize = 57;
    pub const MMAP: usize = 222;
    pub const MUNMAP: usize = 215;
    pub const IOCTL: usize = 29;
    pub const MREMAP: usize = 216;
    pub const NANOSLEEP: usize = 101;
    pub const GETPID: usize = 172;
    pub const GETDENTS64: usize = 61;
    pub const CLOCK_GETTIME: usize = 113;
    pub const EXIT_GROUP: usize = 94;
    pub const OPENAT: usize = 56;
    pub const MKDIRAT: usize = 34;
    pub const UNLINKAT: usize = 35;
    pub const STATX: usize = 291;
    pub const GETRANDOM: usize = 278;
    pub const RENAMEAT2: usize = 276;
    pub const PIPE2: usize = 59;
    pub const DUP3: usize = 24;
    pub const EXECVE: usize = 221;
    pub const WAIT4: usize = 260;
    pub const SOCKET: usize = 198;
    pub const SETSOCKOPT: usize = 208;
    pub const CONNECT: usize = 203;
    pub const SENDTO: usize = 206;
    pub const RECVFROM: usize = 207;
    pub const BIND: usize = 200;
    pub const SHUTDOWN: usize = 210;
    pub const GETSOCKNAME: usize = 204;
    pub const LISTEN: usize = 201;
    pub const ACCEPT4: usize = 242;
    pub const PPOLL: usize = 73;
    pub const MPROTECT: usize = 226;
    pub const CLONE: usize = 220;
    pub const EXIT: usize = 93;
    pub const FUTEX: usize = 98;
    pub const SCHED_GETAFFINITY: usize = 123;
    pub const O_DIRECTORY: usize = 0o40000;
}

use numbers::*;

const AT_FDCWD: usize = -100isize as usize;
const EINTR: isize = 4;
const EEXIST: isize = 17;

// The program starts here: the system put the number of words, then the words (`argv`), then
// the environment (`envp`) on the stack.
#[cfg(target_arch = "x86_64")]
global_asm!(".globl _start", "_start:", "xor ebp, ebp", "mov rdi, rsp", "and rsp, -16", "call {start}", "ud2", start = sym start);
#[cfg(target_arch = "aarch64")]
global_asm!(".globl _start", "_start:", "mov x29, xzr", "mov x30, xzr", "mov x0, sp", "bl {start}", "brk #0", start = sym start);

// A program using the web is linked with the C library (OpenSSL needs it): it starts here
// instead, so that the C library starts first, then calls `libc_main` (src/link.rs `elf`).
#[cfg(target_arch = "x86_64")]
global_asm!(
    // A section of its own: kept only when it is the entry (`.text` holds `_start`).
    ".pushsection .text.kova_libc_start, \"ax\"",
    ".globl kova_libc_start",
    "kova_libc_start:",
    "xor ebp, ebp",
    "mov r9, rdx", // what to call at the end, from the dynamic linker
    "pop rsi",     // argc
    "mov rdx, rsp", // argv
    "and rsp, -16",
    "push rax",
    "push rsp", // the top of the stack
    "xor r8d, r8d",
    "xor ecx, ecx",
    "lea rdi, [rip + {main}]",
    "call __libc_start_main",
    "ud2",
    ".popsection",
    main = sym libc_main,
);
#[cfg(target_arch = "aarch64")]
global_asm!(
    ".pushsection .text.kova_libc_start, \"ax\"",
    ".globl kova_libc_start",
    "kova_libc_start:",
    "mov x29, xzr",
    "mov x30, xzr",
    "mov x5, x0", // what to call at the end, from the dynamic linker
    "ldr x1, [sp]", // argc
    "add x2, sp, #8", // argv
    "mov x6, sp", // the top of the stack
    "adrp x0, {main}",
    "add x0, x0, :lo12:{main}",
    "mov x3, xzr",
    "mov x4, xzr",
    "bl __libc_start_main",
    "brk #0",
    ".popsection",
    main = sym libc_main,
);

/// Called by the C library, with the words the system put on the stack: `argc` just before them.
unsafe extern "C" fn libc_main(_argc: i32, argv: *const usize) -> ! {
    unsafe extern "C" {
        fn dlopen(name: *const u8, flags: i32) -> *mut u8;
        fn dlsym(library: *mut u8, name: *const u8) -> *mut u8;
    }
    // Its threads are made by it (`spawn`): it keeps a state for each (`errno`, its memory).
    // In `libc.so.6` since glibc 2.34, in `libpthread.so.0` before. 0 = everything loaded.
    unsafe {
        let mut f = dlsym(null_mut(), c"pthread_create".as_ptr() as *const u8);
        if f.is_null() {
            f = dlsym(dlopen(c"libpthread.so.0".as_ptr() as *const u8, 2), c"pthread_create".as_ptr() as *const u8);
        }
        *PTHREAD_CREATE.get() = f as usize;
        let mut join = dlsym(null_mut(), c"pthread_join".as_ptr() as *const u8);
        if join.is_null() {
            join = dlsym(dlopen(c"libpthread.so.0".as_ptr() as *const u8, 2), c"pthread_join".as_ptr() as *const u8);
        }
        *PTHREAD_JOIN.get() = join as usize;
        start(argv.sub(1))
    }
}

/// The C library's `pthread_create` and `pthread_join`, when it started the program
/// (`libc_main`), or 0.
static PTHREAD_CREATE: Global<usize> = Global::new(0);
static PTHREAD_JOIN: Global<usize> = Global::new(0);

/// The environment's `NAME=value` texts.
static ENVIRONMENT: Global<&[*const u8]> = Global::new(&[]);

unsafe extern "C" fn start(stack: *const usize) -> ! {
    unsafe {
        let argc = *stack;
        let argv = stack.add(1) as *const *const u8;
        *crate::ARGS.get() = (0..argc).map(|i| String::from_utf8_lossy(c_text(*argv.add(i))).into_owned()).collect();
        let envp = argv.add(argc + 1);
        let mut n = 0;
        while !(*envp.add(n)).is_null() {
            n += 1;
        }
        *ENVIRONMENT.get() = core::slice::from_raw_parts(envp, n);
    }
    crate::run()
}

/// The environment's variables say it (`LANG`).
pub fn system_language() -> Option<String> {
    None
}

pub fn var(name: &str) -> Option<String> {
    ENVIRONMENT.get().iter().find_map(|&e| {
        let e = unsafe { c_text(e) };
        let value = e.strip_prefix(name.as_bytes())?.strip_prefix(b"=")?;
        Some(String::from_utf8_lossy(value).into_owned())
    })
}

pub fn exit(code: i32) -> ! {
    unsafe { call(EXIT_GROUP, code as usize, 0, 0, 0, 0, 0) };
    unreachable!()
}

pub fn write(fd: usize, mut b: &[u8]) {
    while !b.is_empty() {
        match unsafe { call(WRITE, fd, b.as_ptr() as usize, b.len(), 0, 0, 0) } {
            n if n > 0 => b = &b[n as usize..],
            n if n == -EINTR => {}
            _ => return,
        }
    }
}

/// What was read (0 at the end, or after an error).
pub fn read(fd: usize, b: &mut [u8]) -> usize {
    loop {
        match unsafe { call(READ, fd, b.as_mut_ptr() as usize, b.len(), 0, 0, 0) } {
            n if n == -EINTR => {}
            n => return n.max(0) as usize,
        }
    }
}

pub fn is_terminal(fd: usize) -> bool {
    let mut settings = [0u8; 64];
    unsafe { call(IOCTL, fd, 0x5401, settings.as_mut_ptr() as usize, 0, 0, 0) == 0 } // TCGETS
}

/// The processors this program may use.
pub fn cores() -> i64 {
    let mut mask = [0u64; 128];
    let got = unsafe { call(SCHED_GETAFFINITY, 0, 1024, mask.as_mut_ptr() as usize, 0, 0, 0) };
    if (got as isize) <= 0 { 1 } else { mask.iter().map(|m| m.count_ones() as i64).sum::<i64>().max(1) }
}

/// The nanoseconds since 1970 (UTC).
pub fn now() -> u64 {
    let mut t = [0u64; 2];
    unsafe { call(CLOCK_GETTIME, 0, t.as_mut_ptr() as usize, 0, 0, 0, 0) }; // CLOCK_REALTIME
    t[0] * 1_000_000_000 + t[1]
}

pub fn sleep(s: f64) {
    let mut t = [s as u64, ((s - (s as u64) as f64) * 1e9) as u64];
    while unsafe { call(NANOSLEEP, t.as_ptr() as usize, t.as_mut_ptr() as usize, 0, 0, 0, 0) } == -EINTR {}
}

pub fn pid() -> u64 {
    unsafe { call(GETPID, 0, 0, 0, 0, 0, 0) as u64 }
}

/// Fills `b` with the system's unguessable chance (`secure.random`).
pub fn chance(mut b: &mut [u8]) {
    while !b.is_empty() {
        let n = unsafe { call(GETRANDOM, b.as_mut_ptr() as usize, b.len(), 0, 0, 0, 0) };
        if n > 0 {
            b = &mut b[n as usize..];
        } else if n != -EINTR {
            panic!("getrandom");
        }
    }
}

// ---- The network: the C library finds the addresses (`getaddrinfo` reads /etc/hosts, asks the
// name servers…), so a program using it is dynamic (src/link.rs `elf`); the rest are system calls.

unsafe extern "C" {
    fn getaddrinfo(host: *const u8, port: *const u8, hints: *const AddrInfo, found: *mut *mut AddrInfo) -> i32;
    fn freeaddrinfo(found: *mut AddrInfo);
}

#[repr(C)]
struct AddrInfo {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addrlen: u32,
    addr: *const u8,
    canonname: *const u8,
    next: *mut AddrInfo,
}

const SOCK_STREAM: i32 = 1;
const ETIMEDOUT: isize = 110;
const EAGAIN: isize = 11;

pub struct Net;

impl Network for Net {
    type Link = usize;

    /// Never `secure`: src/openssl.rs encrypts around it.
    fn open(&mut self, host: &str, port: u16, _: bool, ms: u64) -> Result<usize, Fail> {
        if host.contains('\0') {
            return Err(Fail::NoHost);
        }
        let (host, port) = (c_path(host), format!("{port}\0"));
        let hints = AddrInfo { flags: 0, family: 0, socktype: SOCK_STREAM, protocol: 0, addrlen: 0, addr: null(), canonname: null(), next: null_mut() };
        let mut found = null_mut();
        if unsafe { getaddrinfo(host.as_ptr(), port.as_ptr(), &hints, &mut found) } != 0 {
            return Err(Fail::NoHost);
        }
        let (mut failure, mut a) = (Err(Fail::NoHost), found);
        while let Some(i) = unsafe { a.as_ref() } {
            let fd = unsafe { call(SOCKET, i.family as usize, SOCK_STREAM as usize | 0o2000000, i.protocol as usize, 0, 0, 0) };
            if fd >= 0 {
                if ms > 0 {
                    // SO_RCVTIMEO, SO_SNDTIMEO (which `connect` follows too): seconds, microseconds.
                    let limit = [(ms / 1000) as i64, (ms % 1000 * 1000) as i64];
                    for option in [20, 21] {
                        unsafe { call(SETSOCKOPT, fd as usize, 1, option, limit.as_ptr() as usize, 16, 0) };
                    }
                }
                match unsafe { call(CONNECT, fd as usize, i.addr as usize, i.addrlen as usize, 0, 0, 0) } {
                    0 => {
                        failure = Ok(fd as usize);
                        break;
                    }
                    // EINPROGRESS: the time limit came first.
                    e => failure = Err(if e == -ETIMEDOUT || e == -115 { Fail::Timeout } else { Fail::Refused }),
                }
                close(fd as usize);
            }
            a = i.next;
        }
        unsafe { freeaddrinfo(found) };
        failure
    }

    fn send(&mut self, fd: &mut usize, mut data: &[u8]) -> Result<(), Fail> {
        while !data.is_empty() {
            // MSG_NOSIGNAL: a closed connection is a failure, not the end of the program.
            match unsafe { call(SENDTO, *fd, data.as_ptr() as usize, data.len(), 0x4000, 0, 0) } {
                e if e == -EINTR => {}
                n if n >= 0 => data = &data[n as usize..],
                e => return Err(cut(e)),
            }
        }
        Ok(())
    }

    fn receive(&mut self, fd: &mut usize, buffer: &mut [u8]) -> Result<usize, Fail> {
        loop {
            match unsafe { call(RECVFROM, *fd, buffer.as_mut_ptr() as usize, buffer.len(), 0, 0, 0) } {
                e if e == -EINTR => {}
                n if n >= 0 => return Ok(n as usize),
                e => return Err(cut(e)),
            }
        }
    }

    fn wake(&mut self, fd: &usize) {
        unsafe { call(SHUTDOWN, *fd, 2, 0, 0, 0, 0) };
    }

    fn close(&mut self, fd: usize) {
        close(fd);
    }
}

/// The port of a socket: a `sockaddr_in` holds it after the family.
fn port_of(fd: usize) -> u16 {
    let (mut a, mut n) = ([0u8; 16], 16u32);
    unsafe { call(GETSOCKNAME, fd, a.as_mut_ptr() as usize, &mut n as *mut u32 as usize, 0, 0, 0) };
    u16::from_be_bytes([a[2], a[3]])
}

/// The first IPv4 address of `host`, for UDP.
fn address_of(host: &str, port: u16) -> Result<([u8; 16], u32), Fail> {
    if host.contains('\0') {
        return Err(Fail::NoHost);
    }
    let (host, port) = (c_path(host), format!("{port}\0"));
    let hints = AddrInfo { flags: 0, family: 2, socktype: 2, protocol: 0, addrlen: 0, addr: null(), canonname: null(), next: null_mut() };
    let mut found = null_mut();
    if unsafe { getaddrinfo(host.as_ptr(), port.as_ptr(), &hints, &mut found) } != 0 {
        return Err(Fail::NoHost);
    }
    let Some(i) = (unsafe { found.as_ref() }) else { return Err(Fail::NoHost) };
    let mut a = [0u8; 16];
    a.copy_from_slice(unsafe { core::slice::from_raw_parts(i.addr, 16) });
    let n = i.addrlen;
    unsafe { freeaddrinfo(found) };
    Ok((a, n))
}

impl Serve for Net {
    type Listener = usize;
    type Datagram = usize;

    fn stop(&mut self, fd: usize) {
        close(fd);
    }

    fn port(&mut self, fd: &usize) -> u16 {
        port_of(*fd)
    }

    fn datagram_port(&mut self, fd: &usize) -> u16 {
        port_of(*fd)
    }

    fn udp(&mut self, port: u16, everyone: bool, ms: u64) -> Option<usize> {
        let fd = unsafe { call(SOCKET, 2, 2 | 0o2000000, 0, 0, 0, 0) };
        if fd < 0 {
            return None;
        }
        let address = sockaddr_in(port, everyone);
        if unsafe { call(BIND, fd as usize, address.as_ptr() as usize, 16, 0, 0, 0) } != 0 {
            close(fd as usize);
            return None;
        }
        let limit = [(ms / 1000) as i64, (ms % 1000 * 1000) as i64];
        unsafe { call(SETSOCKOPT, fd as usize, 1, 20, limit.as_ptr() as usize, 16, 0) };
        Some(fd as usize)
    }

    fn send_to(&mut self, fd: &usize, host: &str, port: u16, data: &[u8]) -> Result<(), Fail> {
        let (a, n) = address_of(host, port)?;
        match unsafe { call(SENDTO, *fd, data.as_ptr() as usize, data.len(), 0x4000, a.as_ptr() as usize, n as usize) } {
            e if e < 0 => Err(cut(e)),
            _ => Ok(()),
        }
    }

    fn receive_from(&mut self, fd: &usize, buffer: &mut [u8]) -> Result<(usize, [u8; 4], u16), Fail> {
        let (mut from, mut n) = ([0u8; 16], 16u32);
        loop {
            match unsafe { call(RECVFROM, *fd, buffer.as_mut_ptr() as usize, buffer.len(), 0, from.as_mut_ptr() as usize, &mut n as *mut u32 as usize) } {
                e if e == -EINTR => {}
                got if got >= 0 => return Ok((got as usize, [from[4], from[5], from[6], from[7]], u16::from_be_bytes([from[2], from[3]]))),
                e => return Err(cut(e)),
            }
        }
    }

    fn close_datagram(&mut self, fd: usize) {
        close(fd);
    }

    fn listen(&mut self, port: u16, everyone: bool) -> Option<usize> {
        let fd = unsafe { call(SOCKET, 2, SOCK_STREAM as usize | 0o2000000, 0, 0, 0, 0) };
        if fd < 0 {
            return None;
        }
        // SO_REUSEADDR: the port can be taken again right after a server stops.
        unsafe { call(SETSOCKOPT, fd as usize, 1, 2, &1i32 as *const i32 as usize, 4, 0) };
        let address = sockaddr_in(port, everyone);
        let bound = unsafe { call(BIND, fd as usize, address.as_ptr() as usize, 16, 0, 0, 0) } == 0;
        if !bound || unsafe { call(LISTEN, fd as usize, 128, 0, 0, 0, 0) } != 0 {
            close(fd as usize);
            return None;
        }
        Some(fd as usize)
    }

    fn accept(&mut self, listener: &mut usize, ms: u64) -> Option<(usize, [u8; 4])> {
        let (mut from, mut n) = ([0u8; 16], 16u32);
        let fd = unsafe { call(ACCEPT4, *listener, from.as_mut_ptr() as usize, &mut n as *mut u32 as usize, 0o2000000, 0, 0) };
        if fd < 0 {
            return None;
        }
        let limit = [(ms / 1000) as i64, (ms % 1000 * 1000) as i64];
        for option in [20, 21] {
            unsafe { call(SETSOCKOPT, fd as usize, 1, option, limit.as_ptr() as usize, 16, 0) };
        }
        Some((fd as usize, [from[4], from[5], from[6], from[7]]))
    }

    /// SIOCGIFCONF, so that a server needs no C library: `struct ifconf` (its length, then where
    /// the `struct ifreq`s go, 40 bytes each: the name, then a `sockaddr_in`).
    fn addresses(&mut self) -> Vec<(String, [u8; 4])> {
        let fd = unsafe { call(SOCKET, 2, 2 | 0o2000000, 0, 0, 0, 0) };
        if fd < 0 {
            return Vec::new();
        }
        let mut found = alloc::vec![0u8; 40 * 64];
        let mut conf = [found.len(), found.as_mut_ptr() as usize];
        let asked = unsafe { call(IOCTL, fd as usize, 0x8912, conf.as_mut_ptr() as usize, 0, 0, 0) };
        close(fd as usize);
        if asked != 0 {
            return Vec::new();
        }
        let n = (conf[0] as u32 as usize / 40).min(64);
        let named = |r: &[u8]| String::from_utf8_lossy(&r[..r[..16].iter().position(|&b| b == 0).unwrap_or(16)]).into_owned();
        found.chunks(40).take(n).map(|r| (named(r), [r[20], r[21], r[22], r[23]])).collect()
    }

    /// `ppoll` (arm64 has no `poll`), without a time limit: `struct pollfd`, POLLIN.
    fn ready(&mut self, listener: &usize, links: &[usize]) -> Vec<bool> {
        let mut fds: Vec<[i32; 2]> = links.iter().chain([listener]).map(|&fd| [fd as i32, 1]).collect();
        if unsafe { call(PPOLL, fds.as_mut_ptr() as usize, fds.len(), 0, 0, 8, 0) } <= 0 {
            return alloc::vec![false; fds.len()];
        }
        // `revents` is the upper half of the second number.
        fds.iter().map(|p| p[1] >> 16 != 0).collect()
    }
}

/// 127.0.0.1 (0.0.0.0 = every address for `everyone`), `port`, as the system wants it: family,
/// port (big end first), address, 8 zeros.
fn sockaddr_in(port: u16, everyone: bool) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[0] = 2;
    a[2..4].copy_from_slice(&port.to_be_bytes());
    if !everyone {
        a[4..8].copy_from_slice(&[127, 0, 0, 1]);
    }
    a
}

/// Like `cut` in src/web.rs.
fn cut(e: isize) -> Fail {
    if e == -ETIMEDOUT || e == -EAGAIN { Fail::Timeout } else { Fail::Broken }
}

// ---- Files ----

/// A system failure as an `IoError` variant + 1, like `failure` in src/interp.rs.
fn failure(e: isize) -> i64 {
    match -e {
        2 | 20 => 1,       // ENOENT, ENOTDIR
        1 | 13 | 30 => 2,  // EPERM, EACCES, EROFS
        21 => 3,           // EISDIR
        _ => 5,
    }
}

/// The path with the 0 byte the system wants after it.
fn c_path(path: &str) -> Vec<u8> {
    [path.as_bytes(), &[0]].concat()
}

fn open(path: &str, flags: usize) -> Result<usize, i64> {
    let p = c_path(path);
    match unsafe { call(OPENAT, AT_FDCWD, p.as_ptr() as usize, flags | 0o2000000, 0o666, 0, 0) } {
        fd if fd >= 0 => Ok(fd as usize),
        e => Err(failure(e)),
    }
}

fn close(fd: usize) -> isize {
    unsafe { call(CLOSE, fd, 0, 0, 0, 0, 0) }
}

/// Ok(true) for a directory, Ok(false) for anything else that is there.
pub fn kind(path: &str) -> Result<bool, i64> {
    let p = c_path(path);
    let mut st = [0u8; 256];
    match unsafe { call(STATX, AT_FDCWD, p.as_ptr() as usize, 0, 1, st.as_mut_ptr() as usize, 0) } {
        0 => Ok(u16::from_ne_bytes([st[0x1C], st[0x1D]]) & 0o170000 == 0o040000),
        e => Err(failure(e)),
    }
}

/// What `files.size` and `files.modified` say; `follow` = a link's target rather than the link.
pub fn info(path: &str, follow: bool) -> Result<crate::Info, i64> {
    let p = c_path(path);
    let mut st = [0u8; 256];
    // STATX_TYPE | STATX_MODE | STATX_MTIME | STATX_SIZE, AT_SYMLINK_NOFOLLOW.
    let flags = if follow { 0 } else { 0x100 };
    match unsafe { call(STATX, AT_FDCWD, p.as_ptr() as usize, flags, 0x243, st.as_mut_ptr() as usize, 0) } {
        0 => {
            let kind = u16::from_ne_bytes([st[0x1C], st[0x1D]]) & 0o170000;
            let number = |at: usize| i64::from_ne_bytes(st[at..at + 8].try_into().unwrap());
            Ok(crate::Info { folder: kind == 0o040000, link: kind == 0o120000, size: number(0x28), modified: number(0x70) })
        }
        e => Err(failure(e)),
    }
}

/// `a` named `b` from now on; Err(0) if they are on two disks (EXDEV).
pub fn rename(a: &str, b: &str) -> Result<(), i64> {
    let (a, b) = (c_path(a), c_path(b));
    match unsafe { call(RENAMEAT2, AT_FDCWD, a.as_ptr() as usize, AT_FDCWD, b.as_ptr() as usize, 0, 0) } {
        0 => Ok(()),
        -18 => Err(0),
        e => Err(failure(e)),
    }
}

pub fn read_file(path: &str) -> Result<Vec<u8>, i64> {
    let fd = open(path, 0)?;
    let mut bytes = Vec::new();
    // In growing pieces: the size the system gives may be wrong (`/proc`, a pipe).
    let r = loop {
        bytes.reserve(1 << 16);
        let free = bytes.spare_capacity_mut();
        match unsafe { call(READ, fd, free.as_mut_ptr() as usize, free.len(), 0, 0, 0) } {
            0 => break Ok(bytes),
            n if n > 0 => unsafe { bytes.set_len(bytes.len() + n as usize) },
            n if n == -EINTR => {}
            _ => break Err(5),
        }
    };
    close(fd);
    r
}

pub fn write_file(path: &str, mut b: &[u8], append: bool) -> Result<(), i64> {
    // O_WRONLY | O_CREAT, then O_APPEND or O_TRUNC.
    let fd = open(path, 0o1 | 0o100 | if append { 0o2000 } else { 0o1000 })?;
    let mut r = Ok(());
    while !b.is_empty() {
        match unsafe { call(WRITE, fd, b.as_ptr() as usize, b.len(), 0, 0, 0) } {
            n if n > 0 => b = &b[n as usize..],
            n if n == -EINTR => {}
            _ => {
                r = Err(5);
                break;
            }
        }
    }
    if close(fd) != 0 && r.is_ok() {
        r = Err(5);
    }
    r
}

/// One directory; Err(0) if something is already there.
pub fn make_dir(path: &str) -> Result<(), i64> {
    let p = c_path(path);
    match unsafe { call(MKDIRAT, AT_FDCWD, p.as_ptr() as usize, 0o777, 0, 0, 0) } {
        0 => Ok(()),
        e if e == -EEXIST => Err(0),
        e => Err(failure(e)),
    }
}

pub fn remove_file(path: &str) -> Result<(), i64> {
    let p = c_path(path);
    match unsafe { call(UNLINKAT, AT_FDCWD, p.as_ptr() as usize, 0, 0, 0, 0) } {
        0 => Ok(()),
        e => Err(failure(e)),
    }
}

/// The names in a directory, without `.` and `..`; Err(4) if one is not UTF-8.
pub fn list_dir(path: &str) -> Result<Vec<String>, i64> {
    let fd = open(path, O_DIRECTORY)?;
    let mut names = Vec::new();
    let mut buffer = [0u8; 1 << 14];
    let r = loop {
        let n = unsafe { call(GETDENTS64, fd, buffer.as_mut_ptr() as usize, buffer.len(), 0, 0, 0) };
        if n == -EINTR {
            continue;
        }
        if n <= 0 {
            break if n == 0 { Ok(names) } else { Err(5) };
        }
        // Each entry: inode (8 bytes), place (8), its own length (2), kind (1), name and a 0.
        let mut at = 0;
        while at < n as usize {
            let size = u16::from_ne_bytes([buffer[at + 16], buffer[at + 17]]) as usize;
            let name = unsafe { c_text(buffer[at + 19..].as_ptr()) };
            at += size;
            if name == b"." || name == b".." {
                continue;
            }
            match String::from_utf8(name.to_vec()) {
                Ok(name) => names.push(name),
                Err(_) => {
                    close(fd);
                    return Err(4);
                }
            }
        }
    };
    close(fd);
    r
}

// ---- Other programs ----

/// `system.run`: runs `program` (a path) with `args`, its input empty, and gives what it wrote,
/// what it wrote on stderr, and its exit status (`shared::exit_code`); or the `RunError` variant
/// + 1. Everything the new process needs is made before it exists: it then only calls the
/// system, never the memory functions, whose lock another thread may have held at that moment.
pub fn run(program: &str, args: &[&str]) -> Result<(Vec<u8>, Vec<u8>, i64), i64> {
    const O_CLOEXEC: usize = 0o2000000;
    let words: Vec<Vec<u8>> = [program].iter().chain(args).map(|w| c_path(w)).collect();
    let mut argv: Vec<*const u8> = words.iter().map(|w| w.as_ptr()).collect();
    argv.push(null());
    let mut envp: Vec<*const u8> = ENVIRONMENT.get().to_vec();
    envp.push(null());
    let null = open("/dev/null", 0).map_err(|_| 3i64)?;
    // What it writes, what it writes on stderr, and whether `execve` failed (closed if not).
    let mut pipes = [[0i32; 2]; 3];
    for p in &mut pipes {
        if unsafe { call(PIPE2, p.as_mut_ptr() as usize, O_CLOEXEC, 0, 0, 0, 0) } != 0 {
            return Err(3);
        }
    }
    let [out, err, failed] = pipes.map(|[r, w]| (r as usize, w as usize));
    // A copy of this process (fork), which becomes the program.
    let pid = unsafe { call(CLONE, 17, 0, 0, 0, 0, 0) }; // SIGCHLD
    if pid == 0 {
        unsafe {
            call(DUP3, null, 0, 0, 0, 0, 0);
            call(DUP3, out.1, 1, 0, 0, 0, 0);
            call(DUP3, err.1, 2, 0, 0, 0, 0);
            let e = call(EXECVE, argv[0] as usize, argv.as_ptr() as usize, envp.as_ptr() as usize, 0, 0, 0);
            call(WRITE, failed.1, &e as *const isize as usize, 8, 0, 0, 0);
            call(EXIT_GROUP, 127, 0, 0, 0, 0, 0);
        }
    }
    for fd in [null, out.1, err.1, failed.1] {
        close(fd);
    }
    if pid < 0 {
        for fd in [out.0, err.0, failed.0] {
            close(fd);
        }
        return Err(3);
    }
    let mut e = 0isize;
    let started = unsafe { call(READ, failed.0, &mut e as *mut isize as usize, 8, 0, 0, 0) } != 8;
    close(failed.0);
    // Both at once: a program filling one pipe while this one waits on the other would stop.
    let mut texts = [Vec::new(), Vec::new()];
    let mut open = [started, started];
    let mut buffer = [0u8; 1 << 14];
    while open.contains(&true) {
        let mut fds: Vec<[i32; 2]> = [out.0, err.0].iter().zip(open).filter(|(_, o)| *o).map(|(&fd, _)| [fd as i32, 1]).collect();
        let n = unsafe { call(PPOLL, fds.as_mut_ptr() as usize, fds.len(), 0, 0, 8, 0) };
        if n == -EINTR {
            continue;
        }
        for p in &fds {
            if p[1] >> 16 == 0 && n >= 0 {
                continue;
            }
            let k = if p[0] as usize == out.0 { 0 } else { 1 };
            match unsafe { call(READ, p[0] as usize, buffer.as_mut_ptr() as usize, buffer.len(), 0, 0, 0) } {
                r if r > 0 => texts[k].extend_from_slice(&buffer[..r as usize]),
                r if r == -EINTR => {}
                _ => open[k] = false,
            }
        }
    }
    close(out.0);
    close(err.0);
    let mut status = 0i32;
    while unsafe { call(WAIT4, pid as usize, &mut status as *mut i32 as usize, 0, 0, 0, 0) } == -EINTR {}
    if !started {
        return Err(match -e {
            2 | 20 => 1,     // ENOENT, ENOTDIR
            1 | 13 => 2,     // EPERM, EACCES
            _ => 3,
        });
    }
    let [output, errors] = texts;
    Ok((output, errors, crate::shared::exit_code(status)))
}

// ---- Memory ----
// Blocks of 16 bytes to 64 KB, in powers of two: each size has its list of freed blocks, new ones
// are cut from 1 MB pieces asked of the system. Bigger blocks come straight from the system.
// ponytail: a freed block is kept for its size only, never given back nor merged; a program that
// frees many small blocks then needs big ones keeps the small ones' memory.

const SIZES: usize = 13;
const LARGEST: usize = 16 << (SIZES - 1);
const PIECE: usize = 1 << 20;

struct Blocks {
    freed: [*mut u8; SIZES],
    next: usize,
    end: usize,
}

static BLOCKS: Locked<Blocks> = Locked::new(Blocks { freed: [core::ptr::null_mut(); SIZES], next: 0, end: 0 });

/// The number of the size of a block holding `n` bytes.
fn size_of(n: usize) -> usize {
    n.max(16).next_power_of_two().trailing_zeros() as usize - 4
}

fn pages(n: usize) -> usize {
    (n + 4095) & !4095
}

fn system_memory(n: usize) -> *mut u8 {
    // PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS.
    match unsafe { call(MMAP, 0, n, 3, 0x22, usize::MAX, 0) } {
        p if p < 0 => core::ptr::null_mut(),
        p => p as *mut u8,
    }
}

pub struct Memory;

unsafe impl GlobalAlloc for Memory {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        debug_assert!(l.align() <= 16);
        if l.size() > LARGEST {
            return system_memory(pages(l.size()));
        }
        let k = size_of(l.size());
        BLOCKS.with(|b| {
            let block = b.freed[k];
            if !block.is_null() {
                b.freed[k] = unsafe { *(block as *mut *mut u8) };
                return block;
            }
            if b.next + (16 << k) > b.end {
                let piece = system_memory(PIECE);
                if piece.is_null() {
                    return piece;
                }
                b.next = piece as usize;
                b.end = b.next + PIECE;
            }
            b.next += 16 << k;
            (b.next - (16 << k)) as *mut u8
        })
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if l.size() > LARGEST {
            unsafe { call(MUNMAP, p as usize, pages(l.size()), 0, 0, 0, 0) };
            return;
        }
        let k = size_of(l.size());
        BLOCKS.with(|b| {
            unsafe { *(p as *mut *mut u8) = b.freed[k] };
            b.freed[k] = p;
        })
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let old = l.size();
        if old > LARGEST && n > LARGEST {
            // MREMAP_MAYMOVE: the system moves the pages without copying them.
            return match unsafe { call(MREMAP, p as usize, pages(old), pages(n), 1, 0, 0) } {
                q if q < 0 => core::ptr::null_mut(),
                q => q as *mut u8,
            };
        }
        if old <= LARGEST && n <= LARGEST && size_of(old) == size_of(n) {
            return p;
        }
        unsafe {
            let q = self.alloc(Layout::from_size_align_unchecked(n, l.align()));
            if !q.is_null() {
                core::ptr::copy_nonoverlapping(p, q, old.min(n));
                self.dealloc(p, l);
            }
            q
        }
    }
}

// ---- Threads (`serve(…) workers(…)`) ----

/// What one thread at a time may do (`crate::Locked`): 0 free, 1 taken, 2 taken and a thread
/// sleeps waiting for it (Drepper, "Futexes are tricky").
pub struct Lock(AtomicU32);

impl Lock {
    pub const fn new() -> Lock {
        Lock(AtomicU32::new(0))
    }
    pub fn lock(&self) {
        if self.0.compare_exchange(0, 1, Acquire, Relaxed).is_err() {
            while self.0.swap(2, Acquire) != 0 {
                // FUTEX_WAIT_PRIVATE: sleeps while it is still 2.
                unsafe { call(FUTEX, self.0.as_ptr() as usize, 128, 2, 0, 0, 0) };
            }
        }
    }
    pub fn unlock(&self) {
        if self.0.swap(0, Release) == 2 {
            // FUTEX_WAKE_PRIVATE: one of them.
            unsafe { call(FUTEX, self.0.as_ptr() as usize, 129, 1, 0, 0, 0) };
        }
    }
}

/// Sleeps while `word` is `seen`.
pub fn wait(word: &AtomicU32, seen: u32) {
    // FUTEX_WAIT_PRIVATE.
    unsafe { call(FUTEX, word.as_ptr() as usize, 128, seen as usize, 0, 0, 0) };
}

/// `wait`, for `ns` nanoseconds at most.
pub fn wait_for(word: &AtomicU32, seen: u32, ns: u64) {
    let time = [ns / 1_000_000_000, ns % 1_000_000_000];
    // FUTEX_WAIT_PRIVATE: a time that counts from now.
    unsafe { call(FUTEX, word.as_ptr() as usize, 128, seen as usize, time.as_ptr() as usize, 0, 0) };
}

/// Wakes those sleeping on `word`.
pub fn wake(word: &AtomicU32) {
    // FUTEX_WAKE_PRIVATE.
    unsafe { call(FUTEX, word.as_ptr() as usize, 129, i32::MAX as usize, 0, 0, 0) };
}

/// The stack of each thread, as big as the main thread's usually is (`ulimit -s`).
const STACK: usize = 8 << 20;

/// A thread `spawn` started: made by `clone` (the word Linux clears when it ends, and its
/// stack), or by the C library.
pub enum Thread {
    Own(alloc::boxed::Box<AtomicU32>, usize),
    Libc(usize),
}

/// `f(arg)` on a new thread. None if the system cannot start it.
pub fn spawn(f: extern "C" fn(i64), arg: i64) -> Option<Thread> {
    let create = *PTHREAD_CREATE.get();
    if create != 0 {
        type Start = extern "C" fn(*mut u8) -> *mut u8;
        extern "C" fn begin(p: *mut u8) -> *mut u8 {
            let (f, arg) = *unsafe { alloc::boxed::Box::from_raw(p as *mut (extern "C" fn(i64), i64)) };
            f(arg);
            null_mut()
        }
        let create: extern "C" fn(*mut usize, *const u8, Start, *mut u8) -> i32 = unsafe { core::mem::transmute(create) };
        let mut thread = 0;
        let started = create(&mut thread, null(), begin, alloc::boxed::Box::into_raw(alloc::boxed::Box::new((f, arg))) as *mut u8) == 0;
        return started.then_some(Thread::Libc(thread));
    }
    // PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE | MAP_STACK.
    let base = match unsafe { call(MMAP, 0, STACK, 3, 0x22 | 0x4000 | 0x20000, usize::MAX, 0) } {
        p if p < 0 => return None,
        p => p as usize,
    };
    // Its lowest page cannot be reached: going past the stack stops the program instead of
    // writing over other memory.
    unsafe { call(MPROTECT, base, 4096, 0, 0, 0, 0) };
    // The new thread starts on its stack, where it finds `f` then `arg`.
    let top = (base + STACK - 16) as *mut usize;
    // Linux writes the thread's number there before `clone` returns, and 0 when it ends.
    let word = alloc::boxed::Box::new(AtomicU32::new(0));
    unsafe {
        top.write(f as usize);
        top.add(1).write(arg as usize);
        if start_thread(top as usize, word.as_ptr() as usize) < 0 {
            call(MUNMAP, base, STACK, 0, 0, 0, 0);
            return None;
        }
    }
    Some(Thread::Own(word, base))
}

/// Waits for the end of `t`.
pub fn join(t: Thread) {
    match t {
        Thread::Own(word, base) => {
            loop {
                let n = word.load(Acquire);
                if n == 0 {
                    break;
                }
                // FUTEX_WAIT, not private: Linux wakes the word it clears so.
                unsafe { call(FUTEX, word.as_ptr() as usize, 0, n as usize, 0, 0, 0) };
            }
            unsafe { call(MUNMAP, base, STACK, 0, 0, 0, 0) };
        }
        Thread::Libc(t) => {
            let join: extern "C" fn(usize, *mut usize) -> i32 = unsafe { core::mem::transmute(*PTHREAD_JOIN.get()) };
            join(t, null_mut());
        }
    }
}

/// CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
/// CLONE_PARENT_SETTID | CLONE_CHILD_CLEARTID: a thread sharing everything, whose number is
/// written in a word that Linux clears (and wakes) when it ends.
const THREAD: usize = 0x50f00 | 0x0010_0000 | 0x0020_0000;

/// A new thread on the stack `sp` (holding the function, then its argument), which calls the
/// function then stops: it never comes back here, whose stack it does not have. `word`: see
/// `THREAD`.
#[cfg(target_arch = "x86_64")]
unsafe fn start_thread(sp: usize, word: usize) -> isize {
    let r;
    unsafe {
        asm!(
            "syscall",
            "test rax, rax",
            "jnz 2f",
            "xor ebp, ebp",
            "pop rax",
            "pop rdi",
            "call rax",
            "mov eax, {exit}",
            "xor edi, edi",
            "syscall",
            "ud2",
            "2:",
            exit = const EXIT,
            // flags, stack, parent's word, child's word, TLS.
            inlateout("rax") CLONE as isize => r, in("rdi") THREAD, in("rsi") sp, in("rdx") word, in("r10") word, in("r8") 0usize,
            lateout("rcx") _, lateout("r11") _,
        )
    };
    r
}

#[cfg(target_arch = "aarch64")]
unsafe fn start_thread(sp: usize, word: usize) -> isize {
    let r;
    unsafe {
        asm!(
            "svc 0",
            "cbnz x0, 2f",
            "ldp x1, x0, [sp], #16",
            "mov x29, xzr",
            "mov x30, xzr",
            "blr x1",
            "mov x8, {exit}",
            "mov x0, xzr",
            "svc 0",
            "brk #0",
            "2:",
            exit = const EXIT,
            // flags, stack, parent's word, TLS, child's word: not x86-64's order.
            in("x8") CLONE, inlateout("x0") THREAD as isize => r, in("x1") sp, in("x2") word, in("x3") 0usize, in("x4") word,
        )
    };
    r
}

/// `extern` (PLAN Part 4 "`extern`, step 1"): a library loaded by the system's loader, or None.
pub fn library(name: &str) -> Option<usize> {
    let name = [name.as_bytes(), &[0]].concat();
    // RTLD_NOW
    let h = unsafe { dl::dlopen(name.as_ptr(), 2) };
    (!h.is_null()).then_some(h as usize)
}

/// A function of a library loaded by `library`, or None.
pub fn symbol(library: usize, name: &str) -> Option<usize> {
    let name = [name.as_bytes(), &[0]].concat();
    let f = unsafe { dl::dlsym(library as *mut u8, name.as_ptr()) };
    (!f.is_null()).then_some(f as usize)
}

mod dl {
    unsafe extern "C" {
        pub fn dlopen(name: *const u8, flags: i32) -> *mut u8;
        pub fn dlsym(library: *mut u8, name: *const u8) -> *mut u8;
    }
}

/// `ui.show` (src/ui.rs) with GTK 4, found at the first call like an `extern` library. The
/// window stays; its content is made again from `view` when the view's shape changed, else its
/// widgets only take the new texts, so a field being typed into keeps its cursor. The button
/// clicked, a field changed (`shared::ui_changed`, its text in `typed`), or -1 once the window is
/// closed (or when GTK is missing or there is no display).
pub fn ui_show(title: &str, view: &crate::shared::UiNode, menus: &[crate::shared::UiMenu], width: i32, height: i32, acts: &[i64], typed: &mut String) -> i64 {
    // The library (0 = not looked for yet, 1 = missing), the window, what happened (`WAITING`:
    // nothing yet, -1: the window closed), the text of the field that changed.
    static GTK: Global<usize> = Global::new(0);
    static WINDOW: Global<usize> = Global::new(0);
    static CLICKED: Global<i64> = Global::new(WAITING);
    static TYPED: Global<String> = Global::new(String::new());
    // The view's shape and its widgets (with each field's `changed` handler), to change them in place.
    static SHAPE: Global<String> = Global::new(String::new());
    static MADE: Global<Vec<(P, u64, usize)>> = Global::new(Vec::new());
    const WAITING: i64 = i64::MIN;
    extern "C" fn clicked(_button: P, number: P) {
        *CLICKED.get() = number as i64;
    }
    extern "C" fn changed(field: P, number: P) {
        let get: extern "C" fn(P) -> *const u8 = unsafe { core::mem::transmute(symbol(*GTK.get(), "gtk_editable_get_text").unwrap_or(0)) };
        *TYPED.get() = gtk_text(get(field));
        *CLICKED.get() = crate::shared::ui_changed(number as usize);
    }
    extern "C" fn closing(_window: P, _data: P) -> i32 {
        *CLICKED.get() = -1;
        0
    }
    fn valued(text: String, number: usize) {
        *TYPED.get() = text;
        *CLICKED.get() = crate::shared::ui_changed(number);
    }
    extern "C" fn toggled(b: P, number: P) {
        let get: extern "C" fn(P) -> i32 = unsafe { core::mem::transmute(symbol(*GTK.get(), "gtk_check_button_get_active").unwrap_or(0)) };
        valued((if get(b) != 0 { "on" } else { "off" }).into(), number as usize);
    }
    extern "C" fn slid(r: P, number: P) {
        let get: extern "C" fn(P) -> f64 = unsafe { core::mem::transmute(symbol(*GTK.get(), "gtk_range_get_value").unwrap_or(0)) };
        valued(format!("{}", get(r) as i64), number as usize);
    }
    extern "C" fn chosen(d: P, _spec: P, number: P) {
        let get: extern "C" fn(P) -> u32 = unsafe { core::mem::transmute(symbol(*GTK.get(), "gtk_drop_down_get_selected").unwrap_or(0)) };
        valued(format!("{}", get(d)), number as usize);
    }
    extern "C" fn menu_picked(_action: P, _parameter: P, number: P) {
        *CLICKED.get() = number as i64;
    }
    extern "C" fn row_chosen(_list: P, row: P, number: P) {
        if !row.is_null() {
            let get: extern "C" fn(P) -> i32 = unsafe { core::mem::transmute(symbol(*GTK.get(), "gtk_list_box_row_get_index").unwrap_or(0)) };
            valued(format!("{}", get(row)), number as usize);
        }
    }
    if *GTK.get() == 0 {
        *GTK.get() = library("libgtk-4.so.1").unwrap_or(1);
    }
    if *GTK.get() == 1 || *CLICKED.get() == -1 {
        return -1;
    }
    *CLICKED.get() = WAITING;
    GTK_LIBRARY.get().clone_from(GTK.get());
    let f = |name: &str| symbol(*GTK.get(), name).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let g = Gtk {
            library: *GTK.get(),
            label: t(f("gtk_label_new")),
            button: t(f("gtk_button_new_with_label")),
            entry: t(f("gtk_entry_new")),
            set_text: t(f("gtk_editable_set_text")),
            boxed: t(f("gtk_box_new")),
            append: t(f("gtk_box_append")),
            halign: t(f("gtk_widget_set_halign")),
            valign: t(f("gtk_widget_set_valign")),
            margins: [t(f("gtk_widget_set_margin_top")), t(f("gtk_widget_set_margin_bottom")), t(f("gtk_widget_set_margin_start")), t(f("gtk_widget_set_margin_end"))],
            connect: t(f("g_signal_connect_data")),
            clicked: clicked as usize,
            changed: changed as usize,
            values: [toggled as usize, slid as usize, chosen as usize, row_chosen as usize, menu_picked as usize],
        };
        if *WINDOW.get() == 0 {
            // The program's name, for the taskbar (on Wayland, the window's app id).
            let program = crate::ARGS.get().first().map_or("", |a| a.rsplit('/').next().unwrap_or(a));
            let set_name: extern "C" fn(*const u8) = t(f("g_set_prgname"));
            set_name(c_string(program).as_ptr());
            let init: extern "C" fn() -> i32 = t(f("gtk_init_check"));
            if init() == 0 {
                return -1;
            }
            let new: extern "C" fn() -> P = t(f("gtk_window_new"));
            let w = new();
            // The `header`'s icon, given to the window itself once it exists: on Wayland, GTK
            // sends it to the desktop (protocol `xdg_toplevel_icon_v1`, KDE and others).
            let icon = crate::icon();
            if !icon.is_empty() {
                let bytes: extern "C" fn(*const u8, usize) -> P = t(f("g_bytes_new"));
                let texture: extern "C" fn(P, *mut P) -> P = t(f("gdk_texture_new_from_bytes"));
                let image = texture(bytes(icon.as_ptr(), icon.len()), null_mut());
                if !image.is_null() {
                    let realize: extern "C" fn(P) = t(f("gtk_widget_realize"));
                    let surface: extern "C" fn(P) -> P = t(f("gtk_native_get_surface"));
                    let append: extern "C" fn(P, P) -> P = t(f("g_list_append"));
                    let set_icons: extern "C" fn(P, P) = t(f("gdk_toplevel_set_icon_list"));
                    realize(w);
                    set_icons(surface(w), append(null_mut(), image));
                }
            }
            // The size the program asked for, else at least 320 pixels wide, as on Windows and
            // macOS, so the title shows. GTK never makes it smaller than the content.
            let default_size: extern "C" fn(P, i32, i32) = t(f("gtk_window_set_default_size"));
            default_size(w, if width > 0 { width } else { 320 }, if height > 0 { height } else { -1 });
            if acts.contains(&0) {
                let decorated: extern "C" fn(P, i32) = t(f("gtk_window_set_decorated"));
                decorated(w, 0);
                *GTK_BARE.get() = true;
            }
            (g.connect)(w, c"close-request".as_ptr() as *const u8, closing as usize, null_mut(), null_mut(), 0);
            *WINDOW.get() = w as usize;
        }
        let w = *WINDOW.get() as P;
        *GTK_WINDOW.get() = w as usize;
        let set_title: extern "C" fn(P, *const u8) = t(f("gtk_window_set_title"));
        set_title(w, c_string(title).as_ptr());
        let shape = alloc::format!("{menus:?}{}", crate::shared::ui_shape(view));
        if shape == *SHAPE.get() {
            // The same widgets: only the texts that differ change.
            let label_text: extern "C" fn(P) -> *const u8 = t(f("gtk_label_get_text"));
            let set_label: extern "C" fn(P, *const u8) = t(f("gtk_label_set_text"));
            let button_text: extern "C" fn(P) -> *const u8 = t(f("gtk_button_get_label"));
            let set_button: extern "C" fn(P, *const u8) = t(f("gtk_button_set_label"));
            let field_text: extern "C" fn(P) -> *const u8 = t(f("gtk_editable_get_text"));
            let block: extern "C" fn(P, u64) = t(f("g_signal_handler_block"));
            let unblock: extern "C" fn(P, u64) = t(f("g_signal_handler_unblock"));
            for ((leaf, _), &(widget, handler, _)) in crate::shared::ui_leaves(view).into_iter().zip(MADE.get().iter()) {
                use crate::shared::UiNode::*;
                match leaf {
                    Text(s) if gtk_text(label_text(widget)) != *s => set_label(widget, c_string(s).as_ptr()),
                    Button(s, _) if gtk_text(button_text(widget)) != *s => set_button(widget, c_string(s).as_ptr()),
                    Canvas(c) => {
                        let draw_again: extern "C" fn(P) = t(f("gtk_widget_queue_draw"));
                        if let Some((_, there)) = GTK_CANVASES.get().iter_mut().find(|(area, _)| *area == widget) {
                            there.ops.clone_from(&c.ops);
                        }
                        draw_again(widget);
                    }
                    // Set from here, a value must not tell its handler the user changed it.
                    Check(_, on, _) => {
                        let (get, set): (extern "C" fn(P) -> i32, extern "C" fn(P, i32)) = (t(f("gtk_check_button_get_active")), t(f("gtk_check_button_set_active")));
                        if (get(widget) != 0) != *on {
                            block(widget, handler);
                            set(widget, *on as i32);
                            unblock(widget, handler);
                        }
                    }
                    Slider(v, ..) => {
                        let (get, set): (extern "C" fn(P) -> f64, extern "C" fn(P, f64)) = (t(f("gtk_range_get_value")), t(f("gtk_range_set_value")));
                        if get(widget) as i32 != *v {
                            block(widget, handler);
                            set(widget, *v as f64);
                            unblock(widget, handler);
                        }
                    }
                    Choice(_, k, _) => {
                        let (get, set): (extern "C" fn(P) -> u32, extern "C" fn(P, u32)) = (t(f("gtk_drop_down_get_selected")), t(f("gtk_drop_down_set_selected")));
                        if get(widget) != *k as u32 {
                            block(widget, handler);
                            set(widget, *k as u32);
                            unblock(widget, handler);
                        }
                    }
                    Table(table) => {
                        block(widget, handler);
                        gtk_select(&g, widget, table.selected);
                        unblock(widget, handler);
                    }
                    // Set from here, a field must not tell `changed` it was typed into.
                    Input(s, _) if gtk_text(field_text(widget)) != *s => {
                        block(widget, handler);
                        (g.set_text)(widget, c_string(s).as_ptr());
                        unblock(widget, handler);
                    }
                    _ => {}
                }
            }
        } else {
            let made = MADE.get();
            made.clear();
            GTK_CANVASES.get().clear();
            let root = gtk_make(&g, view, made, 1, 1);
            // 16 pixels around it; none in a window that has given up its frame.
            if !*GTK_BARE.get() {
                g.margins.iter().for_each(|margin| margin(root, 16));
            }
            let content = if menus.is_empty() {
                root
            } else {
                let both = (g.boxed)(1, 0);
                (g.append)(both, gtk_menu_bar(&g, w, menus));
                (g.append)(both, root);
                both
            };
            let set_child: extern "C" fn(P, P) = t(f("gtk_window_set_child"));
            set_child(w, content);
            *SHAPE.get() = shape;
            // The keys go to the canvas that hears them.
            let focus: extern "C" fn(P) -> i32 = t(f("gtk_widget_grab_focus"));
            if let Some((area, _)) = GTK_CANVASES.get().iter().find(|(area, _)| gtk_hears_keys(*area)) {
                focus(*area);
            }
        }
        // The clocks of the view that was there stop, this one's start. Below the redrawing
        // (G_PRIORITY_DEFAULT_IDLE): a clock faster than the program must not stop it drawing.
        let clocks = crate::shared::ui_clocks(view);
        if clocks != *CLOCKS_ASKED.get() {
            let remove: extern "C" fn(u32) -> i32 = t(f("g_source_remove"));
            let every: extern "C" fn(i32, u32, usize, P, P) -> u32 = t(f("g_timeout_add_full"));
            CLOCKS.get().drain(..).for_each(|clock| {
                remove(clock);
            });
            for &(ms, n) in &clocks {
                CLOCKS.get().push(every(200, ms as u32, gtk_tick as usize, n as P, null_mut()));
            }
            *CLOCKS_ASKED.get() = clocks;
        }
        for ((_, style), (widget, _, class)) in crate::shared::ui_leaves(view).into_iter().zip(MADE.get().iter_mut()) {
            gtk_style(&f, *widget, style, class);
        }
        let present: extern "C" fn(P) = t(f("gtk_window_present"));
        present(w);
        let iteration: extern "C" fn(P, i32) -> i32 = t(f("g_main_context_iteration"));
        for act in acts {
            match act {
                1 => {
                    let destroy: extern "C" fn(P) = t(f("gtk_window_destroy"));
                    destroy(w);
                    while iteration(null_mut(), 0) != 0 {}
                    *CLICKED.get() = -1;
                    return -1;
                }
                2 => {
                    let minimize: extern "C" fn(P) = t(f("gtk_window_minimize"));
                    minimize(w);
                }
                3 => gtk_drag(w),
                4 => {
                    let is_big: extern "C" fn(P) -> i32 = t(f("gtk_window_is_maximized"));
                    let change: extern "C" fn(P) = t(f(if is_big(w) != 0 { "gtk_window_unmaximize" } else { "gtk_window_maximize" }));
                    change(w);
                }
                5 => {
                    let is_full: extern "C" fn(P) -> i32 = t(f("gtk_window_is_fullscreen"));
                    let change: extern "C" fn(P) = t(f(if is_full(w) != 0 { "gtk_window_unfullscreen" } else { "gtk_window_fullscreen" }));
                    change(w);
                }
                // 6, `ui.place`: Wayland gives no program its place, and GTK 4 has no call that
                // resizes an open window (`gtk_window_set_default_size` does nothing then).
                _ => {}
            }
        }
        while *CLICKED.get() == WAITING && GTK_HEARD.get().is_empty() {
            iteration(null_mut(), 1);
        }
        if *CLICKED.get() == WAITING {
            let (answer, text) = GTK_HEARD.get().next().unwrap_or((-1, String::new()));
            *typed = text;
            return answer;
        }
        if *CLICKED.get() <= -2 {
            *typed = TYPED.get().clone();
        }
        *CLICKED.get()
    }
}

static GTK_LIBRARY: Global<usize> = Global::new(0);
/// The window, for `ui_areas`.
static GTK_WINDOW: Global<usize> = Global::new(0);

/// `ui.window` then `ui.screen`: x, y, width, height. A program never knows its place on Wayland,
/// nor what the panels take from the screen.
pub fn ui_areas() -> Option<[i32; 8]> {
    let w = *GTK_WINDOW.get() as P;
    if w.is_null() {
        return None;
    }
    let f = |name: &str| symbol(*GTK_LIBRARY.get(), name).unwrap_or(0);
    let mut screen = [0i32; 4];
    unsafe {
        use core::mem::transmute as t;
        let (width, height): (extern "C" fn(P) -> i32, extern "C" fn(P) -> i32) = (t(f("gtk_widget_get_width")), t(f("gtk_widget_get_height")));
        let surface_of: extern "C" fn(P) -> P = t(f("gtk_native_get_surface"));
        let surface = surface_of(w);
        if !surface.is_null() {
            let display: extern "C" fn(P) -> P = t(f("gdk_surface_get_display"));
            let monitor: extern "C" fn(P, P) -> P = t(f("gdk_display_get_monitor_at_surface"));
            let geometry: extern "C" fn(P, *mut i32) = t(f("gdk_monitor_get_geometry"));
            let at = monitor(display(surface), surface);
            if !at.is_null() {
                geometry(at, screen.as_mut_ptr());
            }
        }
        Some([0, 0, width(w), height(w), screen[0], screen[1], screen[2], screen[3]])
    }
}
/// `ui.run(…) frame(false)`.
static GTK_BARE: Global<bool> = Global::new(false);
/// What a canvas or a clock heard, for `ui_show`.
static GTK_HEARD: Global<crate::shared::UiQueue> = Global::new(crate::shared::UiQueue::new());
/// The running clocks (`g_timeout_add_full`).
static CLOCKS: Global<Vec<u32>> = Global::new(Vec::new());
/// What those clocks are (`shared::ui_clocks`).
static CLOCKS_ASKED: Global<Vec<(i32, usize)>> = Global::new(Vec::new());
/// Each canvas now on screen, with what it draws.
static GTK_CANVASES: Global<Vec<(P, crate::shared::UiCanvas)>> = Global::new(Vec::new());

fn gtk_symbol(name: &str) -> usize {
    symbol(*GTK_LIBRARY.get(), name).unwrap_or(0)
}

extern "C" fn gtk_tick(n: P) -> i32 {
    GTK_HEARD.get().hear(n as i64, String::new());
    1 // G_SOURCE_CONTINUE
}

/// The press `ui.drag` starts from: device, button, place in the window, time.
static GTK_PRESS: Global<(P, i32, f64, f64, u32)> = Global::new((null_mut(), 0, 0.0, 0.0, 0));

/// Hands the window over to the system, which moves it until the button is released.
fn gtk_drag(w: P) {
    let (device, button, x, y, time) = *GTK_PRESS.get();
    if device.is_null() {
        return;
    }
    unsafe {
        use core::mem::transmute as t;
        let surface: extern "C" fn(P) -> P = t(gtk_symbol("gtk_native_get_surface"));
        let begin: extern "C" fn(P, P, i32, f64, f64, u32) = t(gtk_symbol("gdk_toplevel_begin_move"));
        begin(surface(w), device, button, x, y, time);
    }
}

fn gtk_mouse(gesture: Option<P>, kind: &str, x: f64, y: f64, number: P) {
    if let (Some(g), "press") = (gesture, kind) {
        unsafe {
            use core::mem::transmute as t;
            let device: extern "C" fn(P) -> P = t(gtk_symbol("gtk_gesture_get_device"));
            let time: extern "C" fn(P) -> u32 = t(gtk_symbol("gtk_event_controller_get_current_event_time"));
            let button: extern "C" fn(P) -> u32 = t(gtk_symbol("gtk_gesture_single_get_current_button"));
            *GTK_PRESS.get() = (device(g), button(g) as i32, x, y, time(g));
        }
    }
    let button = gesture.map_or(0, |g| {
        let current: extern "C" fn(P) -> u32 = unsafe { core::mem::transmute(gtk_symbol("gtk_gesture_single_get_current_button")) };
        if current(g) == 3 { 2 } else { 1 }
    });
    GTK_HEARD.get().hear(crate::shared::ui_changed(number as usize), crate::shared::ui_mouse(kind, x as i32, y as i32, button));
}

extern "C" fn gtk_pressed(g: P, _times: i32, x: f64, y: f64, number: P) {
    gtk_mouse(Some(g), "press", x, y, number);
}

extern "C" fn gtk_released(g: P, _times: i32, x: f64, y: f64, number: P) {
    gtk_mouse(Some(g), "release", x, y, number);
}

extern "C" fn gtk_moved(_motion: P, x: f64, y: f64, number: P) {
    gtk_mouse(None, "move", x, y, number);
}

/// `ui.Key`'s name: what the key writes, or the same word on every system.
extern "C" fn gtk_key(_keys: P, key: u32, _code: u32, _state: u32, number: P) -> i32 {
    let (unicode, name): (extern "C" fn(u32) -> u32, extern "C" fn(u32) -> *const u8) = unsafe { (core::mem::transmute(gtk_symbol("gdk_keyval_to_unicode")), core::mem::transmute(gtk_symbol("gdk_keyval_name"))) };
    let word = gtk_text(name(key));
    let named = match word.as_str() {
        "space" => "Space",
        "Return" | "KP_Enter" => "Enter",
        "BackSpace" => "Backspace",
        "Page_Up" => "PageUp",
        "Page_Down" => "PageDown",
        "Tab" | "Escape" | "Delete" | "Left" | "Right" | "Up" | "Down" | "Home" | "End" => &word,
        _ if word.starts_with('F') && word[1..].parse::<u32>().is_ok() => &word,
        _ => "",
    };
    let text = match (named, char::from_u32(unicode(key))) {
        ("", Some(c)) if c != '\0' => c.into(),
        ("", _) => return 0, // a modifier alone
        (n, _) => String::from(n),
    };
    GTK_HEARD.get().hear(crate::shared::ui_changed(number as usize), crate::shared::ui_key(&text));
    1
}

fn gtk_hears_keys(area: P) -> bool {
    let focusable: extern "C" fn(P) -> i32 = unsafe { core::mem::transmute(gtk_symbol("gtk_widget_get_focusable")) };
    focusable(area) != 0
}

/// Paints a canvas, from what `ui.canvas` drew. Thickness stays inside a shape, as on every system.
extern "C" fn gtk_paint(area: P, cairo: P, width: i32, height: i32, _data: P) {
    use crate::shared::UiOp::*;
    let Some((_, canvas)) = GTK_CANVASES.get().iter().find(|(a, _)| *a == area) else { return };
    let ops = &canvas.ops;
    if (canvas.width == 0 || canvas.height == 0) && crate::ui_room(canvas.number, width, height) {
        GTK_HEARD.get().hear(crate::shared::UI_RESIZED, String::new());
    }
    unsafe {
        use core::mem::transmute as t;
        let f = gtk_symbol;
        let source: extern "C" fn(P, f64, f64, f64) = t(f("cairo_set_source_rgb"));
        let paint: extern "C" fn(P) = t(f("cairo_paint"));
        let rectangle: extern "C" fn(P, f64, f64, f64, f64) = t(f("cairo_rectangle"));
        let arc: extern "C" fn(P, f64, f64, f64, f64, f64) = t(f("cairo_arc"));
        let fill: extern "C" fn(P) = t(f("cairo_fill"));
        let stroke: extern "C" fn(P) = t(f("cairo_stroke"));
        let thickness: extern "C" fn(P, f64) = t(f("cairo_set_line_width"));
        let move_to: extern "C" fn(P, f64, f64) = t(f("cairo_move_to"));
        let line_to: extern "C" fn(P, f64, f64) = t(f("cairo_line_to"));
        let font: extern "C" fn(P, *const u8, i32, i32) = t(f("cairo_select_font_face"));
        let font_size: extern "C" fn(P, f64) = t(f("cairo_set_font_size"));
        let extents: extern "C" fn(P, *mut [f64; 5]) = t(f("cairo_font_extents"));
        let show: extern "C" fn(P, *const u8) = t(f("cairo_show_text"));
        let color = |c: u32| source(cairo, (c >> 16 & 255) as f64 / 255.0, (c >> 8 & 255) as f64 / 255.0, (c & 255) as f64 / 255.0);
        let outlined = |n: i32| {
            if n > 0 {
                thickness(cairo, n as f64);
                stroke(cairo);
            } else {
                fill(cairo);
            }
        };
        color(0xffffff);
        paint(cairo);
        font(cairo, c"sans-serif".as_ptr() as *const u8, 0, 0);
        for op in ops {
            match op {
                Gpu | GlClear(_) | GlShader(..) | GlUniform(..) | GlTriangles(..) | GlTexture(..) => {}
                Clear(c) => {
                    color(*c);
                    paint(cairo);
                }
                Rect { x, y, w, h, color: c, outline } => {
                    color(*c);
                    let inset = *outline as f64 / 2.0;
                    rectangle(cairo, *x as f64 + inset, *y as f64 + inset, *w as f64 - 2.0 * inset, *h as f64 - 2.0 * inset);
                    outlined(*outline);
                }
                Circle { x, y, r, color: c, outline } => {
                    color(*c);
                    arc(cairo, *x as f64, *y as f64, (*r as f64 - *outline as f64 / 2.0).max(0.0), 0.0, 6.283185307179586);
                    outlined(*outline);
                }
                Line { x, y, x2, y2, color: c, thickness: n } => {
                    color(*c);
                    thickness(cairo, *n as f64);
                    move_to(cairo, *x as f64, *y as f64);
                    line_to(cairo, *x2 as f64, *y2 as f64);
                    stroke(cairo);
                }
                Image { x, y, w, h, path } => {
                    if !GTK_IMAGES.get().iter().any(|i| i.0 == *path) {
                        let load: extern "C" fn(*const u8, *mut P) -> P = t(f("gdk_pixbuf_new_from_file"));
                        GTK_IMAGES.get().push((path.clone(), load(c_string(path).as_ptr(), null_mut())));
                    }
                    let pixbuf = GTK_IMAGES.get().iter().find(|i| i.0 == *path).map_or(null_mut(), |i| i.1);
                    if !pixbuf.is_null() {
                        let size: [extern "C" fn(P) -> i32; 2] = [t(f("gdk_pixbuf_get_width")), t(f("gdk_pixbuf_get_height"))];
                        let (own_w, own_h) = (size[0](pixbuf) as f64, size[1](pixbuf) as f64);
                        let save: extern "C" fn(P) = t(f("cairo_save"));
                        let restore: extern "C" fn(P) = t(f("cairo_restore"));
                        let translate: extern "C" fn(P, f64, f64) = t(f("cairo_translate"));
                        let scale: extern "C" fn(P, f64, f64) = t(f("cairo_scale"));
                        let source: extern "C" fn(P, P, f64, f64) = t(f("gdk_cairo_set_source_pixbuf"));
                        save(cairo);
                        translate(cairo, *x as f64, *y as f64);
                        scale(cairo, if *w > 0 { *w as f64 / own_w } else { 1.0 }, if *h > 0 { *h as f64 / own_h } else { 1.0 });
                        source(cairo, pixbuf, 0.0, 0.0);
                        paint(cairo);
                        restore(cairo);
                    }
                }
                Text { x, y, size, color: c, text } => {
                    color(*c);
                    font_size(cairo, *size as f64);
                    let mut measures = [0.0; 5];
                    extents(cairo, &mut measures);
                    move_to(cairo, *x as f64, *y as f64 + measures[0]);
                    show(cairo, c_string(text).as_ptr());
                }
            }
        }
    }
}

/// Paints a `ui.gpu`. GTK has made its context current; libepoxy, which GTK itself uses, holds
/// the functions of that context as variables `epoxy_glClear`…
extern "C" fn gtk_gpu_paint(area: P, _context: P, _data: P) -> i32 {
    let Some((_, canvas)) = GTK_CANVASES.get().iter().find(|(a, _)| *a == area) else { return 1 };
    let Some(epoxy) = library("libepoxy.so.0") else { crate::gl_stop(crate::GPU_MISSING, b"") };
    unsafe {
        use core::mem::transmute as t;
        let size: [extern "C" fn(P) -> i32; 3] = [t(gtk_symbol("gtk_widget_get_width")), t(gtk_symbol("gtk_widget_get_height")), t(gtk_symbol("gtk_widget_get_scale_factor"))];
        let (width, height, scale) = (size[0](area), size[1](area), size[2](area));
        if (canvas.width == 0 || canvas.height == 0) && crate::ui_room(canvas.number, width, height) {
            GTK_HEARD.get().hear(crate::shared::UI_RESIZED, String::new());
        }
        crate::gl_paint(area as usize, &canvas.ops, width * scale, height * scale, &|name| symbol(epoxy, &format!("epoxy_{name}")).map_or(0, |at| *(at as *const usize)));
    }
    1
}

/// GTK could not make a context: this computer has no OpenGL to speak of.
extern "C" fn gtk_gpu_ready(area: P, _data: P) {
    let error: extern "C" fn(P) -> P = unsafe { core::mem::transmute(gtk_symbol("gtk_gl_area_get_error")) };
    if !error(area).is_null() {
        crate::gl_stop(crate::GPU_MISSING, b"");
    }
}

/// A `ui.gpu`'s area.
fn gtk_gpu(g: &Gtk, c: &crate::shared::UiCanvas) -> P {
    unsafe {
        use core::mem::transmute as t;
        let f = gtk_symbol;
        let new: extern "C" fn() -> P = t(f("gtk_gl_area_new"));
        let area = new();
        let version: extern "C" fn(P, i32, i32) = t(f("gtk_gl_area_set_required_version"));
        version(area, 3, 3);
        let depth: extern "C" fn(P, i32) = t(f("gtk_gl_area_set_has_depth_buffer"));
        depth(area, 1);
        // GTK 4.12 and later may otherwise choose OpenGL ES, which refuses `#version 330 core`.
        if let Some(apis) = symbol(*GTK_LIBRARY.get(), "gtk_gl_area_set_allowed_apis") {
            let apis: extern "C" fn(P, u32) = t(apis);
            apis(area, 1); // GDK_GL_API_GL
        }
        let least: extern "C" fn(P, i32, i32) = t(f("gtk_widget_set_size_request"));
        least(area, if c.width == 0 { -1 } else { c.width }, if c.height == 0 { -1 } else { c.height });
        (g.connect)(area, c"render".as_ptr() as *const u8, gtk_gpu_paint as usize, null_mut(), null_mut(), 0);
        (g.connect)(area, c"realize".as_ptr() as *const u8, gtk_gpu_ready as usize, null_mut(), null_mut(), 1); // G_CONNECT_AFTER
        area
    }
}

/// A canvas and what hears it.
fn gtk_canvas(g: &Gtk, c: &crate::shared::UiCanvas) -> P {
    unsafe {
        use core::mem::transmute as t;
        let f = gtk_symbol;
        let area = if c.gpu() {
            gtk_gpu(g, c)
        } else {
            let new: extern "C" fn() -> P = t(f("gtk_drawing_area_new"));
            let area = new();
            let content: [extern "C" fn(P, i32); 2] = [t(f("gtk_drawing_area_set_content_width")), t(f("gtk_drawing_area_set_content_height"))];
            content[0](area, c.width);
            content[1](area, c.height);
            let draw: extern "C" fn(P, usize, P, P) = t(f("gtk_drawing_area_set_draw_func"));
            draw(area, gtk_paint as usize, null_mut(), null_mut());
            area
        };
        GTK_CANVASES.get().push((area, c.clone()));
        let expand: [extern "C" fn(P, i32); 2] = [t(f("gtk_widget_set_hexpand")), t(f("gtk_widget_set_vexpand"))];
        expand[0](area, (c.width == 0) as i32);
        expand[1](area, (c.height == 0) as i32);
        let add: extern "C" fn(P, P) = t(f("gtk_widget_add_controller"));
        if let Some(n) = c.mouse {
            let clicks: extern "C" fn() -> P = t(f("gtk_gesture_click_new"));
            let any_button: extern "C" fn(P, u32) = t(f("gtk_gesture_single_set_button"));
            let (click, number) = (clicks(), n as P);
            any_button(click, 0);
            (g.connect)(click, c"pressed".as_ptr() as *const u8, gtk_pressed as usize, number, null_mut(), 0);
            (g.connect)(click, c"released".as_ptr() as *const u8, gtk_released as usize, number, null_mut(), 0);
            add(area, click);
            let moves: extern "C" fn() -> P = t(f("gtk_event_controller_motion_new"));
            let motion = moves();
            (g.connect)(motion, c"motion".as_ptr() as *const u8, gtk_moved as usize, number, null_mut(), 0);
            add(area, motion);
        }
        if let Some(n) = c.key {
            let keys: extern "C" fn() -> P = t(f("gtk_event_controller_key_new"));
            let focusable: extern "C" fn(P, i32) = t(f("gtk_widget_set_focusable"));
            let controller = keys();
            (g.connect)(controller, c"key-pressed".as_ptr() as *const u8, gtk_key as usize, n as P, null_mut(), 0);
            add(area, controller);
            focusable(area, 1);
        }
        area
    }
}

/// Gives a widget its style: a CSS class per style met (`kova-1`…), its number in `class`, 0 for none.
fn gtk_style(f: &dyn Fn(&str) -> usize, widget: P, style: crate::shared::UiStyle, class: &mut usize) {
    use crate::shared::UiStyle;
    static STYLES: Global<Vec<UiStyle>> = Global::new(Vec::new());
    let styles = STYLES.get();
    let wanted = match styles.iter().position(|s| *s == style) {
        _ if style == UiStyle::default() => 0,
        Some(k) => k + 1,
        None => unsafe {
            use core::mem::transmute as t;
            styles.push(style);
            let mut css = format!(".kova-{} {{", styles.len());
            if let Some(c) = style.color {
                css.push_str(&format!(" color: #{c:06x};"));
            }
            if style.size > 0 {
                css.push_str(&format!(" font-size: {}px;", style.size));
            }
            if style.bold {
                css.push_str(" font-weight: bold;");
            }
            css.push_str(" }");
            let new: extern "C" fn() -> P = t(f("gtk_css_provider_new"));
            // Not `load_from_string`: it needs GTK 4.12.
            let load: extern "C" fn(P, *const u8, isize) = t(f("gtk_css_provider_load_from_data"));
            let display: extern "C" fn() -> P = t(f("gdk_display_get_default"));
            let add: extern "C" fn(P, P, u32) = t(f("gtk_style_context_add_provider_for_display"));
            let provider = new();
            load(provider, css.as_ptr(), css.len() as isize);
            add(display(), provider, 600); // GTK_STYLE_PROVIDER_PRIORITY_APPLICATION
            styles.len()
        },
    };
    if wanted != *class {
        unsafe {
            use core::mem::transmute as t;
            let change: [extern "C" fn(P, *const u8); 2] = [t(f("gtk_widget_remove_css_class")), t(f("gtk_widget_add_css_class"))];
            if *class > 0 {
                change[0](widget, c_string(&format!("kova-{class}")).as_ptr());
            }
            if wanted > 0 {
                change[1](widget, c_string(&format!("kova-{wanted}")).as_ptr());
            }
        }
        *class = wanted;
    }
}

/// A text C gives (ended by a 0 byte), copied.
fn gtk_text(p: *const u8) -> String {
    if p.is_null() { String::new() } else { String::from_utf8_lossy(unsafe { c_text(p) }).into_owned() }
}

type P = *mut u8;

/// The GTK functions `gtk_make` calls.
struct Gtk {
    library: usize,
    label: extern "C" fn(*const u8) -> P,
    button: extern "C" fn(*const u8) -> P,
    entry: extern "C" fn() -> P,
    set_text: extern "C" fn(P, *const u8),
    boxed: extern "C" fn(i32, i32) -> P,
    halign: extern "C" fn(P, i32),
    valign: extern "C" fn(P, i32),
    margins: [extern "C" fn(P, i32); 4],
    append: extern "C" fn(P, P),
    connect: extern "C" fn(P, *const u8, usize, P, P, i32) -> u64,
    clicked: usize,
    changed: usize,
    /// What hears a check box, a slider, a drop-down list and a list.
    values: [usize; 5],
}

impl Gtk {
    fn get(&self, name: &str) -> usize {
        symbol(self.library, name).unwrap_or(0)
    }
}

/// The window's menus: a model of `GMenu`s, whose items start actions of an action group.
fn gtk_menu_bar(g: &Gtk, window: P, menus: &[crate::shared::UiMenu]) -> P {
    use core::mem::transmute as t;
    unsafe {
        let new_menu: extern "C" fn() -> P = t(g.get("g_menu_new"));
        let append: extern "C" fn(P, *const u8, *const u8) = t(g.get("g_menu_append"));
        let section: extern "C" fn(P, *const u8, P) = t(g.get("g_menu_append_section"));
        let submenu: extern "C" fn(P, *const u8, P) = t(g.get("g_menu_append_submenu"));
        let new_group: extern "C" fn() -> P = t(g.get("g_simple_action_group_new"));
        let new_action: extern "C" fn(*const u8, P) -> P = t(g.get("g_simple_action_new"));
        let add_action: extern "C" fn(P, P) = t(g.get("g_action_map_add_action"));
        let insert: extern "C" fn(P, *const u8, P) = t(g.get("gtk_widget_insert_action_group"));
        let bar_of: extern "C" fn(P) -> P = t(g.get("gtk_popover_menu_bar_new_from_model"));
        let (bar, group) = (new_menu(), new_group());
        for m in menus {
            let (sub, mut part) = (new_menu(), new_menu());
            for (label, event) in &m.items {
                match event {
                    Some(n) => {
                        let action = new_action(c_string(&format!("i{n}")).as_ptr(), null_mut());
                        (g.connect)(action, c"activate".as_ptr() as *const u8, g.values[4], *n as P, null_mut(), 0);
                        add_action(group, action);
                        append(part, c_string(label).as_ptr(), c_string(&format!("kova.i{n}")).as_ptr());
                    }
                    None => {
                        section(sub, null(), part);
                        part = new_menu();
                    }
                }
            }
            section(sub, null(), part);
            submenu(bar, c_string(&m.title).as_ptr(), sub);
        }
        insert(window, c"kova".as_ptr() as *const u8, group);
        bar_of(bar)
    }
}

/// `ui.dialog`. GTK's dialogs answer later: its loop runs until they do. None if the person gave
/// up, and on a GTK older than 4.10.
pub fn ui_dialog(kind: i64, title: &str, text: &str) -> Option<String> {
    use core::mem::transmute as t;
    static REPLY: Global<Option<Option<String>>> = Global::new(None);
    extern "C" fn filed(dialog: P, result: P, finish: P) {
        let finish: extern "C" fn(P, P, *mut P) -> P = unsafe { t(finish) };
        let file = finish(dialog, result, null_mut());
        *REPLY.get() = Some(if file.is_null() {
            None
        } else {
            let path: extern "C" fn(P) -> *const u8 = unsafe { t(gtk_symbol("g_file_get_path")) };
            Some(gtk_text(path(file)))
        });
    }
    extern "C" fn answered(dialog: P, result: P, _: P) {
        let finish: extern "C" fn(P, P, *mut P) -> i32 = unsafe { t(gtk_symbol("gtk_alert_dialog_choose_finish")) };
        *REPLY.get() = Some((finish(dialog, result, null_mut()) == 0).then(|| "yes".into()));
    }
    if *GTK_LIBRARY.get() < 2 {
        return None;
    }
    let f = gtk_symbol;
    let window = *GTK_WINDOW.get() as P;
    *REPLY.get() = None;
    unsafe {
        if kind >= 3 {
            let new = f("gtk_alert_dialog_new");
            if new == 0 {
                return None;
            }
            let new: extern "C" fn(*const u8) -> P = t(new);
            let detail: extern "C" fn(P, *const u8) = t(f("gtk_alert_dialog_set_detail"));
            let buttons: extern "C" fn(P, *const *const u8) = t(f("gtk_alert_dialog_set_buttons"));
            let cancel: extern "C" fn(P, i32) = t(f("gtk_alert_dialog_set_cancel_button"));
            let choose: extern "C" fn(P, P, P, usize, P) = t(f("gtk_alert_dialog_choose"));
            let d = new(c_string(&title.replace('%', "%%")).as_ptr());
            detail(d, c_string(text).as_ptr());
            let names: [*const u8; 3] = if kind == 3 { [c"OK".as_ptr().cast(), null(), null()] } else { [c"Yes".as_ptr().cast(), c"No".as_ptr().cast(), null()] };
            buttons(d, names.as_ptr());
            cancel(d, if kind == 3 { 0 } else { 1 });
            choose(d, window, null_mut(), answered as usize, null_mut());
        } else {
            let new = f("gtk_file_dialog_new");
            if new == 0 {
                return None;
            }
            let new: extern "C" fn() -> P = t(new);
            let set_title: extern "C" fn(P, *const u8) = t(f("gtk_file_dialog_set_title"));
            let d = new();
            set_title(d, c_string(title).as_ptr());
            if kind == 1 {
                let name: extern "C" fn(P, *const u8) = t(f("gtk_file_dialog_set_initial_name"));
                name(d, c_string(text).as_ptr());
            }
            let (ask, finish) = match kind {
                0 => ("gtk_file_dialog_open", "gtk_file_dialog_open_finish"),
                1 => ("gtk_file_dialog_save", "gtk_file_dialog_save_finish"),
                _ => ("gtk_file_dialog_select_folder", "gtk_file_dialog_select_folder_finish"),
            };
            let ask: extern "C" fn(P, P, P, usize, usize) = t(f(ask));
            ask(d, window, null_mut(), filed as usize, f(finish));
        }
        let iteration: extern "C" fn(P, i32) -> i32 = t(f("g_main_context_iteration"));
        while REPLY.get().is_none() {
            iteration(null_mut(), 1);
        }
    }
    REPLY.get().take().flatten()
}

/// `sound.play`: PulseAudio's simple library, which PipeWire answers too.
pub fn sound(_whole: &[u8], wav: &crate::shared::Wav) -> bool {
    use core::mem::transmute as t;
    let Some(pulse) = library("libpulse-simple.so.0") else { return false };
    let f = |name: &str| symbol(pulse, name).unwrap_or(0);
    // `pa_sample_format_t`: U8, S16LE, S24LE, S32LE.
    let format: u32 = match wav.bits {
        8 => 0,
        16 => 3,
        24 => 9,
        _ => 7,
    };
    let mut spec = [0u8; 12];
    spec[..4].copy_from_slice(&format.to_le_bytes());
    spec[4..8].copy_from_slice(&wav.rate.to_le_bytes());
    spec[8] = wav.channels as u8;
    unsafe {
        let new: extern "C" fn(*const u8, *const u8, i32, *const u8, *const u8, *const u8, *const u8, *const u8, *mut i32) -> P = t(f("pa_simple_new"));
        let write: extern "C" fn(P, *const u8, usize, *mut i32) -> i32 = t(f("pa_simple_write"));
        let drain: extern "C" fn(P, *mut i32) -> i32 = t(f("pa_simple_drain"));
        let free: extern "C" fn(P) = t(f("pa_simple_free"));
        let mut failure = 0;
        let name = c_string(crate::ARGS.get().first().map_or("kova", |a| a.rsplit('/').next().unwrap_or(a)));
        // PA_STREAM_PLAYBACK = 1.
        let stream = new(null(), name.as_ptr(), 1, null(), c"sound".as_ptr().cast(), spec.as_ptr(), null(), null(), &mut failure);
        if stream.is_null() {
            return false;
        }
        let played = write(stream, wav.samples.as_ptr(), wav.samples.len(), &mut failure) == 0 && drain(stream, &mut failure) == 0;
        free(stream);
        played
    }
}

/// The images a canvas drew: read once.
static GTK_IMAGES: Global<Vec<(String, P)>> = Global::new(Vec::new());

/// The line `k` of a `GtkListBox` selected, the others not.
fn gtk_select(g: &Gtk, list: P, k: i32) {
    use core::mem::transmute as t;
    unsafe {
        let line: extern "C" fn(P, i32) -> P = t(g.get("gtk_list_box_get_row_at_index"));
        let select: extern "C" fn(P, P) = t(g.get("gtk_list_box_select_row"));
        select(list, if k < 0 { null_mut() } else { line(list, k) });
    }
}

/// `ui.list`, `ui.table`: the headers, then the lines in a scrolled area; a size group per column
/// gives the same width to its cells. The `GtkListBox` that holds the lines, and what holds both.
fn gtk_table(g: &Gtk, table: &crate::shared::UiTable, handler: &mut u64) -> (P, P) {
    use core::mem::transmute as t;
    unsafe {
        let new_group: extern "C" fn(i32) -> P = t(g.get("gtk_size_group_new"));
        let add: extern "C" fn(P, P) = t(g.get("gtk_size_group_add_widget"));
        let align: extern "C" fn(P, f32) = t(g.get("gtk_label_set_xalign"));
        let columns = table.headers.len().max(table.rows.first().map_or(1, |r| r.len()));
        let groups: Vec<P> = (0..columns).map(|_| new_group(1)).collect();
        let line = |cells: &[String]| {
            let row = (g.boxed)(0, 16);
            (g.margins[2])(row, 6);
            (g.margins[3])(row, 6);
            for (cell, &group) in cells.iter().zip(&groups) {
                let label = (g.label)(c_string(cell).as_ptr());
                align(label, 0.0);
                add(group, label);
                (g.append)(row, label);
            }
            row
        };
        let outer = (g.boxed)(1, 4);
        if !table.headers.is_empty() {
            (g.append)(outer, line(&table.headers));
        }
        let new_list: extern "C" fn() -> P = t(g.get("gtk_list_box_new"));
        let list = new_list();
        let mode: extern "C" fn(P, i32) = t(g.get("gtk_list_box_set_selection_mode"));
        mode(list, 1); // GTK_SELECTION_SINGLE
        let append: extern "C" fn(P, P) = t(g.get("gtk_list_box_append"));
        table.rows.iter().for_each(|cells| append(list, line(cells)));
        gtk_select(g, list, table.selected);
        *handler = (g.connect)(list, c"row-selected".as_ptr() as *const u8, g.values[3], table.number as P, null_mut(), 0);
        let new_scrolled: extern "C" fn() -> P = t(g.get("gtk_scrolled_window_new"));
        let scrolled = new_scrolled();
        let tall: extern "C" fn(P, i32) = t(g.get("gtk_scrolled_window_set_min_content_height"));
        let wide: extern "C" fn(P, i32) = t(g.get("gtk_scrolled_window_set_min_content_width"));
        let natural: extern "C" fn(P, i32) = t(g.get("gtk_scrolled_window_set_propagate_natural_width"));
        let child: extern "C" fn(P, P) = t(g.get("gtk_scrolled_window_set_child"));
        tall(scrolled, 200);
        wide(scrolled, 300);
        natural(scrolled, 1);
        child(scrolled, list);
        (g.append)(outer, scrolled);
        (list, outer)
    }
}

/// The widgets of a view: a button gives its number to `clicked`, a field to `changed`. Each
/// text, button and field goes into `made`, in order, with its `changed` handler (0 if none).
/// `h` and `v`: where its parent places it (`GtkAlign`).
fn gtk_make(g: &Gtk, node: &crate::shared::UiNode, made: &mut Vec<(P, u64, usize)>, h: i32, v: i32) -> P {
    use crate::shared::UiNode::*;
    let mut handler = 0;
    let mut inner = None;
    let widget = match node {
        Text(s) => (g.label)(c_string(s).as_ptr()),
        Check(s, on, n) => unsafe {
            use core::mem::transmute as t;
            let (new, set): (extern "C" fn(*const u8) -> P, extern "C" fn(P, i32)) = (t(g.get("gtk_check_button_new_with_label")), t(g.get("gtk_check_button_set_active")));
            let b = new(c_string(s).as_ptr());
            set(b, *on as i32);
            handler = (g.connect)(b, c"toggled".as_ptr() as *const u8, g.values[0], *n as P, null_mut(), 0);
            b
        },
        Slider(v, min, max, n) => unsafe {
            use core::mem::transmute as t;
            let new: extern "C" fn(i32, f64, f64, f64) -> P = t(g.get("gtk_scale_new_with_range"));
            let set: extern "C" fn(P, f64) = t(g.get("gtk_range_set_value"));
            let size: extern "C" fn(P, i32, i32) = t(g.get("gtk_widget_set_size_request"));
            let s = new(0, *min as f64, *max as f64, 1.0);
            set(s, *v as f64);
            size(s, 200, -1);
            handler = (g.connect)(s, c"value-changed".as_ptr() as *const u8, g.values[1], *n as P, null_mut(), 0);
            s
        },
        Choice(items, k, n) => unsafe {
            use core::mem::transmute as t;
            let texts: Vec<Vec<u8>> = items.iter().map(|i| c_string(i)).collect();
            let mut list: Vec<*const u8> = texts.iter().map(|i| i.as_ptr()).collect();
            list.push(null());
            let new: extern "C" fn(*const *const u8) -> P = t(g.get("gtk_drop_down_new_from_strings"));
            let set: extern "C" fn(P, u32) = t(g.get("gtk_drop_down_set_selected"));
            let d = new(list.as_ptr());
            set(d, *k as u32);
            handler = (g.connect)(d, c"notify::selected".as_ptr() as *const u8, g.values[2], *n as P, null_mut(), 0);
            d
        },
        Table(table) => {
            let (list, outer) = gtk_table(g, table, &mut handler);
            inner = Some(list);
            outer
        }
        Button(s, n) => {
            let b = (g.button)(c_string(s).as_ptr());
            (g.connect)(b, c"clicked".as_ptr() as *const u8, g.clicked, *n as P, null_mut(), 0);
            b
        }
        // Its text first: `changed` only hears the user.
        Input(s, n) => {
            let e = (g.entry)();
            (g.set_text)(e, c_string(s).as_ptr());
            handler = (g.connect)(e, c"changed".as_ptr() as *const u8, g.changed, *n as P, null_mut(), 0);
            e
        }
        // GTK_ORIENTATION_HORIZONTAL = 0, VERTICAL = 1; 8 pixels between the parts.
        Column(parts, a) | Row(parts, a) => {
            let column = matches!(node, Column(..));
            let b = (g.boxed)(column as i32, 8);
            // GTK_ALIGN_START = 1, END = 2, CENTER = 3.
            let place = match a {
                crate::shared::UiAlign::Start => 1,
                crate::shared::UiAlign::End => 2,
                crate::shared::UiAlign::Center => 3,
            };
            for p in parts {
                let (h, v) = if column { (place, 1) } else { (1, place) };
                (g.append)(b, gtk_make(g, p, made, h, v));
            }
            b
        }
        // Through a pixbuf, the one way to have a smaller image: a `GtkPicture` asks for the file's size.
        Image(path, width) => unsafe {
            use core::mem::transmute as t;
            let f = |name: &str| symbol(g.library, name).unwrap_or(0);
            let file = c_string(path);
            let pixbuf = if *width > 0 {
                let load: extern "C" fn(*const u8, i32, i32, i32, *mut P) -> P = t(f("gdk_pixbuf_new_from_file_at_scale"));
                load(file.as_ptr(), *width, -1, 1, null_mut())
            } else {
                let load: extern "C" fn(*const u8, *mut P) -> P = t(f("gdk_pixbuf_new_from_file"));
                load(file.as_ptr(), null_mut())
            };
            if pixbuf.is_null() {
                (g.label)(c"".as_ptr() as *const u8)
            } else {
                let texture: extern "C" fn(P) -> P = t(f("gdk_texture_new_for_pixbuf"));
                let picture: extern "C" fn(P) -> P = t(f("gtk_picture_new_for_paintable"));
                let shrink: extern "C" fn(P, i32) = t(f("gtk_picture_set_can_shrink"));
                let p = picture(texture(pixbuf));
                shrink(p, 0);
                p
            }
        },
        Canvas(c) => gtk_canvas(g, c),
        Styled(_, inner) | Every(_, _, inner) => return gtk_make(g, inner, made, h, v),
        Spaced(m, inner) => {
            let widget = gtk_make(g, inner, made, h, v);
            g.margins.iter().for_each(|margin| margin(widget, *m));
            return widget;
        }
    };
    if !matches!(node, Column(..) | Row(..)) {
        made.push((inner.unwrap_or(widget), handler, 0));
    }
    // By default on the left and at the top, as Win32 and AppKit place it: GTK would centre a
    // text alone on its line. GTK_ALIGN_FILL = 0.
    (g.halign)(widget, if crate::shared::ui_fills(node) { 0 } else { h });
    (g.valign)(widget, if crate::shared::ui_fills_down(node) { 0 } else { v });
    widget
}

/// A text for C, ended by a 0 byte.
fn c_string(t: &str) -> Vec<u8> {
    [t.as_bytes(), &[0]].concat()
}
