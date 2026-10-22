// What the runtime asks of macOS: `libSystem`, which every Mac has (Apple wants programs to go
// through it, not to call the system directly). Its `main` is the program's entry point.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};

use crate::c_text;

pub const SLASH: char = '/';

mod c {
    unsafe extern "C" {
        pub fn write(fd: i32, b: *const u8, n: usize) -> isize;
        pub fn read(fd: i32, b: *mut u8, n: usize) -> isize;
        pub fn open(path: *const u8, flags: i32, ...) -> i32;
        pub fn close(fd: i32) -> i32;
        pub fn exit(code: i32) -> !;
        pub fn getenv(name: *const u8) -> *const u8;
        pub fn isatty(fd: i32) -> i32;
        pub fn clock_gettime(clock: u32, t: *mut [i64; 2]) -> i32;
        pub fn nanosleep(t: *const [i64; 2], rest: *mut [i64; 2]) -> i32;
        pub fn getpid() -> i32;
        // Intel Macs name the versions with 64-bit file numbers so.
        #[cfg_attr(target_arch = "x86_64", link_name = "stat$INODE64")]
        pub fn stat(path: *const u8, st: *mut u8) -> i32;
        pub fn mkdir(path: *const u8, mode: u16) -> i32;
        pub fn unlink(path: *const u8) -> i32;
        #[cfg_attr(target_arch = "x86_64", link_name = "opendir$INODE64")]
        pub fn opendir(path: *const u8) -> *mut u8;
        #[cfg_attr(target_arch = "x86_64", link_name = "readdir$INODE64")]
        pub fn readdir(d: *mut u8) -> *const u8;
        pub fn closedir(d: *mut u8) -> i32;
        pub fn __error() -> *mut i32;
        pub fn malloc(n: usize) -> *mut u8;
        pub fn realloc(p: *mut u8, n: usize) -> *mut u8;
        pub fn free(p: *mut u8);
    }
}

const EINTR: i32 = 4;
const EEXIST: i32 = 17;

/// The program starts here.
#[unsafe(no_mangle)]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    *crate::ARGS.get() = (0..argc as usize).map(|i| String::from_utf8_lossy(unsafe { c_text(*argv.add(i)) }).into_owned()).collect();
    crate::run()
}

fn errno() -> i32 {
    unsafe { *c::__error() }
}

/// The text with the 0 byte the system wants after it.
fn c_path(s: &str) -> Vec<u8> {
    [s.as_bytes(), &[0]].concat()
}

pub fn var(name: &str) -> Option<String> {
    let v = unsafe { c::getenv(c_path(name).as_ptr()) };
    if v.is_null() { None } else { Some(String::from_utf8_lossy(unsafe { c_text(v) }).into_owned()) }
}

pub fn exit(code: i32) -> ! {
    unsafe { c::exit(code) }
}

fn write_all(fd: i32, mut b: &[u8]) -> bool {
    while !b.is_empty() {
        match unsafe { c::write(fd, b.as_ptr(), b.len()) } {
            n if n > 0 => b = &b[n as usize..],
            _ if errno() == EINTR => {}
            _ => return false,
        }
    }
    true
}

pub fn write(fd: usize, b: &[u8]) {
    write_all(fd as i32, b);
}

/// What was read (0 at the end, or after an error).
pub fn read(fd: usize, b: &mut [u8]) -> usize {
    loop {
        match unsafe { c::read(fd as i32, b.as_mut_ptr(), b.len()) } {
            n if n >= 0 => return n as usize,
            _ if errno() == EINTR => {}
            _ => return 0,
        }
    }
}

pub fn is_terminal(fd: usize) -> bool {
    unsafe { c::isatty(fd as i32) == 1 }
}

/// The nanoseconds since 1970 (UTC).
pub fn now() -> u64 {
    let mut t = [0; 2];
    unsafe { c::clock_gettime(0, &mut t) }; // CLOCK_REALTIME
    t[0] as u64 * 1_000_000_000 + t[1] as u64
}

pub fn sleep(s: f64) {
    let mut t = [s as i64, ((s - (s as i64) as f64) * 1e9) as i64];
    while unsafe { c::nanosleep(&t, &mut t) } != 0 && errno() == EINTR {}
}

