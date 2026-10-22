// What the runtime asks of Windows: `kernel32.dll` only, which every Windows has (no C library),
// and its own entry point `kova_start`. Windows speaks UTF-16: paths, names and words are
// turned from and into UTF-8 here.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{null, null_mut};

use crate::Global;

pub const SLASH: char = '\\';

type Handle = isize;
const INVALID: Handle = -1;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(n: u32) -> Handle;
    fn GetUserDefaultLocaleName(name: *mut u16, n: i32) -> i32;
    fn WriteFile(h: Handle, b: *const u8, n: u32, done: *mut u32, overlapped: *mut u8) -> i32;
    fn ReadFile(h: Handle, b: *mut u8, n: u32, done: *mut u32, overlapped: *mut u8) -> i32;
    fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
    fn SetConsoleOutputCP(page: u32) -> i32;
    fn SetConsoleCP(page: u32) -> i32;
    fn ExitProcess(code: u32) -> !;
    fn GetCommandLineW() -> *const u16;
    fn GetEnvironmentVariableW(name: *const u16, b: *mut u16, n: u32) -> u32;
    fn GetLastError() -> u32;
    fn GetProcessHeap() -> Handle;
    fn HeapAlloc(heap: Handle, flags: u32, n: usize) -> *mut u8;
    fn HeapReAlloc(heap: Handle, flags: u32, p: *mut u8, n: usize) -> *mut u8;
    fn HeapFree(heap: Handle, flags: u32, p: *mut u8) -> i32;
    fn CreateFileW(path: *const u16, access: u32, share: u32, security: *const u8, how: u32, flags: u32, model: Handle) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn CreateDirectoryW(path: *const u16, security: *const u8) -> i32;
    fn DeleteFileW(path: *const u16) -> i32;
    fn FindFirstFileW(pattern: *const u16, found: *mut u32) -> Handle;
    fn FindNextFileW(h: Handle, found: *mut u32) -> i32;
    fn FindClose(h: Handle) -> i32;
    fn GetSystemTimePreciseAsFileTime(t: *mut u64);
    fn Sleep(ms: u32);
    fn GetCurrentProcessId() -> u32;
}

/// The program starts here.
#[unsafe(no_mangle)]
pub extern "system" fn kova_start() -> ! {
    unsafe {
        // The console reads and shows UTF-8, the bytes `kova run` writes.
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
        let line = GetCommandLineW();
        let mut n = 0;
        while line.add(n).read_volatile() != 0 {
            n += 1;
        }
        *crate::ARGS.get() = words(core::slice::from_raw_parts(line, n));
    }
    crate::run()
}

fn utf8(w: &[u16]) -> String {
    char::decode_utf16(w.iter().copied()).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

/// The text with the 0 Windows wants after it, in UTF-16.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// The words of the command line, cut as Microsoft's C library does (the same as Rust's `std`,
/// so `kova run`'s): spaces separate words except between quotes; `\"` is a quote; `\` before a
/// quote is doubled; the program's name, first, is cut more simply.
fn words(line: &[u16]) -> Vec<String> {
    let (quote, backslash) = (b'"' as u16, b'\\' as u16);
    let space = |c: u16| c == b' ' as u16 || c == b'\t' as u16;
    let mut words = Vec::new();
    let mut i = 0;
    let name: &[u16] = if line.first() == Some(&quote) {
        let end = line[1..].iter().position(|&c| c == quote).map_or(line.len(), |k| k + 1);
        i = (end + 1).min(line.len());
        &line[1..end]
    } else {
        while i < line.len() && !space(line[i]) {
            i += 1;
        }
        &line[..i]
    };
    words.push(utf8(name));
    let (mut word, mut started, mut quoted, mut slashes) = (Vec::new(), false, false, 0);
    while i < line.len() {
        let c = line[i];
        i += 1;
        if c == backslash {
            slashes += 1;
            started = true;
            continue;
        }
        if c == quote {
            word.extend(core::iter::repeat_n(backslash, slashes / 2));
            if slashes % 2 == 1 {
                word.push(quote);
            } else if quoted && line.get(i) == Some(&quote) {
                word.push(quote);
                i += 1;
            } else {
                quoted = !quoted;
            }
            slashes = 0;
            started = true;
            continue;
        }
        word.extend(core::iter::repeat_n(backslash, slashes));
        slashes = 0;
        if space(c) && !quoted {
            if started {
                words.push(utf8(&word));
                word.clear();
                started = false;
            }
        } else {
            word.push(c);
            started = true;
        }
    }
    word.extend(core::iter::repeat_n(backslash, slashes));
    if started {
        words.push(utf8(&word));
    }
    words
}

/// Windows sets no `LANG`: the language of the user's settings (`fr-FR`).
pub fn system_language() -> Option<String> {
    let mut name = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    (n > 1).then(|| String::from_utf16_lossy(&name[..n as usize - 1]))
}

pub fn var(name: &str) -> Option<String> {
    let name = wide(name);
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), null_mut(), 0) };
    if n == 0 {
        return None;
    }
    let mut b = alloc::vec![0u16; n as usize];
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), b.as_mut_ptr(), n) };
    Some(utf8(&b[..n as usize]))
}

