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
    pub const LISTEN: usize = 50;
    pub const ACCEPT4: usize = 288;
    pub const PPOLL: usize = 271;
    pub const MPROTECT: usize = 10;
    pub const CLONE: usize = 56;
    pub const EXIT: usize = 60;
    pub const FUTEX: usize = 202;
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
    pub const LISTEN: usize = 201;
    pub const ACCEPT4: usize = 242;
    pub const PPOLL: usize = 73;
    pub const MPROTECT: usize = 226;
    pub const CLONE: usize = 220;
    pub const EXIT: usize = 93;
    pub const FUTEX: usize = 98;
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

    fn close(&mut self, fd: usize) {
        close(fd);
    }
}

impl Serve for Net {
    type Listener = usize;

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