pub fn pid() -> u64 {
    unsafe { c::getpid() as u64 }
}

// ---- Files ----

/// The last failure as an `IoError` variant + 1, like `failure` in src/interp.rs.
fn failure() -> i64 {
    match errno() {
        2 | 20 => 1,      // ENOENT, ENOTDIR
        1 | 13 | 30 => 2, // EPERM, EACCES, EROFS
        21 => 3,          // EISDIR
        _ => 5,
    }
}

fn open(path: &str, flags: i32) -> Result<i32, i64> {
    match unsafe { c::open(c_path(path).as_ptr(), flags | 0x1000000, 0o666u32) } {
        // O_CLOEXEC
        fd if fd >= 0 => Ok(fd),
        _ => Err(failure()),
    }
}

/// Ok(true) for a directory, Ok(false) for anything else that is there.
pub fn kind(path: &str) -> Result<bool, i64> {
    let mut st = [0u8; 256];
    if unsafe { c::stat(c_path(path).as_ptr(), st.as_mut_ptr()) } != 0 {
        return Err(failure());
    }
    Ok(u16::from_ne_bytes([st[4], st[5]]) & 0o170000 == 0o040000)
}

pub fn read_file(path: &str) -> Result<Vec<u8>, i64> {
    let fd = open(path, 0)?;
    let mut bytes = Vec::new();
    // In growing pieces: the size the system gives may be wrong (a pipe).
    let r = loop {
        bytes.reserve(1 << 16);
        let free = bytes.spare_capacity_mut();
        match unsafe { c::read(fd, free.as_mut_ptr() as *mut u8, free.len()) } {
            0 => break Ok(bytes),
            n if n > 0 => unsafe { bytes.set_len(bytes.len() + n as usize) },
            _ if errno() == EINTR => {}
            _ => break Err(5),
        }
    };
    unsafe { c::close(fd) };
    r
}

pub fn write_file(path: &str, b: &[u8], append: bool) -> Result<(), i64> {
    // O_WRONLY | O_CREAT, then O_APPEND or O_TRUNC.
    let fd = open(path, 0x1 | 0x200 | if append { 0x8 } else { 0x400 })?;
    let written = write_all(fd, b);
    if unsafe { c::close(fd) } != 0 || !written {
        return Err(5);
    }
    Ok(())
}

/// One directory; Err(0) if something is already there.
pub fn make_dir(path: &str) -> Result<(), i64> {
    if unsafe { c::mkdir(c_path(path).as_ptr(), 0o777) } == 0 {
        return Ok(());
    }
    if errno() == EEXIST { Err(0) } else { Err(failure()) }
}

pub fn remove_file(path: &str) -> Result<(), i64> {
    if unsafe { c::unlink(c_path(path).as_ptr()) } == 0 { Ok(()) } else { Err(failure()) }
}

/// The names in a directory, without `.` and `..`; Err(4) if one is not UTF-8.
pub fn list_dir(path: &str) -> Result<Vec<String>, i64> {
    let d = unsafe { c::opendir(c_path(path).as_ptr()) };
    if d.is_null() {
        return Err(failure());
    }
    let mut names = Vec::new();
    let r = loop {
        let e = unsafe { c::readdir(d) };
        if e.is_null() {
            break Ok(names);
        }
        // Each entry: its file number (8 bytes), place (8), length (2 + 2), kind (1), then the name.
        let name = unsafe { c_text(e.add(21)) };
        if name == b"." || name == b".." {
            continue;
        }
        match String::from_utf8(name.to_vec()) {
            Ok(name) => names.push(name),
            Err(_) => break Err(4),
        }
    };
    unsafe { c::closedir(d) };
    r
}

// ---- Memory: the system's (its blocks are aligned on 16 bytes, enough for the runtime) ----

pub struct Memory;

unsafe impl GlobalAlloc for Memory {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        unsafe { c::malloc(l.size()) }
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        unsafe { c::free(p) };
    }
    unsafe fn realloc(&self, p: *mut u8, _: Layout, n: usize) -> *mut u8 {
        unsafe { c::realloc(p, n) }
    }
}