pub fn exit(code: i32) -> ! {
    unsafe { ExitProcess(code as u32) }
}

/// The handle of stdin (0), stdout (1) or stderr (2).
fn standard(fd: usize) -> Handle {
    unsafe { GetStdHandle((-10 - fd as i32) as u32) }
}

fn write_all(h: Handle, mut b: &[u8]) -> bool {
    while !b.is_empty() {
        let mut done = 0;
        let n = b.len().min(1 << 30) as u32;
        if unsafe { WriteFile(h, b.as_ptr(), n, &mut done, null_mut()) } == 0 {
            return false;
        }
        b = &b[done as usize..];
    }
    true
}

pub fn write(fd: usize, b: &[u8]) {
    write_all(standard(fd), b);
}

/// What was read (0 at the end, or after an error).
pub fn read(fd: usize, b: &mut [u8]) -> usize {
    let mut done = 0;
    let n = b.len().min(1 << 30) as u32;
    if unsafe { ReadFile(standard(fd), b.as_mut_ptr(), n, &mut done, null_mut()) } == 0 {
        return 0;
    }
    done as usize
}

pub fn is_terminal(fd: usize) -> bool {
    let mut mode = 0;
    unsafe { GetConsoleMode(standard(fd), &mut mode) != 0 }
}

/// The nanoseconds since 1970 (UTC).
pub fn now() -> u64 {
    let mut t = 0; // in 100 ns since 1601
    unsafe { GetSystemTimePreciseAsFileTime(&mut t) };
    t * 100 - 11_644_473_600_000_000_000
}

pub fn sleep(s: f64) {
    // ponytail: whole milliseconds, up to 46 days at a time.
    unsafe { Sleep(if s < 4e6 { (s * 1000.0) as u32 } else { 4_000_000_000 }) }
}

pub fn pid() -> u64 {
    unsafe { GetCurrentProcessId() as u64 }
}

// ---- Files ----

/// The last failure as an `IoError` variant + 1, like `failure` in src/interp.rs (Rust's `std`
/// reads Windows' numbers the same way).
fn failure() -> i64 {
    match unsafe { GetLastError() } {
        2 | 3 | 267 => 1, // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_DIRECTORY
        5 | 19 => 2,      // ERROR_ACCESS_DENIED, ERROR_WRITE_PROTECT
        _ => 5,
    }
}

/// Ok(true) for a directory, Ok(false) for anything else that is there.
pub fn kind(path: &str) -> Result<bool, i64> {
    match unsafe { GetFileAttributesW(wide(path).as_ptr()) } {
        u32::MAX => Err(failure()),
        a => Ok(a & 0x10 != 0), // FILE_ATTRIBUTE_DIRECTORY
    }
}

