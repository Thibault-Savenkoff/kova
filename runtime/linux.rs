// What the runtime asks of Linux: system calls made directly (no C library), its own entry point
// `_start`, and its own memory allocator. The executable is then static: it runs on any Linux.
// ponytail: x86-64 only; arm64 Linux keeps the C runtime (`runtime/runtime.c`) until it gets
// its system call numbers here and a machine to test them.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::{asm, global_asm};

use crate::{Global, c_text};

pub const SLASH: char = '/';

unsafe fn call(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let r;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b, in("rdx") c, in("r10") d, in("r8") e, in("r9") f, lateout("rcx") _, lateout("r11") _, options(nostack))
    };
    r
}

const READ: usize = 0;
const WRITE: usize = 1;
const CLOSE: usize = 3;
const MMAP: usize = 9;
const MUNMAP: usize = 11;
const IOCTL: usize = 16;
const MREMAP: usize = 25;
const NANOSLEEP: usize = 35;
const GETPID: usize = 39;
const GETDENTS64: usize = 217;
const CLOCK_GETTIME: usize = 228;
const EXIT_GROUP: usize = 231;
const OPENAT: usize = 257;
const MKDIRAT: usize = 258;
const UNLINKAT: usize = 263;
const STATX: usize = 332;

const AT_FDCWD: usize = -100isize as usize;
const EINTR: isize = 4;
const EEXIST: isize = 17;

// The program starts here: the system put the number of words, then the words (`argv`), then
// the environment (`envp`) on the stack.
global_asm!(".globl _start", "_start:", "xor ebp, ebp", "mov rdi, rsp", "and rsp, -16", "call {start}", "ud2", start = sym start);

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
    let fd = open(path, 0o200000)?; // O_DIRECTORY
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
