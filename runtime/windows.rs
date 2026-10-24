// What the runtime asks of Windows: `kernel32.dll` (and `ws2_32.dll` for the network), which every
// Windows has (no C library), and its own entry point `kova_start`. Windows speaks UTF-16: paths, names and words are
// turned from and into UTF-8 here.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{null, null_mut};

use crate::Global;
use crate::http::{Fail, Network, Serve};

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
    fn FileTimeToLocalFileTime(utc: *const u64, local: *mut u64) -> i32;
    fn Sleep(ms: u32);
    fn GetCurrentProcessId() -> u32;
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
    fn CreateThread(security: *const u8, stack: usize, start: extern "system" fn(*mut u8) -> u32, arg: *mut u8, flags: u32, id: *mut u32) -> Handle;
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

/// The local time in seconds since 1970, by the time zone of Windows' settings.
pub fn local_now() -> i64 {
    let (mut utc, mut local) = (0u64, 0u64);
    unsafe {
        GetSystemTimePreciseAsFileTime(&mut utc);
        FileTimeToLocalFileTime(&utc, &mut local);
    }
    (local / 10_000_000) as i64 - 11_644_473_600
}

pub fn sleep(s: f64) {
    // ponytail: whole milliseconds, up to 46 days at a time.
    unsafe { Sleep(if s < 4e6 { (s * 1000.0) as u32 } else { 4_000_000_000 }) }
}

pub fn pid() -> u64 {
    unsafe { GetCurrentProcessId() as u64 }
}

// ---- The network: `ws2_32.dll`, which every Windows has too ----

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSAGetLastError() -> i32;
    fn getaddrinfo(host: *const u8, port: *const u8, hints: *const AddrInfo, found: *mut *mut AddrInfo) -> i32;
    fn freeaddrinfo(found: *mut AddrInfo);
    fn socket(family: i32, kind: i32, protocol: i32) -> usize;
    fn setsockopt(s: usize, level: i32, name: i32, value: *const u32, n: i32) -> i32;
    fn connect(s: usize, address: *const u8, n: i32) -> i32;
    fn send(s: usize, b: *const u8, n: i32, flags: i32) -> i32;
    fn recv(s: usize, b: *mut u8, n: i32, flags: i32) -> i32;
    fn closesocket(s: usize) -> i32;
    fn bind(s: usize, address: *const u8, n: i32) -> i32;
    fn listen(s: usize, backlog: i32) -> i32;
    fn accept(s: usize, address: *mut u8, n: *mut i32) -> usize;
    fn WSAPoll(fds: *mut Poll, n: u32, ms: i32) -> i32;
    fn gethostname(name: *mut u8, n: i32) -> i32;
}

/// `WSAPOLLFD`: the socket, then what to wait for and what came.
#[repr(C)]
struct Poll {
    fd: usize,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct AddrInfo {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addrlen: usize,
    canonname: *const u8,
    addr: *const u8,
    next: *mut AddrInfo,
}

const SOCK_STREAM: i32 = 1;
const WSAEINTR: i32 = 10004;
const WSAEWOULDBLOCK: i32 = 10035;
const WSAETIMEDOUT: i32 = 10060;

pub struct Net;

impl Network for Net {
    type Link = usize;

    /// Never `secure`: src/schannel.rs encrypts around it.
    fn open(&mut self, host: &str, port: u16, _: bool, ms: u64) -> Result<usize, Fail> {
        if host.contains('\0') {
            return Err(Fail::NoHost);
        }
        // Windows counts the calls: one more each time costs nothing.
        let mut data = [0u8; 512];
        unsafe { WSAStartup(0x202, data.as_mut_ptr()) };
        let (host, port) = ([host.as_bytes(), &[0]].concat(), alloc::format!("{port}\0"));
        let hints = AddrInfo { flags: 0, family: 0, socktype: SOCK_STREAM, protocol: 0, addrlen: 0, canonname: null(), addr: null(), next: null_mut() };
        let mut found = null_mut();
        if unsafe { getaddrinfo(host.as_ptr(), port.as_ptr(), &hints, &mut found) } != 0 {
            return Err(Fail::NoHost);
        }
        let (mut failure, mut a) = (Err(Fail::NoHost), found);
        while let Some(i) = unsafe { a.as_ref() } {
            let s = unsafe { socket(i.family, SOCK_STREAM, i.protocol) };
            if s != usize::MAX {
                if ms > 0 {
                    // SO_RCVTIMEO, SO_SNDTIMEO: milliseconds; TCP_MAXRT (which `connect` follows): seconds.
                    for option in [0x1006, 0x1005] {
                        unsafe { setsockopt(s, 0xFFFF, option, &(ms.min(u32::MAX as u64) as u32), 4) };
                    }
                    unsafe { setsockopt(s, 6, 5, &(ms.div_ceil(1000).min(u32::MAX as u64) as u32), 4) };
                }
                if unsafe { connect(s, i.addr, i.addrlen as i32) } == 0 {
                    failure = Ok(s);
                    break;
                }
                failure = Err(if unsafe { WSAGetLastError() } == WSAETIMEDOUT { Fail::Timeout } else { Fail::Refused });
                unsafe { closesocket(s) };
            }
            a = i.next;
        }
        unsafe { freeaddrinfo(found) };
        failure
    }

    fn send(&mut self, s: &mut usize, mut data: &[u8]) -> Result<(), Fail> {
        while !data.is_empty() {
            let n = data.len().min(1 << 30);
            match unsafe { send(*s, data.as_ptr(), n as i32, 0) } {
                n if n >= 0 => data = &data[n as usize..],
                _ if unsafe { WSAGetLastError() } == WSAEINTR => {}
                _ => return Err(cut()),
            }
        }
        Ok(())
    }