/// `access`, `how` as CreateFileW wants them; other programs may read, write or delete it meanwhile.
fn open(path: &str, access: u32, how: u32) -> Result<Handle, i64> {
    match unsafe { CreateFileW(wide(path).as_ptr(), access, 7, null(), how, 0x80, 0) } {
        INVALID => Err(failure()),
        h => Ok(h),
    }
}

pub fn read_file(path: &str) -> Result<Vec<u8>, i64> {
    let h = open(path, 0x8000_0000, 3)?; // GENERIC_READ, OPEN_EXISTING
    let mut bytes = Vec::new();
    let r = loop {
        bytes.reserve(1 << 16);
        let free = bytes.spare_capacity_mut();
        let mut done = 0;
        if unsafe { ReadFile(h, free.as_mut_ptr() as *mut u8, free.len() as u32, &mut done, null_mut()) } == 0 {
            break Err(5);
        }
        if done == 0 {
            break Ok(bytes);
        }
        unsafe { bytes.set_len(bytes.len() + done as usize) };
    };
    unsafe { CloseHandle(h) };
    r
}

pub fn write_file(path: &str, b: &[u8], append: bool) -> Result<(), i64> {
    // FILE_APPEND_DATA and OPEN_ALWAYS, or GENERIC_WRITE and CREATE_ALWAYS.
    let h = if append { open(path, 4, 4) } else { open(path, 0x4000_0000, 2) }?;
    let written = write_all(h, b);
    if unsafe { CloseHandle(h) } == 0 || !written {
        return Err(5);
    }
    Ok(())
}

/// One directory; Err(0) if something is already there.
pub fn make_dir(path: &str) -> Result<(), i64> {
    if unsafe { CreateDirectoryW(wide(path).as_ptr(), null()) } != 0 {
        return Ok(());
    }
    if unsafe { GetLastError() } == 183 { Err(0) } else { Err(failure()) } // ERROR_ALREADY_EXISTS
}

pub fn remove_file(path: &str) -> Result<(), i64> {
    if unsafe { DeleteFileW(wide(path).as_ptr()) } != 0 { Ok(()) } else { Err(failure()) }
}

/// The names in a directory, without `.` and `..`; Err(4) if one is not valid UTF-16.
pub fn list_dir(path: &str) -> Result<Vec<String>, i64> {
    // WIN32_FIND_DATAW: 592 bytes, the name (260 UTF-16 units ending with 0) from byte 44 on.
    let mut found = [0u32; 148];
    let h = unsafe { FindFirstFileW(wide(&alloc::format!("{path}\\*")).as_ptr(), found.as_mut_ptr()) };
    if h == INVALID {
        return Err(failure());
    }
    let mut names = Vec::new();
    let r = loop {
        let name = unsafe { core::slice::from_raw_parts((found.as_ptr() as *const u16).add(22), 260) };
        let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(260)];
        match char::decode_utf16(name.iter().copied()).collect::<Result<String, _>>() {
            Ok(n) if n == "." || n == ".." => {}
            Ok(n) => names.push(n),
            Err(_) => break Err(4),
        }
        if unsafe { FindNextFileW(h, found.as_mut_ptr()) } == 0 {
            break Ok(names);
        }
    };
    unsafe { FindClose(h) };
    r
}

// ---- Memory: Windows' own heap ----

static HEAP: Global<Handle> = Global::new(0);

fn heap() -> Handle {
    let h = HEAP.get();
    if *h == 0 {
        *h = unsafe { GetProcessHeap() };
    }
    *h
}

pub struct Memory;

// Windows' blocks are aligned on 16 bytes, enough for the runtime.
unsafe impl GlobalAlloc for Memory {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        unsafe { HeapAlloc(heap(), 0, l.size()) }
    }
    unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
        unsafe { HeapFree(heap(), 0, p) };
    }
    unsafe fn realloc(&self, p: *mut u8, _: Layout, n: usize) -> *mut u8 {
        unsafe { HeapReAlloc(heap(), 0, p, n) }
    }
}
