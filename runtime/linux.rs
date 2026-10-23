// What the runtime asks of Linux: system calls made directly (no C library), its own entry point
// `_start`, and its own memory allocator. The executable is then static: it runs on any Linux.
// On x86-64 and on arm64: only the system calls' numbers, how they are made and `_start` differ.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};
use core::ptr::{null, null_mut};

use crate::http::{Fail, Network};

use crate::{Global, c_text};

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
    pub const SOCKET: usize = 41;
    pub const CONNECT: usize = 42;
    pub const SENDTO: usize = 44;
    pub const RECVFROM: usize = 45;
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
    pub const SOCKET: usize = 198;
    pub const CONNECT: usize = 203;
    pub const SENDTO: usize = 206;
    pub const RECVFROM: usize = 207;
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
    unsafe { start(argv.sub(1)) }
}

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
    fn open(&mut self, host: &str, port: u16, _: bool) -> Result<usize, Fail> {
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
                match unsafe { call(CONNECT, fd as usize, i.addr as usize, i.addrlen as usize, 0, 0, 0) } {
                    0 => {
                        failure = Ok(fd as usize);
                        break;
                    }
                    e => failure = Err(if e == -ETIMEDOUT { Fail::Timeout } else { Fail::Refused }),
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

static BLOCKS: Global<Blocks> = Global::new(Blocks { freed: [core::ptr::null_mut(); SIZES], next: 0, end: 0 });

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
        let b = BLOCKS.get();
        let k = size_of(l.size());
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
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if l.size() > LARGEST {
            unsafe { call(MUNMAP, p as usize, pages(l.size()), 0, 0, 0, 0) };
            return;
        }
        let b = BLOCKS.get();
        let k = size_of(l.size());
        unsafe { *(p as *mut *mut u8) = b.freed[k] };
        b.freed[k] = p;
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