    fn receive(&mut self, s: &mut usize, buffer: &mut [u8]) -> Result<usize, Fail> {
        let n = buffer.len().min(1 << 30);
        loop {
            match unsafe { recv(*s, buffer.as_mut_ptr(), n as i32, 0) } {
                n if n >= 0 => return Ok(n as usize),
                _ if unsafe { WSAGetLastError() } == WSAEINTR => {}
                _ => return Err(cut()),
            }
        }
    }

    fn close(&mut self, s: usize) {
        unsafe { closesocket(s) };
    }
}

impl Serve for Net {
    type Listener = usize;

    /// Without SO_REUSEADDR, which on Windows would let another program take the port too.
    fn listen(&mut self, port: u16, everyone: bool) -> Option<usize> {
        let mut data = [0u8; 512];
        unsafe { WSAStartup(0x202, data.as_mut_ptr()) };
        let s = unsafe { socket(2, SOCK_STREAM, 0) };
        if s == usize::MAX {
            return None;
        }
        let mut a = [0u8; 16];
        a[0] = 2;
        a[2..4].copy_from_slice(&port.to_be_bytes());
        if !everyone {
            a[4..8].copy_from_slice(&[127, 0, 0, 1]);
        }
        if unsafe { bind(s, a.as_ptr(), 16) } != 0 || unsafe { listen(s, 128) } != 0 {
            unsafe { closesocket(s) };
            return None;
        }
        Some(s)
    }

    fn accept(&mut self, listener: &mut usize, ms: u64) -> Option<(usize, [u8; 4])> {
        let (mut from, mut n) = ([0u8; 16], 16i32);
        let s = unsafe { accept(*listener, from.as_mut_ptr(), &mut n) };
        if s == usize::MAX {
            return None;
        }
        for option in [0x1006, 0x1005] {
            unsafe { setsockopt(s, 0xFFFF, option, &(ms.min(u32::MAX as u64) as u32), 4) };
        }
        Some((s, [from[4], from[5], from[6], from[7]]))
    }

    /// The IPv4 addresses of this computer's own name: Windows gives every adapter's, without
    /// their names.
    fn addresses(&mut self) -> Vec<(String, [u8; 4])> {
        let mut name = [0u8; 256];
        let mut found = Vec::new();
        if unsafe { gethostname(name.as_mut_ptr(), 256) } != 0 {
            return found;
        }
        let hints = AddrInfo { flags: 0, family: 2, socktype: SOCK_STREAM, protocol: 0, addrlen: 0, canonname: null(), addr: null(), next: null_mut() };
        let mut list = null_mut();
        if unsafe { getaddrinfo(name.as_ptr(), null(), &hints, &mut list) } != 0 {
            return found;
        }
        let mut at = list;
        while let Some(i) = unsafe { at.as_ref() } {
            let ip = unsafe { core::slice::from_raw_parts(i.addr.add(4), 4) };
            found.push((String::new(), [ip[0], ip[1], ip[2], ip[3]]));
            at = i.next;
        }
        unsafe { freeaddrinfo(list) };
        found
    }

    /// `WSAPoll` without a time limit, POLLRDNORM (it refuses POLLIN's other half).
    fn ready(&mut self, listener: &usize, links: &[usize]) -> Vec<bool> {
        let mut fds: Vec<Poll> = links.iter().chain([listener]).map(|&fd| Poll { fd, events: 0x100, revents: 0 }).collect();
        if unsafe { WSAPoll(fds.as_mut_ptr(), fds.len() as u32, -1) } <= 0 {
            return alloc::vec![false; fds.len()];
        }
        fds.iter().map(|p| p.revents != 0).collect()
    }
}

/// Like `cut` in src/web.rs.
fn cut() -> Fail {
    if matches!(unsafe { WSAGetLastError() }, WSAETIMEDOUT | WSAEWOULDBLOCK) { Fail::Timeout } else { Fail::Broken }
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

// ---- Threads (`serve(…) workers(…)`) ----

/// What one thread at a time may do (`crate::Locked`).
pub struct Lock(core::cell::UnsafeCell<usize>);

unsafe impl Sync for Lock {}

impl Lock {
    pub const fn new() -> Lock {
        Lock(core::cell::UnsafeCell::new(0))
    }
    pub fn lock(&self) {
        unsafe { AcquireSRWLockExclusive(self.0.get()) }
    }
    pub fn unlock(&self) {
        unsafe { ReleaseSRWLockExclusive(self.0.get()) }
    }
}

/// `f(arg)` on a new thread, which never returns. False if the system cannot start it.
pub fn spawn(f: extern "C" fn(i64), arg: i64) -> bool {
    extern "system" fn begin(p: *mut u8) -> u32 {
        let (f, arg) = *unsafe { alloc::boxed::Box::from_raw(p as *mut (extern "C" fn(i64), i64)) };
        f(arg);
        0
    }
    let arg = alloc::boxed::Box::into_raw(alloc::boxed::Box::new((f, arg))) as *mut u8;
    // STACK_SIZE_PARAM_IS_A_RESERVATION: 8 MB kept for its stack, taken as it grows.
    let h = unsafe { CreateThread(core::ptr::null(), 8 << 20, begin, arg, 0x10000, core::ptr::null_mut()) };
    h != 0 && unsafe { CloseHandle(h) } != 0
}
