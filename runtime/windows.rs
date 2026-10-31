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
    fn SetConsoleMode(h: Handle, mode: u32) -> i32;
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn GetProcAddress(library: Handle, name: *const u8) -> Handle;
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
    fn GetFileAttributesExW(path: *const u16, level: u32, data: *mut u32) -> i32;
    fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
    fn FileTimeToSystemTime(t: *const u64, s: *mut [u16; 8]) -> i32;
    fn SystemTimeToTzSpecificLocalTime(zone: *const u8, utc: *const [u16; 8], local: *mut [u16; 8]) -> i32;
    fn SystemTimeToFileTime(s: *const [u16; 8], t: *mut u64) -> i32;
    fn CreateDirectoryW(path: *const u16, security: *const u8) -> i32;
    fn DeleteFileW(path: *const u16) -> i32;
    fn FindFirstFileW(pattern: *const u16, found: *mut u32) -> Handle;
    fn FindNextFileW(h: Handle, found: *mut u32) -> i32;
    fn FindClose(h: Handle) -> i32;
    fn GetSystemTimePreciseAsFileTime(t: *mut u64);
    fn GetActiveProcessorCount(group: u16) -> u32;
    fn FileTimeToLocalFileTime(utc: *const u64, local: *mut u64) -> i32;
    fn Sleep(ms: u32);
    fn GetCurrentProcessId() -> u32;
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn SleepConditionVariableSRW(condition: *mut usize, lock: *mut usize, milliseconds: u32, flags: u32) -> i32;
    fn WakeAllConditionVariable(condition: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
    fn CreateThread(security: *const u8, stack: usize, start: extern "system" fn(*mut u8) -> u32, arg: *mut u8, flags: u32, id: *mut u32) -> Handle;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn CreatePipe(read: *mut Handle, write: *mut Handle, security: *const usize, size: u32) -> i32;
    fn SetHandleInformation(h: Handle, mask: u32, flags: u32) -> i32;
    fn CreateProcessW(application: *const u16, line: *mut u16, process: *const u8, thread: *const u8, inherit: i32, flags: u32, environment: *const u8, folder: *const u16, start: *const usize, made: *mut usize) -> i32;
    fn GetExitCodeProcess(h: Handle, code: *mut u32) -> i32;
}

/// The program starts here.
#[unsafe(no_mangle)]
pub extern "system" fn kova_start() -> ! {
    unsafe {
        // The console reads and shows UTF-8, the bytes `kova run` writes.
        SetConsoleOutputCP(65001);
        SetConsoleCP(65001);
        // It reads the colors of `s.red` too (ENABLE_VIRTUAL_TERMINAL_PROCESSING).
        let mut mode = 0;
        if GetConsoleMode(standard(1), &mut mode) != 0 {
            SetConsoleMode(standard(1), mode | 4);
        }
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
pub fn cores() -> i64 {
    // ALL_PROCESSOR_GROUPS.
    (unsafe { GetActiveProcessorCount(0xffff) } as i64).max(1)
}

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

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: Handle, b: *mut u8, n: u32, flags: u32) -> i32;
}

/// Fills `b` with the system's unguessable chance (`secure.random`).
pub fn chance(b: &mut [u8]) {
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG
    if unsafe { BCryptGenRandom(0, b.as_mut_ptr(), b.len() as u32, 2) } != 0 {
        panic!("BCryptGenRandom");
    }
}

// ---- The network: `ws2_32.dll`, which every Windows has too ----

unsafe extern "system" {
    fn CancelIoEx(file: usize, overlapped: *const u8) -> i32;
}

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
    fn sendto(s: usize, b: *const u8, n: i32, flags: i32, to: *const u8, size: i32) -> i32;
    fn recvfrom(s: usize, b: *mut u8, n: i32, flags: i32, from: *mut u8, size: *mut i32) -> i32;
    fn listen(s: usize, backlog: i32) -> i32;
    fn accept(s: usize, address: *mut u8, n: *mut i32) -> usize;
    fn shutdown(s: usize, how: i32) -> i32;
    fn getsockname(s: usize, address: *mut u8, n: *mut i32) -> i32;
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

    /// `shutdown` alone leaves a blocked `recv` waiting: its waiting is an operation of the socket.
    fn wake(&mut self, s: &usize) {
        unsafe {
            shutdown(*s, 2);
            CancelIoEx(*s, null());
        }
    }

    fn close(&mut self, s: usize) {
        unsafe { closesocket(s) };
    }
}

/// The port of a socket: a `sockaddr_in` holds it after the family.
fn port_of(s: usize) -> u16 {
    let (mut a, mut n) = ([0u8; 16], 16i32);
    unsafe { getsockname(s, a.as_mut_ptr(), &mut n) };
    u16::from_be_bytes([a[2], a[3]])
}

impl Serve for Net {
    type Listener = usize;
    type Datagram = usize;

    fn port(&mut self, s: &usize) -> u16 {
        port_of(*s)
    }

    fn datagram_port(&mut self, s: &usize) -> u16 {
        port_of(*s)
    }

    fn stop(&mut self, s: usize) {
        unsafe { closesocket(s) };
    }

    fn udp(&mut self, port: u16, everyone: bool, ms: u64) -> Option<usize> {
        let mut data = [0u8; 512];
        unsafe { WSAStartup(0x202, data.as_mut_ptr()) };
        let s = unsafe { socket(2, 2, 0) };
        if s == usize::MAX {
            return None;
        }
        let mut a = [0u8; 16];
        a[0] = 2;
        a[2..4].copy_from_slice(&port.to_be_bytes());
        if !everyone {
            a[4..8].copy_from_slice(&[127, 0, 0, 1]);
        }
        if unsafe { bind(s, a.as_ptr(), 16) } != 0 {
            unsafe { closesocket(s) };
            return None;
        }
        unsafe { setsockopt(s, 0xFFFF, 0x1006, &(ms.min(u32::MAX as u64) as u32), 4) };
        Some(s)
    }

    fn send_to(&mut self, s: &usize, host: &str, port: u16, data: &[u8]) -> Result<(), Fail> {
        if host.contains('\0') {
            return Err(Fail::NoHost);
        }
        let (host, port) = ([host.as_bytes(), &[0]].concat(), alloc::format!("{port}\0"));
        let hints = AddrInfo { flags: 0, family: 2, socktype: 2, protocol: 0, addrlen: 0, canonname: null(), addr: null(), next: null_mut() };
        let mut found = null_mut();
        if unsafe { getaddrinfo(host.as_ptr(), port.as_ptr(), &hints, &mut found) } != 0 {
            return Err(Fail::NoHost);
        }
        let Some(i) = (unsafe { found.as_ref() }) else { return Err(Fail::NoHost) };
        let sent = unsafe { sendto(*s, data.as_ptr(), data.len() as i32, 0, i.addr, i.addrlen as i32) };
        unsafe { freeaddrinfo(found) };
        if sent < 0 { Err(Fail::Broken) } else { Ok(()) }
    }

    fn receive_from(&mut self, s: &usize, buffer: &mut [u8]) -> Result<(usize, [u8; 4], u16), Fail> {
        let (mut from, mut n) = ([0u8; 16], 16i32);
        let got = unsafe { recvfrom(*s, buffer.as_mut_ptr(), buffer.len() as i32, 0, from.as_mut_ptr(), &mut n) };
        if got >= 0 {
            return Ok((got as usize, [from[4], from[5], from[6], from[7]], u16::from_be_bytes([from[2], from[3]])));
        }
        // WSAETIMEDOUT.
        Err(if unsafe { WSAGetLastError() } == 10060 { Fail::Timeout } else { Fail::Broken })
    }

    fn close_datagram(&mut self, s: usize) {
        unsafe { closesocket(s) };
    }

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

/// What `files.size` and `files.modified` say, of the link itself if it is one (`follow`
/// unused: ponytail, links are rare on Windows; open the file to follow them). A link is what
/// Rust's `is_symlink` calls one (`kova run`): a reparse point whose tag is a name surrogate
/// (symbolic link, junction), not a OneDrive folder.
pub fn info(path: &str, _follow: bool) -> Result<crate::Info, i64> {
    // WIN32_FILE_ATTRIBUTE_DATA: attributes, 3 times (creation, access, writing), size high, low.
    let mut d = [0u32; 9];
    if unsafe { GetFileAttributesExW(wide(path).as_ptr(), 0, d.as_mut_ptr()) } == 0 {
        return Err(failure());
    }
    let written = (d[6] as u64) << 32 | d[5] as u64; // a FILETIME: low half first
    let modified = (written / 10_000_000) as i64 - 11_644_473_600;
    let size = ((d[7] as u64) << 32 | d[8] as u64) as i64;
    let link = d[0] & 0x400 != 0 && {
        // WIN32_FIND_DATAW: the tag after the attributes, the 3 times and the size.
        let mut found = [0u32; 148];
        let h = unsafe { FindFirstFileW(wide(path).as_ptr(), found.as_mut_ptr()) };
        h != INVALID && unsafe { FindClose(h) } != 0 && found[9] & 0x2000_0000 != 0
    };
    Ok(crate::Info { folder: d[0] & 0x10 != 0, link, size, modified })
}

/// The local time at `utc` (seconds since 1970), by the rules of that day (summer time or not).
pub fn local_at(utc: i64) -> i64 {
    let t = ((utc + 11_644_473_600) * 10_000_000) as u64;
    let (mut s, mut local, mut back) = ([0u16; 8], [0u16; 8], 0u64);
    unsafe {
        if FileTimeToSystemTime(&t, &mut s) == 0 || SystemTimeToTzSpecificLocalTime(null(), &s, &mut local) == 0 || SystemTimeToFileTime(&local, &mut back) == 0 {
            return utc;
        }
    }
    (back / 10_000_000) as i64 - 11_644_473_600
}

/// `a` named `b` from now on (replacing a file `b`); Err(0) if they are on two disks.
pub fn rename(a: &str, b: &str) -> Result<(), i64> {
    if unsafe { MoveFileExW(wide(a).as_ptr(), wide(b).as_ptr(), 1) } != 0 {
        return Ok(());
    }
    if unsafe { GetLastError() } == 17 { Err(0) } else { Err(failure()) } // ERROR_NOT_SAME_DEVICE
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

// ---- Other programs ----

/// Held while a process is made: the pipe ends it inherits must be only its own.
static SPAWNING: Lock = Lock::new();

/// `system.run`: runs `program` (a path) with `args`, its input empty, and gives what it wrote,
/// what it wrote on stderr, and its exit code; or the `RunError` variant + 1. The command line
/// is written as `kova run` (Rust's `std::process::Command`) writes it.
pub fn run(program: &str, args: &[&str]) -> Result<(Vec<u8>, Vec<u8>, i64), i64> {
    let mut line = wide(&crate::shared::command_line(program, args));
    let path = wide(program);
    // SECURITY_ATTRIBUTES: its length, no descriptor, inherited.
    let inherit: [usize; 3] = [24, 0, 1];
    let mut pipes = [(INVALID, INVALID); 2];
    SPAWNING.lock();
    let made = pipes.iter_mut().all(|(r, w)| unsafe { CreatePipe(r, w, inherit.as_ptr(), 0) != 0 && SetHandleInformation(*r, 1, 0) != 0 });
    // GENERIC_READ, shared, OPEN_EXISTING.
    let nothing = unsafe { CreateFileW(wide("NUL").as_ptr(), 0x8000_0000, 7, inherit.as_ptr() as *const u8, 3, 0, 0) };
    // STARTUPINFOW (104 bytes): its length, STARTF_USESTDHANDLES, the three handles.
    let mut start = [0usize; 13];
    start[0] = 104;
    start[7] = 0x100 << 32;
    start[10..13].copy_from_slice(&[nothing as usize, pipes[0].1 as usize, pipes[1].1 as usize]);
    let mut process = [0usize; 3];
    let ok = made && nothing != INVALID && unsafe { CreateProcessW(path.as_ptr(), line.as_mut_ptr(), null(), null(), 1, 0, null(), null(), start.as_ptr(), process.as_mut_ptr()) } != 0;
    let e = unsafe { GetLastError() };
    for h in [nothing, pipes[0].1, pipes[1].1] {
        unsafe { CloseHandle(h) };
    }
    SPAWNING.unlock();
    if !ok {
        for (r, _) in pipes {
            unsafe { CloseHandle(r) };
        }
        return Err(match e {
            2 | 3 => 1, // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND
            5 => 2,     // ERROR_ACCESS_DENIED
            _ => 3,
        });
    }
    // stderr read by another thread meanwhile: a program filling one pipe while this one waits
    // on the other would stop.
    extern "C" fn drain(p: i64) {
        let (h, bytes) = unsafe { &mut *(p as *mut (Handle, Vec<u8>)) };
        read_all(*h, bytes);
    }
    let mut errors = alloc::boxed::Box::new((pipes[1].0, Vec::new()));
    let reader = spawn(drain, &mut *errors as *mut (Handle, Vec<u8>) as i64);
    let mut output = Vec::new();
    read_all(pipes[0].0, &mut output);
    match reader {
        Some(t) => join(t),
        None => read_all(pipes[1].0, &mut errors.1),
    }
    let mut code = 0u32;
    unsafe {
        WaitForSingleObject(process[0] as Handle, u32::MAX);
        GetExitCodeProcess(process[0] as Handle, &mut code);
        for h in [process[0], process[1]].iter().map(|&h| h as Handle).chain([pipes[0].0, pipes[1].0]) {
            CloseHandle(h);
        }
    }
    Ok((output, errors.1, code as i64))
}

/// Everything until the other end is closed.
fn read_all(h: Handle, bytes: &mut Vec<u8>) {
    let mut buffer = [0u8; 1 << 14];
    loop {
        let mut done = 0;
        if unsafe { ReadFile(h, buffer.as_mut_ptr(), buffer.len() as u32, &mut done, null_mut()) } == 0 || done == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..done as usize]);
    }
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

// `WaitOnAddress` is not in KERNEL32.dll: one lock and one condition for every word.
// ponytail: a wake wakes every sleeping thread; a table of conditions if thousands sleep.
struct Sleepers(core::cell::UnsafeCell<[usize; 2]>);

unsafe impl Sync for Sleepers {}

static SLEEPERS: Sleepers = Sleepers(core::cell::UnsafeCell::new([0; 2]));

/// Sleeps while `word` is `seen`.
pub fn wait(word: &core::sync::atomic::AtomicU32, seen: u32) {
    let [lock, condition] = unsafe { &mut *SLEEPERS.0.get() };
    unsafe {
        AcquireSRWLockExclusive(lock);
        if word.load(core::sync::atomic::Ordering::SeqCst) == seen {
            SleepConditionVariableSRW(condition, lock, u32::MAX, 0);
        }
        ReleaseSRWLockExclusive(lock);
    }
}

/// Wakes those sleeping on `word`.
pub fn wait_for(word: &core::sync::atomic::AtomicU32, seen: u32, ns: u64) {
    let [lock, condition] = unsafe { &mut *SLEEPERS.0.get() };
    let ms = (ns / 1_000_000 + 1).min(u32::MAX as u64 - 1) as u32;
    unsafe {
        AcquireSRWLockExclusive(lock);
        if word.load(core::sync::atomic::Ordering::SeqCst) == seen {
            SleepConditionVariableSRW(condition, lock, ms, 0);
        }
        ReleaseSRWLockExclusive(lock);
    }
}

pub fn wake(_word: &core::sync::atomic::AtomicU32) {
    let [lock, condition] = unsafe { &mut *SLEEPERS.0.get() };
    unsafe {
        // Passing through the lock: a thread between its test and its sleep is not missed.
        AcquireSRWLockExclusive(lock);
        ReleaseSRWLockExclusive(lock);
        WakeAllConditionVariable(condition);
    }
}

/// A thread `spawn` started.
pub struct Thread(Handle);

/// `f(arg)` on a new thread. None if the system cannot start it.
pub fn spawn(f: extern "C" fn(i64), arg: i64) -> Option<Thread> {
    extern "system" fn begin(p: *mut u8) -> u32 {
        let (f, arg) = *unsafe { alloc::boxed::Box::from_raw(p as *mut (extern "C" fn(i64), i64)) };
        f(arg);
        0
    }
    let arg = alloc::boxed::Box::into_raw(alloc::boxed::Box::new((f, arg))) as *mut u8;
    // STACK_SIZE_PARAM_IS_A_RESERVATION: 8 MB kept for its stack, taken as it grows.
    let h = unsafe { CreateThread(core::ptr::null(), 8 << 20, begin, arg, 0x10000, core::ptr::null_mut()) };
    (h != 0).then_some(Thread(h))
}

/// Waits for the end of `t`.
pub fn join(t: Thread) {
    unsafe {
        WaitForSingleObject(t.0, u32::MAX);
        CloseHandle(t.0);
    }
}

/// `extern` (PLAN Part 4 "`extern`, step 1"): a DLL loaded by Windows, or None.
pub fn library(name: &str) -> Option<usize> {
    let h = unsafe { LoadLibraryW(wide(name).as_ptr()) };
    (h != 0).then_some(h as usize)
}

/// A function of a DLL loaded by `library`, or None.
pub fn symbol(library: usize, name: &str) -> Option<usize> {
    let name = [name.as_bytes(), &[0]].concat();
    let f = unsafe { GetProcAddress(library as Handle, name.as_ptr()) };
    (f != 0).then_some(f as usize)
}

/// `ui.show` (src/ui.rs) with Win32 (`user32.dll`, `gdi32.dll`, found at the first call): the
/// window stays, its controls are made again from `view` each time. Win32 has no layout: `win_size`
/// and `win_place` put a column's parts under each other, a row's side by side. When the view's
/// shape has not changed, the controls stay and only move and take their new texts, so a field
/// being typed into keeps its cursor. The button clicked, a field changed (`shared::ui_changed`,
/// its text in `typed`), or -1 once the window is closed. `KOVA_UI_HIDDEN` never shows the window (to drive it
/// from another program).
pub fn ui_show(title: &str, view: &crate::shared::UiNode, menus: &[crate::shared::UiMenu], width: i32, height: i32, acts: &[i64], typed: &mut String) -> i64 {
    static WINDOW: Global<Handle> = Global::new(0);
    static CONTROLS: Global<Vec<Handle>> = Global::new(Vec::new());
    static SHAPE: Global<String> = Global::new(String::new());
    // `WAITING`: nothing yet, -1: the window closed.
    static CLICKED: Global<i64> = Global::new(WAITING);
    static TYPED: Global<String> = Global::new(String::new());
    // The view on the screen, while `ui_show` waits: resizing the window places it again.
    static SHOWN: Global<*const crate::shared::UiNode> = Global::new(null());
    const WAITING: i64 = i64::MIN;
    /// Makes the controls, or moves the ones there, for the window's size now.
    fn arrange(win: &Win32, window: Handle, view: &crate::shared::UiNode, same: bool) {
        let dc = (win.get_dc)(window);
        (win.select)(dc, win.font);
        let mut now = [0; 4];
        (win.client)(window, &mut now);
        *SETTING.get() = true;
        let edge = win_edge();
        win_place(win, dc, window, view, Default::default(), edge, edge, now[2] - now[0] - 2 * edge, CONTROLS.get(), same, &mut 0);
        *SETTING.get() = false;
        (win.release_dc)(window, dc);
    }
    extern "system" fn procedure(h: Handle, message: u32, w: usize, l: isize) -> isize {
        let win = WIN.get().as_ref().expect("made before the window");
        let id = w & 0xffff;
        match message {
            // WM_COMMAND from a button (BN_CLICKED = 0): its number + 1 is its id.
            0x0111 if w >> 16 == 0 && id > 0 && id < FIELD => {
                *CLICKED.get() = id as i64 - 1;
                // A message sent by another program is handled inside `GetMessageW`, which then
                // waits for a posted one: WM_NULL, posted to the thread (the window may be gone), wakes it.
                (win.post)(0, 0, 0, 0);
                0
            }
            // From a field (EN_CHANGE), unless its text was set from here.
            0x0111 if w >> 16 == 0x0300 && id >= FIELD && !*SETTING.get() => {
                *TYPED.get() = win_text(win, l as Handle);
                *CLICKED.get() = crate::shared::ui_changed(id - FIELD);
                (win.post)(0, 0, 0, 0);
                0
            }
            // WM_COMMAND from a check box (BN_CLICKED) or a drop-down list (CBN_SELCHANGE).
            0x0111 if w >> 16 == 0 && id >= FIELD && !*SETTING.get() => {
                *TYPED.get() = (if (win.send)(l as Handle, 0x00F0, 0, 0) == 1 { "on" } else { "off" }).into();
                *CLICKED.get() = crate::shared::ui_changed(id - FIELD);
                (win.post)(0, 0, 0, 0);
                0
            }
            0x0111 if w >> 16 == 1 && id >= FIELD && !*SETTING.get() => {
                *TYPED.get() = alloc::format!("{}", (win.send)(l as Handle, 0x0147, 0, 0));
                *CLICKED.get() = crate::shared::ui_changed(id - FIELD);
                (win.post)(0, 0, 0, 0);
                0
            }
            // WM_HSCROLL from a slider: TBM_GETPOS.
            0x0114 if l != 0 && !*SETTING.get() => {
                let at = alloc::format!("{}", (win.send)(l as Handle, 0x0400, 0, 0));
                let number = (win.control_id)(l as Handle) as usize - FIELD;
                if *TYPED.get() != at || *CLICKED.get() == WAITING {
                    *TYPED.get() = at;
                    *CLICKED.get() = crate::shared::ui_changed(number);
                    (win.post)(0, 0, 0, 0);
                }
                0
            }
            // WM_NOTIFY, LVN_ITEMCHANGED from a list: `NMLISTVIEW` holds the line, then its states.
            0x004E if !*SETTING.get() => {
                let note = l as *const u8;
                let read = |at: usize| unsafe { (note.add(at) as *const i32).read_unaligned() };
                if read(16) == -101 && read(32) & 2 != 0 && read(36) & 2 == 0 {
                    *TYPED.get() = alloc::format!("{}", read(24));
                    *CLICKED.get() = crate::shared::ui_changed(read(8) as usize - FIELD);
                    (win.post)(0, 0, 0, 0);
                }
                (win.default)(h, message, w, l)
            }
            // WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC: a text color goes on top of the system's defaults.
            0x0133 | 0x0138 => {
                let brush = (win.default)(h, message, w, l);
                if let Some(&(_, color)) = COLORED.get().iter().find(|(c, _)| *c == l as Handle) {
                    (win.text_color)(w as Handle, color);
                }
                brush
            }
            // WM_TIMER: a clock's id is its button number + 1.
            0x0113 => {
                WIN_HEARD.get().hear(w as i64 - 1, String::new());
                0
            }
            // WM_SIZE
            0x0005 if !SHOWN.get().is_null() => {
                arrange(win, h, unsafe { &**SHOWN.get() }, true);
                0
            }
            // WM_CLOSE
            0x0010 => {
                *CLICKED.get() = -1;
                (win.post)(0, 0, 0, 0);
                (win.destroy)(h);
                *WINDOW.get() = 0;
                0
            }
            _ => (win.default)(h, message, w, l),
        }
    }
    if *CLICKED.get() == -1 {
        return -1;
    }
    // Before the controls are made: a click or a close may come while they are.
    *CLICKED.get() = WAITING;
    unsafe {
        let fresh = WIN.get().is_none();
        if fresh {
            let (Some(user), Some(gdi), Some(kernel)) = (library("user32.dll"), library("gdi32.dll"), library("kernel32.dll")) else { return -1 };
            let f = |l: usize, name: &str| symbol(l, name).unwrap_or(0);
            use core::mem::transmute as t;
            *WIN.get() = Some(Win32 {
                create: t(f(user, "CreateWindowExW")),
                destroy: t(f(user, "DestroyWindow")),
                set_text: t(f(user, "SetWindowTextW")),
                get_text: t(f(user, "GetWindowTextW")),
                text_length: t(f(user, "GetWindowTextLengthW")),
                control_id: t(f(user, "GetDlgCtrlID")),
                send: t(f(user, "SendMessageW")),
                post: t(f(user, "PostMessageW")),
                default: t(f(user, "DefWindowProcW")),
                get_dc: t(f(user, "GetDC")),
                release_dc: t(f(user, "ReleaseDC")),
                font: (t::<usize, extern "system" fn(i32) -> Handle>(f(gdi, "GetStockObject")))(17), // DEFAULT_GUI_FONT
                select: t(f(gdi, "SelectObject")),
                extent: t(f(gdi, "GetTextExtentPoint32W")),
                object: t(f(gdi, "GetObjectW")),
                new_font: t(f(gdi, "CreateFontIndirectW")),
                text_color: t(f(gdi, "SetTextColor")),
                invalidate: t(f(user, "InvalidateRect")),
                adjust: t(f(user, "AdjustWindowRect")),
                client: t(f(user, "GetClientRect")),
                set_pos: t(f(user, "SetWindowPos")),
                message: t(f(user, "GetMessageW")),
                translate: t(f(user, "TranslateMessage")),
                dispatch: t(f(user, "DispatchMessageW")),
            });
            let win = WIN.get().as_ref().unwrap();
            let module: extern "system" fn(*const u16) -> Handle = t(f(kernel, "GetModuleHandleW"));
            let cursor: extern "system" fn(Handle, usize) -> Handle = t(f(user, "LoadCursorW"));
            let register: extern "system" fn(*const WindowClass) -> u16 = t(f(user, "RegisterClassExW"));
            let class = wide("KovaWindow");
            let instance = module(null());
            // The `header`'s icon, resource number 1 of the executable (src/link.rs `resources`).
            let image: extern "system" fn(Handle, usize, u32, i32, i32, u32) -> Handle = t(f(user, "LoadImageW"));
            let (icon, small_icon) = match crate::icon().is_empty() {
                true => (0, 0),
                false => (image(instance, 1, 1, 0, 0, 0x40), image(instance, 1, 1, 16, 16, 0)), // IMAGE_ICON, LR_DEFAULTSIZE
            };
            let canvas_class = wide("KovaCanvas");
            register(&WindowClass {
                size: core::mem::size_of::<WindowClass>() as u32,
                style: 0x4000, // CS_GLOBALCLASS: `win_place` gives no instance
                procedure: win_canvas as usize,
                extra: [0, 0],
                instance,
                icon: 0,
                cursor: cursor(0, 32512),
                background: 0, // it paints everything itself
                menu: null(),
                class: canvas_class.as_ptr(),
                small_icon: 0,
            });
            register(&WindowClass {
                size: core::mem::size_of::<WindowClass>() as u32,
                style: 0,
                procedure: procedure as usize,
                extra: [0, 0],
                instance,
                icon,
                cursor: cursor(0, 32512), // IDC_ARROW
                background: 16,           // COLOR_BTNFACE + 1
                menu: null(),
                class: class.as_ptr(),
                small_icon,
            });
            // WS_OVERLAPPEDWINDOW, or WS_POPUP with the taskbar's minimize and maximize;
            // CW_USEDEFAULT for the place.
            if acts.contains(&0) {
                *WIN_STYLE.get() = 0x8003_0000;
            }
            let w = (win.create)(0, class.as_ptr(), wide(title).as_ptr(), *WIN_STYLE.get(), i32::MIN, i32::MIN, 320, 200, 0, 0, instance, null_mut());
            *WINDOW.get() = w;
            let show: extern "system" fn(Handle, i32) -> i32 = t(f(user, "ShowWindow"));
            if var("KOVA_UI_HIDDEN").is_none() {
                show(w, 5); // SW_SHOW
            }
        }
        let win = WIN.get().as_ref().unwrap();
        let w = *WINDOW.get();
        (win.set_text)(w, wide(title).as_ptr());
        static MENUS: Global<Vec<crate::shared::UiMenu>> = Global::new(Vec::new());
        if *MENUS.get() != menus {
            win_menus(w, menus);
            MENUS.get().clear();
            MENUS.get().extend_from_slice(menus);
        }
        let shape = crate::shared::ui_shape(view);
        let same = shape == *SHAPE.get();
        if !same {
            for c in CONTROLS.get().drain(..) {
                (win.destroy)(c);
            }
            COLORED.get().clear();
            WIN_CANVASES.get().clear();
            *SHAPE.get() = shape;
        }
        let dc = (win.get_dc)(w);
        (win.select)(dc, win.font);
        let (content_w, content_h) = win_size(win, dc, view);
        (win.release_dc)(w, dc);
        arrange(win, w, view, same);
        *SHOWN.get() = view;
        let user = |name: &str| library("user32.dll").and_then(|l| symbol(l, name)).unwrap_or(0);
        let clocks = crate::shared::ui_clocks(view);
        if clocks != *WIN_CLOCKS_ASKED.get() {
            let set_timer: extern "system" fn(Handle, usize, u32, usize) -> usize = core::mem::transmute(user("SetTimer"));
            let kill_timer: extern "system" fn(Handle, usize) -> i32 = core::mem::transmute(user("KillTimer"));
            for clock in WIN_CLOCKS.get().drain(..) {
                kill_timer(w, clock);
            }
            for &(ms, n) in &clocks {
                WIN_CLOCKS.get().push(set_timer(w, n + 1, ms as u32, 0));
            }
            *WIN_CLOCKS_ASKED.get() = clocks;
        }
        if !same {
            let focus: extern "system" fn(Handle) -> Handle = core::mem::transmute(user("SetFocus"));
            if let Some((hearing, _)) = WIN_CANVASES.get().iter().find(|(_, c)| c.key.is_some()) {
                focus(*hearing);
            }
        }
        // The window first takes the size the program asked for, else fits its content, 16 pixels
        // around it, at least 320 pixels wide; never smaller than the content. Later it only grows
        // when the content needs more room, so a size the user chose stays.
        let (mut cw, mut ch) = (if width > 0 { width } else { 320 }, height.max(0));
        if !fresh {
            let mut now = [0; 4];
            (win.client)(w, &mut now);
            (cw, ch) = (now[2] - now[0], now[3] - now[1]);
        }
        let (want_w, want_h) = ((content_w + 2 * win_edge()).max(cw), (content_h + 2 * win_edge()).max(ch));
        if fresh || want_w > cw || want_h > ch {
            let mut r = [0, 0, want_w, want_h];
            (win.adjust)(&mut r, *WIN_STYLE.get(), 0);
            (win.set_pos)(w, 0, 0, 0, r[2] - r[0], r[3] - r[1], 0x2 | 0x4); // SWP_NOMOVE | SWP_NOZORDER
        }
        let user = |name: &str| library("user32.dll").and_then(|l| symbol(l, name)).unwrap_or(0);
        let show_as: extern "system" fn(Handle, i32) -> i32 = core::mem::transmute(user("ShowWindow"));
        *WIN_WINDOW.get() = w;
        for act in acts {
            match act {
                // WM_CLOSE
                1 => {
                    (win.send)(w, 0x0010, 0, 0);
                    return -1;
                }
                2 => {
                    show_as(w, 6); // SW_MINIMIZE
                }
                // The system moves the window, as from a title bar, until the button is released
                // (WM_NCLBUTTONDOWN, HTCAPTION).
                3 => {
                    let release: extern "system" fn() -> i32 = core::mem::transmute(user("ReleaseCapture"));
                    release();
                    (win.send)(w, 0x00A1, 2, 0);
                }
                4 => {
                    let is_big: extern "system" fn(Handle) -> i32 = core::mem::transmute(user("IsZoomed"));
                    show_as(w, if is_big(w) != 0 { 9 } else { 3 }); // SW_RESTORE, SW_MAXIMIZE
                }
                5 => win_full_screen(w),
                6 => {
                    let [x, y, width, height] = crate::ui_placed();
                    let place: extern "system" fn(Handle, Handle, i32, i32, i32, i32, u32) -> i32 = core::mem::transmute(user("SetWindowPos"));
                    place(w, 0, x, y, width, height, 0x14); // SWP_NOZORDER | SWP_NOACTIVATE
                }
                _ => {}
            }
        }
        // `MSG`, 48 bytes.
        let mut m = [0u64; 6];
        while *CLICKED.get() == WAITING && WIN_HEARD.get().is_empty() {
            if (win.message)(m.as_mut_ptr(), 0, 0, 0) <= 0 {
                *CLICKED.get() = -1;
            }
            (win.translate)(m.as_ptr());
            (win.dispatch)(m.as_ptr());
        }
        *SHOWN.get() = null();
        if *CLICKED.get() == WAITING {
            let (answer, text) = WIN_HEARD.get().next().unwrap_or((-1, String::new()));
            *typed = text;
            return answer;
        }
        if *CLICKED.get() <= -2 {
            *typed = TYPED.get().clone();
        }
        *CLICKED.get()
    }
}

static WIN: Global<Option<Win32>> = Global::new(None);

/// The window's style (`WS_OVERLAPPEDWINDOW`, unless `ui.run(…) frame(false)`).
static WIN_STYLE: Global<u32> = Global::new(0xCF0000);
/// The window, for `ui_areas`.
static WIN_WINDOW: Global<Handle> = Global::new(0);

/// `ui.window` then `ui.screen`: x, y, width, height.
pub fn ui_areas() -> Option<[i32; 8]> {
    let w = *WIN_WINDOW.get();
    if w == 0 {
        return None;
    }
    let (r, info) = win_measures(w);
    Some([r[0], r[1], r[2] - r[0], r[3] - r[1], info[5], info[6], info[7] - info[5], info[8] - info[6]])
}

/// The window's rectangle, and its monitor's `MONITORINFO`: the whole screen from 1, what is left
/// by the taskbar from 5.
fn win_measures(w: Handle) -> ([i32; 4], [i32; 10]) {
    let user = |name: &str| library("user32.dll").and_then(|l| symbol(l, name)).unwrap_or(0);
    let (mut r, mut info) = ([0i32; 4], [0i32; 10]);
    info[0] = 40;
    unsafe {
        use core::mem::transmute as t;
        let rect: extern "system" fn(Handle, *mut i32) -> i32 = t(user("GetWindowRect"));
        let monitor: extern "system" fn(Handle, u32) -> Handle = t(user("MonitorFromWindow"));
        let about: extern "system" fn(Handle, *mut i32) -> i32 = t(user("GetMonitorInfoW"));
        rect(w, r.as_mut_ptr());
        about(monitor(w, 2), info.as_mut_ptr()); // MONITOR_DEFAULTTONEAREST
    }
    (r, info)
}

/// The style and the place the window left for the full screen.
static WIN_BEFORE: Global<Option<(u32, [i32; 4])>> = Global::new(None);

/// `ui.full_screen`: the whole screen with no frame (WS_POPUP | WS_VISIBLE), or back.
fn win_full_screen(w: Handle) {
    let user = |name: &str| library("user32.dll").and_then(|l| symbol(l, name)).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let set_style: extern "system" fn(Handle, i32, u32) -> u32 = t(user("SetWindowLongW"));
        let get_style: extern "system" fn(Handle, i32) -> u32 = t(user("GetWindowLongW"));
        let place: extern "system" fn(Handle, Handle, i32, i32, i32, i32, u32) -> i32 = t(user("SetWindowPos"));
        let (style, r) = match WIN_BEFORE.get().take() {
            Some(before) => before,
            None => {
                let (r, info) = win_measures(w);
                *WIN_BEFORE.get() = Some((get_style(w, -16), r));
                (0x9000_0000, [info[1], info[2], info[3], info[4]])
            }
        };
        set_style(w, -16, style);
        place(w, 0, r[0], r[1], r[2] - r[0], r[3] - r[1], 0x24); // SWP_NOZORDER | SWP_FRAMECHANGED
    }
}

/// While a view is placed: the width and height taken by what comes after, for a `ui.fill` canvas.
static WIN_LEFT: Global<[i32; 2]> = Global::new([0, 0]);

/// The height of a `ui.fill` canvas starting at `y`, with `under` pixels taken below it.
fn win_filled(window: Handle, y: i32, under: i32) -> i32 {
    let mut now = [0; 4];
    if let Some(win) = WIN.get().as_ref() {
        (win.client)(window, &mut now);
    }
    (now[3] - y - win_edge() - under).max(1)
}

/// The empty pixels around the content: none in a window that has given up its frame.
fn win_edge() -> i32 {
    if *WIN_STYLE.get() == 0xCF0000 { 16 } else { 0 }
}

/// What a canvas or a clock heard, for `ui_show`.
static WIN_HEARD: Global<crate::shared::UiQueue> = Global::new(crate::shared::UiQueue::new());
/// The running clocks (`SetTimer`).
static WIN_CLOCKS: Global<Vec<usize>> = Global::new(Vec::new());
/// What those clocks are (`shared::ui_clocks`).
static WIN_CLOCKS_ASKED: Global<Vec<(i32, usize)>> = Global::new(Vec::new());
/// Each canvas now on the screen.
static WIN_CANVASES: Global<Vec<(Handle, crate::shared::UiCanvas)>> = Global::new(Vec::new());

/// `ui.Key`'s name for a key that writes nothing: the same word on every system.
fn win_key(key: usize) -> Option<String> {
    let named = match key {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x1B => "Escape",
        0x20 => "Space",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2E => "Delete",
        0x70..=0x87 => return Some(alloc::format!("F{}", key - 0x6F)),
        _ => return None,
    };
    Some(named.into())
}

/// Paints a `ui.gpu`, with an OpenGL context of its own window. WGL's plain context is enough:
/// every driver gives it the most recent version it has.
fn win_gpu(h: Handle, dc: Handle, canvas: &crate::shared::UiCanvas) {
    static CONTEXTS: Global<Vec<(Handle, Handle)>> = Global::new(Vec::new());
    let Some(gl) = library("opengl32.dll") else { crate::gl_stop(crate::GPU_MISSING, b"") };
    let wgl = |name: &str| symbol(gl, name).unwrap_or(0);
    let gdi = |name: &str| library("gdi32.dll").and_then(|g| symbol(g, name)).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let context = match CONTEXTS.get().iter().find(|c| c.0 == h) {
            Some(found) => found.1,
            None => {
                let choose: extern "system" fn(Handle, *const u8) -> i32 = t(gdi("ChoosePixelFormat"));
                let set: extern "system" fn(Handle, i32, *const u8) -> i32 = t(gdi("SetPixelFormat"));
                let create: extern "system" fn(Handle) -> Handle = t(wgl("wglCreateContext"));
                // `PIXELFORMATDESCRIPTOR`: size, version 1, on a window + OpenGL + double buffer, 32-bit
                // colors, 24-bit depth.
                let mut format = [0u8; 40];
                format[0] = 40;
                format[2] = 1;
                format[4] = 0x25;
                format[9] = 32;
                format[23] = 24;
                let chosen = choose(dc, format.as_ptr());
                let made = if chosen == 0 || set(dc, chosen, format.as_ptr()) == 0 { 0 as Handle } else { create(dc) };
                if made == 0 as Handle {
                    crate::gl_stop(crate::GPU_MISSING, b"");
                }
                CONTEXTS.get().push((h, made));
                made
            }
        };
        let current: extern "system" fn(Handle, Handle) -> i32 = t(wgl("wglMakeCurrent"));
        let address: extern "system" fn(*const u8) -> usize = t(wgl("wglGetProcAddress"));
        let swap: extern "system" fn(Handle) -> i32 = t(gdi("SwapBuffers"));
        current(dc, context);
        // Past OpenGL 1.1, only the context knows; before, only the DLL (some drivers give 1, 2 or 3).
        let load = |name: &str| match address(alloc::format!("{name}\0").as_ptr()) {
            0..=3 | usize::MAX => symbol(gl, name).unwrap_or(0),
            found => found,
        };
        crate::gl_paint(h as usize, &canvas.ops, canvas.width, canvas.height, &load);
        swap(dc);
    }
}

/// The window of a `ui.canvas`.
extern "system" fn win_canvas(h: Handle, message: u32, w: usize, l: isize) -> isize {
    let user = |name: &str| library("user32.dll").and_then(|u| symbol(u, name)).unwrap_or(0);
    let default: extern "system" fn(Handle, u32, usize, isize) -> isize = unsafe { core::mem::transmute(user("DefWindowProcW")) };
    let Some((_, canvas)) = WIN_CANVASES.get().iter().find(|(c, _)| *c == h) else {
        return default(h, message, w, l);
    };
    let (x, y) = ((l & 0xffff) as i16 as i32, (l >> 16 & 0xffff) as i16 as i32);
    let mouse = |kind: &str, button: i32| {
        if let Some(n) = canvas.mouse {
            WIN_HEARD.get().hear(crate::shared::ui_changed(n), crate::shared::ui_mouse(kind, x, y, button));
        }
        0
    };
    let key = |name: String| {
        if let Some(n) = canvas.key {
            WIN_HEARD.get().hear(crate::shared::ui_changed(n), crate::shared::ui_key(&name));
        }
        0
    };
    unsafe {
        use core::mem::transmute as t;
        let capture: extern "system" fn(Handle) -> Handle = t(user("SetCapture"));
        let release: extern "system" fn() -> i32 = t(user("ReleaseCapture"));
        let focus: extern "system" fn(Handle) -> Handle = t(user("SetFocus"));
        match message {
            // WM_ERASEBKGND: nothing to erase, or it would flicker.
            0x0014 => 1,
            // WM_PAINT, through a bitmap for the same reason.
            0x000F => {
                let gdi = |name: &str| library("gdi32.dll").and_then(|g| symbol(g, name)).unwrap_or(0);
                let begin: extern "system" fn(Handle, *mut u64) -> Handle = t(user("BeginPaint"));
                let end: extern "system" fn(Handle, *const u64) -> i32 = t(user("EndPaint"));
                let same_dc: extern "system" fn(Handle) -> Handle = t(gdi("CreateCompatibleDC"));
                let bitmap_for: extern "system" fn(Handle, i32, i32) -> Handle = t(gdi("CreateCompatibleBitmap"));
                let select: extern "system" fn(Handle, Handle) -> Handle = t(gdi("SelectObject"));
                let copy: extern "system" fn(Handle, i32, i32, i32, i32, Handle, i32, i32, u32) -> i32 = t(gdi("BitBlt"));
                let delete: extern "system" fn(Handle) -> i32 = t(gdi("DeleteObject"));
                let delete_dc: extern "system" fn(Handle) -> i32 = t(gdi("DeleteDC"));
                // `PAINTSTRUCT`, 72 bytes.
                let mut paint = [0u64; 9];
                let dc = begin(h, paint.as_mut_ptr());
                if canvas.gpu() {
                    win_gpu(h, dc, canvas);
                    end(h, paint.as_ptr());
                    return 0;
                }
                let memory = same_dc(dc);
                let bitmap = bitmap_for(dc, canvas.width, canvas.height);
                let before = select(memory, bitmap);
                win_draw(memory, canvas);
                copy(dc, 0, 0, canvas.width, canvas.height, memory, 0, 0, 0xCC0020); // SRCCOPY
                select(memory, before);
                delete(bitmap);
                delete_dc(memory);
                end(h, paint.as_ptr());
                0
            }
            // WM_PRINTCLIENT: into a DC given by someone else.
            0x0318 => {
                win_draw(w as Handle, canvas);
                0
            }
            // WM_LBUTTONDOWN, WM_RBUTTONDOWN: what is pressed here stays here until released.
            0x0201 | 0x0204 => {
                focus(h);
                capture(h);
                mouse("press", if message == 0x0204 { 2 } else { 1 })
            }
            0x0202 | 0x0205 => {
                release();
                mouse("release", if message == 0x0205 { 2 } else { 1 })
            }
            0x0200 => mouse("move", 0),
            // WM_KEYDOWN: what writes nothing. WM_CHAR: the rest, as the keyboard's layout says.
            0x0100 => match win_key(w) {
                Some(name) => key(name),
                None => 0,
            },
            0x0102 if w > 0x20 && w != 0x7f => match char::from_u32(w as u32) {
                Some(c) => key(c.into()),
                None => 0,
            },
            // WM_GETDLGCODE: DLGC_WANTALLKEYS, or Tab and the arrows would go elsewhere.
            0x0087 => 4,
            _ => default(h, message, w, l),
        }
    }
}

/// `GdiplusStartup`, once: 0 without GDI+.
fn gdiplus() -> usize {
    static GDIPLUS: Global<usize> = Global::new(0);
    if *GDIPLUS.get() == 0 {
        let start = || unsafe {
            let plus = library("gdiplus.dll")?;
            // `GdiplusStartupInput`: version 1, then nothing asked.
            let (mut token, input) = (0usize, [1usize, 0, 0]);
            let start: extern "system" fn(*mut usize, *const usize, *mut u8) -> i32 = core::mem::transmute(symbol(plus, "GdiplusStartup")?);
            (start(&mut token, input.as_ptr(), core::ptr::null_mut()) == 0).then_some(plus)
        };
        *GDIPLUS.get() = start().unwrap_or(0);
    }
    *GDIPLUS.get()
}

/// Paints a canvas on `dc`: shapes through GDI+, which smooths them; texts through GDI, with the
/// fonts of the rest of the window. Thickness stays inside a shape, as on every system.
fn win_draw(dc: Handle, canvas: &crate::shared::UiCanvas) {
    use crate::shared::UiOp::*;
    let plus = gdiplus();
    if plus == 0 {
        return;
    }
    let f = |name: &str| symbol(plus, name).unwrap_or(0);
    let gdi = |name: &str| library("gdi32.dll").and_then(|g| symbol(g, name)).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let graphics_of: extern "system" fn(Handle, *mut Handle) -> i32 = t(f("GdipCreateFromHDC"));
        let smooth: extern "system" fn(Handle, i32) -> i32 = t(f("GdipSetSmoothingMode"));
        let finish: extern "system" fn(Handle) -> i32 = t(f("GdipDeleteGraphics"));
        let new_brush: extern "system" fn(u32, *mut Handle) -> i32 = t(f("GdipCreateSolidFill"));
        let free_brush: extern "system" fn(Handle) -> i32 = t(f("GdipDeleteBrush"));
        let new_pen: extern "system" fn(u32, f32, i32, *mut Handle) -> i32 = t(f("GdipCreatePen1"));
        let free_pen: extern "system" fn(Handle) -> i32 = t(f("GdipDeletePen"));
        let fill_rect: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32 = t(f("GdipFillRectangleI"));
        let fill_oval: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32 = t(f("GdipFillEllipseI"));
        let draw_rect: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32 = t(f("GdipDrawRectangleI"));
        let draw_oval: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32 = t(f("GdipDrawEllipseI"));
        let draw_line: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32 = t(f("GdipDrawLineI"));
        let select: extern "system" fn(Handle, Handle) -> Handle = t(gdi("SelectObject"));
        let background: extern "system" fn(Handle, i32) -> i32 = t(gdi("SetBkMode"));
        let text_color: extern "system" fn(Handle, u32) -> u32 = t(gdi("SetTextColor"));
        let text_out: extern "system" fn(Handle, i32, i32, *const u16, i32) -> i32 = t(gdi("TextOutW"));
        let opaque = |c: u32| 0xff00_0000 | c;
        let filled = |g: Handle, c: u32, shape: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32, box_: [i32; 4]| {
            let mut brush = 0;
            new_brush(opaque(c), &mut brush);
            shape(g, brush, box_[0], box_[1], box_[2], box_[3]);
            free_brush(brush);
        };
        let lined = |g: Handle, c: u32, n: i32, shape: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32, box_: [i32; 4]| {
            let mut pen = 0;
            new_pen(opaque(c), n as f32, 2, &mut pen); // UnitPixel
            shape(g, pen, box_[0], box_[1], box_[2], box_[3]);
            free_pen(pen);
        };
        // Closed around each text: GDI and GDI+ take turns on the DC.
        let mut g = 0;
        let open = |g: &mut Handle| {
            if *g == 0 {
                graphics_of(dc, g);
                smooth(*g, 4); // SmoothingModeAntiAlias8x4
            }
        };
        open(&mut g);
        filled(g, 0xffffff, fill_rect, [0, 0, canvas.width, canvas.height]);
        let image_of: extern "system" fn(*const u16, *mut Handle) -> i32 = t(f("GdipCreateBitmapFromFile"));
        let measure: [extern "system" fn(Handle, *mut u32) -> i32; 2] = [t(f("GdipGetImageWidth")), t(f("GdipGetImageHeight"))];
        let draw_image: extern "system" fn(Handle, Handle, i32, i32, i32, i32) -> i32 = t(f("GdipDrawImageRectI"));
        for op in &canvas.ops {
            if !matches!(op, Text { .. }) {
                open(&mut g);
            }
            match op {
                Gpu | GlClear(_) | GlShader(..) | GlUniform(..) | GlTriangles(..) | GlTexture(..) => {}
                Clear(c) => filled(g, *c, fill_rect, [0, 0, canvas.width, canvas.height]),
                Rect { x, y, w, h, color, outline: 0 } => filled(g, *color, fill_rect, [*x, *y, *w, *h]),
                Rect { x, y, w, h, color, outline } => lined(g, *color, *outline, draw_rect, [x + outline / 2, y + outline / 2, w - outline, h - outline]),
                Circle { x, y, r, color, outline: 0 } => filled(g, *color, fill_oval, [x - r, y - r, 2 * r, 2 * r]),
                Circle { x, y, r, color, outline } => lined(g, *color, *outline, draw_oval, [x - r + outline / 2, y - r + outline / 2, 2 * r - outline, 2 * r - outline]),
                Line { x, y, x2, y2, color, thickness } => lined(g, *color, *thickness, draw_line, [*x, *y, *x2, *y2]),
                Image { x, y, w, h, path } => {
                    static IMAGES: Global<Vec<(String, Handle)>> = Global::new(Vec::new());
                    if !IMAGES.get().iter().any(|i| i.0 == *path) {
                        let mut image = 0;
                        image_of(wide(path).as_ptr(), &mut image);
                        IMAGES.get().push((path.clone(), image));
                    }
                    let image = IMAGES.get().iter().find(|i| i.0 == *path).map_or(0, |i| i.1);
                    if image != 0 {
                        let (mut own_w, mut own_h) = (0, 0);
                        measure[0](image, &mut own_w);
                        measure[1](image, &mut own_h);
                        draw_image(g, image, *x, *y, if *w > 0 { *w } else { own_w as i32 }, if *h > 0 { *h } else { own_h as i32 });
                    }
                }
                Text { x, y, size, color, text } => {
                    if g != 0 {
                        finish(g);
                        g = 0;
                    }
                    let Some(win) = WIN.get() else { continue };
                    let before = select(dc, win_font(win, crate::shared::UiStyle { size: *size, ..Default::default() }));
                    background(dc, 1); // TRANSPARENT
                    text_color(dc, (color & 0xff) << 16 | (color & 0xff00) | color >> 16);
                    let letters = wide(text);
                    text_out(dc, *x, *y, letters.as_ptr(), letters.len() as i32 - 1);
                    select(dc, before);
                }
            }
        }
        if g != 0 {
            finish(g);
        }
    }
}

/// While controls are made or given their texts: their EN_CHANGE is not the user typing.
static SETTING: Global<bool> = Global::new(false);

/// The controls with a color of their own (a `COLORREF`, 0xBBGGRR), for WM_CTLCOLOR….
static COLORED: Global<Vec<(Handle, u32)>> = Global::new(Vec::new());

/// The font of a style, made once.
fn win_font(win: &Win32, style: crate::shared::UiStyle) -> Handle {
    static FONTS: Global<Vec<(i32, bool, Handle)>> = Global::new(Vec::new());
    if style.size == 0 && !style.bold {
        return win.font;
    }
    if let Some(&(.., font)) = FONTS.get().iter().find(|f| (f.0, f.1) == (style.size, style.bold)) {
        return font;
    }
    // `LOGFONTW`, 92 bytes: the height first (negative: in pixels), the weight at byte 16.
    let mut log = [0i32; 23];
    (win.object)(win.font, 92, log.as_mut_ptr());
    if style.size > 0 {
        log[0] = -style.size;
    }
    if style.bold {
        log[4] = 700;
    }
    let font = (win.new_font)(log.as_ptr());
    FONTS.get().push((style.size, style.bold, font));
    font
}

/// An image as a bitmap, with its size: read by GDI+, the Windows part that knows PNG and JPEG, then
/// kept. Transparency is mixed with the window's color. None if it cannot be read.
fn win_image(path: &str, width: i32) -> Option<(Handle, i32, i32)> {
    static IMAGES: Global<Vec<(String, i32, Option<(Handle, i32, i32)>)>> = Global::new(Vec::new());
    if let Some(known) = IMAGES.get().iter().find(|i| i.0 == path && i.1 == width) {
        return known.2;
    }
    let read = || unsafe {
        use core::mem::transmute as t;
        let plus = gdiplus();
        if plus == 0 {
            return None;
        }
        let f = |name: &str| symbol(plus, name);
        let from_file: extern "system" fn(*const u16, *mut Handle) -> i32 = t(f("GdipCreateBitmapFromFile")?);
        let measure: [extern "system" fn(Handle, *mut u32) -> i32; 2] = [t(f("GdipGetImageWidth")?), t(f("GdipGetImageHeight")?)];
        let scaled: extern "system" fn(Handle, u32, u32, *mut Handle, usize, usize) -> i32 = t(f("GdipGetImageThumbnail")?);
        let bitmap: extern "system" fn(Handle, *mut Handle, u32) -> i32 = t(f("GdipCreateHBITMAPFromBitmap")?);
        let color: extern "system" fn(i32) -> u32 = t(symbol(library("user32.dll")?, "GetSysColor")?);
        let (mut image, mut w, mut h) = (0 as Handle, 0, 0);
        if from_file(wide(path).as_ptr(), &mut image) != 0 {
            return None;
        }
        measure[0](image, &mut w);
        measure[1](image, &mut h);
        if width > 0 && w > 0 {
            h = (h as u64 * width as u64 / w as u64).max(1) as u32;
            w = width as u32;
            let mut smaller = 0 as Handle;
            if scaled(image, w, h, &mut smaller, 0, 0) != 0 {
                return None;
            }
            image = smaller;
        }
        // COLOR_BTNFACE, 0xBBGGRR, to opaque 0xAARRGGBB.
        let c = color(15);
        let (mut made, around) = (0 as Handle, 0xff00_0000 | (c & 0xff) << 16 | (c & 0xff00) | (c >> 16 & 0xff));
        (bitmap(image, &mut made, around) == 0).then_some((made, w as i32, h as i32))
    };
    let image = read();
    IMAGES.get().push((path.into(), width, image));
    image
}

/// A field's id is `FIELD` + its number (a button's, its number + 1).
const FIELD: usize = 0x8000;

/// A control's text.
fn win_text(win: &Win32, control: Handle) -> String {
    let n = (win.text_length)(control);
    let mut buffer = alloc::vec![0u16; n.max(0) as usize + 1];
    let got = (win.get_text)(control, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..got.max(0) as usize])
}

/// `WNDCLASSEXW`.
#[repr(C)]
struct WindowClass {
    size: u32,
    style: u32,
    procedure: usize,
    extra: [i32; 2],
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu: *const u16,
    class: *const u16,
    small_icon: Handle,
}

/// The Win32 functions `ui_show` calls, and the font of every control.
struct Win32 {
    create: extern "system" fn(u32, *const u16, *const u16, u32, i32, i32, i32, i32, Handle, Handle, Handle, *mut u8) -> Handle,
    destroy: extern "system" fn(Handle) -> i32,
    set_text: extern "system" fn(Handle, *const u16) -> i32,
    get_text: extern "system" fn(Handle, *mut u16, i32) -> i32,
    text_length: extern "system" fn(Handle) -> i32,
    control_id: extern "system" fn(Handle) -> i32,
    send: extern "system" fn(Handle, u32, usize, isize) -> isize,
    post: extern "system" fn(Handle, u32, usize, isize) -> i32,
    default: extern "system" fn(Handle, u32, usize, isize) -> isize,
    get_dc: extern "system" fn(Handle) -> Handle,
    release_dc: extern "system" fn(Handle, Handle) -> i32,
    font: Handle,
    select: extern "system" fn(Handle, Handle) -> Handle,
    extent: extern "system" fn(Handle, *const u16, i32, *mut [i32; 2]) -> i32,
    object: extern "system" fn(Handle, i32, *mut i32) -> i32,
    new_font: extern "system" fn(*const i32) -> Handle,
    text_color: extern "system" fn(Handle, u32) -> u32,
    invalidate: extern "system" fn(Handle, *const [i32; 4], i32) -> i32,
    adjust: extern "system" fn(*mut [i32; 4], u32, i32) -> i32,
    client: extern "system" fn(Handle, *mut [i32; 4]) -> i32,
    set_pos: extern "system" fn(Handle, Handle, i32, i32, i32, i32, u32) -> i32,
    message: extern "system" fn(*mut u64, Handle, u32, u32) -> i32,
    translate: extern "system" fn(*const u64) -> i32,
    dispatch: extern "system" fn(*const u64) -> isize,
}

/// Pixels between the parts of a column or a row.
const GAP: i32 = 8;

/// Sliders and lists are in `comctl32.dll`, which registers them when it is loaded.
fn win_common() {
    static DONE: Global<bool> = Global::new(false);
    if !core::mem::replace(DONE.get(), true) {
        if let Some(f) = library("comctl32.dll").and_then(|l| symbol(l, "InitCommonControls")) {
            let start: extern "system" fn() = unsafe { core::mem::transmute(f) };
            start();
        }
    }
}

/// The window's menu bar. An item's id is its event's number + 1, as a button's, so its WM_COMMAND
/// is a click.
fn win_menus(window: Handle, menus: &[crate::shared::UiMenu]) {
    let Some(user) = library("user32.dll") else { return };
    let f = |name: &str| symbol(user, name).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let new_bar: extern "system" fn() -> Handle = t(f("CreateMenu"));
        let new_popup: extern "system" fn() -> Handle = t(f("CreatePopupMenu"));
        let append: extern "system" fn(Handle, u32, usize, *const u16) -> i32 = t(f("AppendMenuW"));
        let set: extern "system" fn(Handle, Handle) -> i32 = t(f("SetMenu"));
        let draw: extern "system" fn(Handle) -> i32 = t(f("DrawMenuBar"));
        let bar = new_bar();
        for m in menus {
            let popup = new_popup();
            for (label, event) in &m.items {
                match event {
                    Some(n) => append(popup, 0, n + 1, wide(label).as_ptr()),
                    None => append(popup, 0x800, 0, null()), // MF_SEPARATOR
                };
            }
            append(bar, 0x10, popup as usize, wide(&m.title).as_ptr()); // MF_POPUP
        }
        set(window, if menus.is_empty() { 0 } else { bar });
        draw(window);
    }
}

/// `sound.play`: SND_MEMORY, which waits for the end.
pub fn sound(whole: &[u8], _wav: &crate::shared::Wav) -> bool {
    let Some(play) = library("winmm.dll").and_then(|l| symbol(l, "PlaySoundW")) else { return false };
    let play: extern "system" fn(*const u8, usize, u32) -> i32 = unsafe { core::mem::transmute(play) };
    play(whole.as_ptr(), 0, 4) != 0
}

/// `ui.dialog`: the system's own, which wait for the answer.
pub fn ui_dialog(kind: i64, title: &str, text: &str) -> Option<String> {
    use core::mem::transmute as t;
    let owner = *WIN_WINDOW.get();
    let call = |dll: &str, name: &str| library(dll).and_then(|l| symbol(l, name));
    unsafe {
        if kind >= 3 {
            let boxed: extern "system" fn(Handle, *const u16, *const u16, u32) -> i32 = t(call("user32.dll", "MessageBoxW")?);
            // MB_OK | MB_ICONINFORMATION, MB_YESNO | MB_ICONQUESTION; IDOK = 1, IDYES = 6.
            let (style, yes) = if kind == 3 { (0x40, 1) } else { (0x24, 6) };
            return (boxed(owner, wide(text).as_ptr(), wide(title).as_ptr(), style) == yes).then(|| "yes".into());
        }
        let title = wide(title);
        let mut path = alloc::vec![0u16; 4096];
        if kind == 2 {
            let browse: extern "system" fn(*const [u64; 8]) -> usize = t(call("shell32.dll", "SHBrowseForFolderW")?);
            let path_of: extern "system" fn(usize, *mut u16) -> i32 = t(call("shell32.dll", "SHGetPathFromIDListW")?);
            let info: [u64; 8] = [owner as u64, 0, path.as_mut_ptr() as u64, title.as_ptr() as u64, 0x41, 0, 0, 0];
            let chosen = browse(&info);
            if chosen == 0 || path_of(chosen, path.as_mut_ptr()) == 0 {
                return None;
            }
        } else {
            let name = wide(text);
            path[..name.len()].copy_from_slice(&name);
            // `OPENFILENAMEW`, by the place of each field. OFN_NOCHANGEDIR, then
            // OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST or OFN_OVERWRITEPROMPT.
            let mut info = [0u64; 19];
            info[0] = 152;
            info[1] = owner as u64;
            info[6] = path.as_mut_ptr() as u64;
            info[7] = path.len() as u64;
            info[11] = title.as_ptr() as u64;
            info[12] = 8 | if kind == 0 { 0x1800 } else { 2 };
            let ask: extern "system" fn(*mut [u64; 19]) -> i32 = t(call("comdlg32.dll", if kind == 0 { "GetOpenFileNameW" } else { "GetSaveFileNameW" })?);
            if ask(&mut info) == 0 {
                return None;
            }
        }
        let n = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        Some(String::from_utf16_lossy(&path[..n]))
    }
}

/// `LVCOLUMNW`.
#[repr(C)]
#[derive(Default)]
struct ListColumn {
    mask: u32,
    format: i32,
    width: i32,
    text: usize,
    room: i32,
    sub_item: i32,
    image: i32,
    order: i32,
    least: i32,
    default: i32,
    ideal: i32,
}

/// `LVITEMW`.
#[repr(C)]
#[derive(Default)]
struct ListItem {
    mask: u32,
    item: i32,
    sub_item: i32,
    state: u32,
    state_mask: u32,
    text: usize,
    room: i32,
    image: i32,
    param: isize,
    indent: i32,
    group: i32,
    columns: u32,
    column_list: usize,
    formats: usize,
    group_again: i32,
}

/// Fills a `SysListView32`: LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETITEMW.
fn win_table(win: &Win32, list: Handle, table: &crate::shared::UiTable, width: i32) {
    let columns = table.headers.len().max(table.rows.first().map_or(1, |r| r.len())).max(1);
    for k in 0..columns {
        let name = wide(table.headers.get(k).map_or("", |h| h));
        let column = ListColumn { mask: 2 | 4, width: (width - 24) / columns as i32, text: name.as_ptr() as usize, ..Default::default() };
        (win.send)(list, 0x1061, k, &column as *const ListColumn as isize);
    }
    for (line, cells) in table.rows.iter().enumerate() {
        for (k, cell) in cells.iter().enumerate() {
            let text = wide(cell);
            let item = ListItem { mask: 1, item: line as i32, sub_item: k as i32, text: text.as_ptr() as usize, ..Default::default() };
            (win.send)(list, if k == 0 { 0x104D } else { 0x104C }, 0, &item as *const ListItem as isize);
        }
    }
}

/// Selects line `k` of a `SysListView32` (-1: none), if it is not: LVM_GETNEXTITEM, LVM_SETITEMSTATE.
fn win_select(win: &Win32, list: Handle, k: i32) {
    let now = (win.send)(list, 0x100C, usize::MAX, 2);
    if now != k as isize {
        for (line, state) in [(now, 0), (k as isize, 2)] {
            if line >= 0 {
                let item = ListItem { state, state_mask: 2, ..Default::default() };
                (win.send)(list, 0x102B, line as usize, &item as *const ListItem as isize);
            }
        }
    }
}

/// The (width, height) of a view's controls.
/// ponytail: one line per text, no wrapping; DPI scaling left to Windows.
fn win_size(win: &Win32, dc: Handle, node: &crate::shared::UiNode) -> (i32, i32) {
    use crate::shared::UiNode::*;
    let text = |s: &str| {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut size = [0i32; 2];
        (win.extent)(dc, w.as_ptr(), w.len() as i32, &mut size);
        (size[0], size[1])
    };
    match node {
        Text(s) => text(s),
        Button(s, _) => {
            let (w, h) = text(s);
            (w + 24, h + 12)
        }
        // Room for about 24 characters, more if its text is longer.
        Input(s, _) => {
            let (w, h) = text(s);
            let (wide, _) = text("abcdefghijklmnopqrstuvwx");
            (w.max(wide) + 12, h + 10)
        }
        Check(s, ..) => {
            let (w, h) = text(s);
            (w + 24, h + 6)
        }
        Slider(..) => (200, 30),
        Choice(items, ..) => {
            let (w, h) = items.iter().map(|i| text(i)).fold((0, 0), |a, b| (a.0.max(b.0), a.1.max(b.1)));
            (w.max(60) + 40, h + 12)
        }
        Table(_) => (320, 200),
        Column(parts, _) => parts.iter().map(|p| win_size(win, dc, p)).fold((0, -GAP), |(w, h), (pw, ph)| (w.max(pw), h + GAP + ph)),
        Row(parts, _) => parts.iter().map(|p| win_size(win, dc, p)).fold((-GAP, 0), |(w, h), (pw, ph)| (w + GAP + pw, h.max(ph))),
        Image(path, width) => win_image(path, *width).map_or((0, 0), |(_, w, h)| (w, h)),
        Canvas(c) => (c.width, c.height),
        Every(_, _, inner) => win_size(win, dc, inner),
        Styled(style, leaf) => {
            let before = (win.select)(dc, win_font(win, *style));
            let size = win_size(win, dc, leaf);
            (win.select)(dc, before);
            size
        }
        Spaced(m, inner) => {
            let (w, h) = win_size(win, dc, inner);
            (w + 2 * m, h + 2 * m)
        }
    }
}

/// Makes a view's controls in `window`, from (x, y), with `room` pixels of width. With `same`, the controls are already
/// there (in `made`, the `at`-th next): they only move and take the texts that changed.
#[allow(clippy::too_many_arguments)]
fn win_place(win: &Win32, dc: Handle, window: Handle, node: &crate::shared::UiNode, style: crate::shared::UiStyle, x: i32, y: i32, room: i32, made: &mut Vec<Handle>, same: bool, at: &mut usize) {
    use crate::shared::UiNode::*;
    let (w, h) = win_size(win, dc, node);
    let mut control = |class: &str, s: &str, id: usize, flags: u32| {
        let c = if same {
            let c = made[*at];
            (win.set_pos)(c, 0, x, y, w, h, 0x4 | 0x10); // SWP_NOZORDER | SWP_NOACTIVATE
            if matches!(class, "BUTTON" | "EDIT" | "STATIC") && win_text(win, c) != s {
                (win.set_text)(c, wide(s).as_ptr());
            }
            c
        } else {
            // WS_CHILD | WS_VISIBLE
            let c = (win.create)(0, wide(class).as_ptr(), wide(s).as_ptr(), 0x5000_0000 | flags, x, y, w, h, window, id as Handle, 0, null_mut());
            made.push(c);
            c
        };
        *at += 1;
        let font = win_font(win, style);
        if (win.send)(c, 0x0031, 0, 0) != font as isize { // WM_GETFONT
            (win.send)(c, 0x0030, font as usize, 1); // WM_SETFONT
        }
        // 0xRRGGBB to a `COLORREF`, 0xBBGGRR.
        let color = style.color.map(|c| (c & 0xff) << 16 | (c & 0xff00) | c >> 16);
        let colored = COLORED.get();
        let before = colored.iter().position(|e| e.0 == c);
        if before.map(|k| colored[k].1) != color {
            if let Some(k) = before {
                colored.remove(k);
            }
            if let Some(color) = color {
                colored.push((c, color));
            }
            (win.invalidate)(c, null(), 1);
        }
    };
    match node {
        Text(s) => control("STATIC", s, 0, 0),
        // BS_AUTOCHECKBOX, WS_TABSTOP; BM_GETCHECK, BM_SETCHECK.
        Check(s, on, n) => {
            control("BUTTON", s, FIELD + n, 3 | 0x0001_0000);
            let c = made[*at - 1];
            if ((win.send)(c, 0x00F0, 0, 0) == 1) != *on {
                (win.send)(c, 0x00F1, *on as usize, 0);
            }
        }
        // TBM_SETRANGEMIN, TBM_SETRANGEMAX, TBM_GETPOS, TBM_SETPOS.
        Slider(v, min, max, n) => {
            win_common();
            control("msctls_trackbar32", "", FIELD + n, 0x0001_0000);
            let c = made[*at - 1];
            if !same {
                (win.send)(c, 0x0407, 0, *min as isize);
                (win.send)(c, 0x0408, 1, *max as isize);
            }
            if (win.send)(c, 0x0400, 0, 0) != *v as isize {
                (win.send)(c, 0x0405, 1, *v as isize);
            }
        }
        // CBS_DROPDOWNLIST, WS_VSCROLL, WS_TABSTOP. Its height is the open list's too.
        Choice(items, k, n) => {
            control("COMBOBOX", "", FIELD + n, 3 | 0x0020_0000 | 0x0001_0000);
            let c = made[*at - 1];
            (win.set_pos)(c, 0, x, y, w, h + 160, 0x4 | 0x10);
            if !same {
                items.iter().for_each(|i| {
                    (win.send)(c, 0x0143, 0, wide(i).as_ptr() as isize); // CB_ADDSTRING
                });
            }
            if (win.send)(c, 0x0147, 0, 0) != *k as isize {
                (win.send)(c, 0x014E, *k as usize, 0); // CB_SETCURSEL
            }
        }
        // LVS_REPORT, LVS_SINGLESEL, LVS_SHOWSELALWAYS, LVS_NOCOLUMNHEADER, WS_BORDER, WS_TABSTOP.
        Table(table) => {
            win_common();
            let bare = if table.headers.is_empty() { 0x4000 } else { 0 };
            control("SysListView32", "", FIELD + table.number, 1 | 4 | 8 | bare | 0x0080_0000 | 0x0001_0000);
            let c = made[*at - 1];
            if !same {
                win_table(win, c, table, w);
            }
            win_select(win, c, table.selected);
        }
        Button(s, n) => control("BUTTON", s, n + 1, 0),
        // WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL
        Input(s, n) => control("EDIT", s, FIELD + n, 0x0080_0000 | 0x0001_0000 | 0x80),
        Column(parts, a) => {
            let room = if crate::shared::ui_fills(node) { room.max(w) } else { w };
            let mut y = y;
            for (k, p) in parts.iter().enumerate() {
                let (pw, ph) = win_size(win, dc, p);
                let px = if crate::shared::ui_fills(p) { x } else { x + a.offset(room - pw) };
                // A canvas of `ui.fill` height leaves room for what comes under it.
                let under: i32 = parts[k + 1..].iter().map(|q| win_size(win, dc, q).1 + GAP).sum();
                WIN_LEFT.get()[1] += under;
                win_place(win, dc, window, p, Default::default(), px, y, room, made, same, at);
                WIN_LEFT.get()[1] -= under;
                y += if crate::shared::ui_fills_down(p) { win_filled(window, y, under) } else { ph } + GAP;
            }
        }
        Row(parts, a) => {
            let mut x = x;
            for (k, p) in parts.iter().enumerate() {
                let (pw, ph) = win_size(win, dc, p);
                let after: i32 = parts[k + 1..].iter().map(|q| win_size(win, dc, q).0 + GAP).sum();
                WIN_LEFT.get()[0] += after;
                win_place(win, dc, window, p, Default::default(), x, y + a.offset(h - ph), pw, made, same, at);
                WIN_LEFT.get()[0] -= after;
                x += pw + GAP;
            }
        }
        Image(path, width) => {
            control("STATIC", "", 0, 0xE); // SS_BITMAP
            if let (false, Some((bitmap, ..))) = (same, win_image(path, *width)) {
                (win.send)(made[*at - 1], 0x0172, 0, bitmap as isize); // STM_SETIMAGE, IMAGE_BITMAP
            }
        }
        Styled(style, leaf) => {
            let before = (win.select)(dc, win_font(win, *style));
            win_place(win, dc, window, leaf, *style, x, y, room, made, same, at);
            (win.select)(dc, before);
        }
        Spaced(m, inner) => win_place(win, dc, window, inner, style, x + m, y + m, room - 2 * m, made, same, at),
        Every(_, _, inner) => win_place(win, dc, window, inner, style, x, y, room, made, same, at),
        Canvas(c) => {
            control("KovaCanvas", "", 0, 0);
            let (handle, canvases) = (made[*at - 1], WIN_CANVASES.get());
            let mut sized = c.clone();
            if c.width == 0 || c.height == 0 {
                let mut now = [0; 4];
                (win.client)(window, &mut now);
                if c.width == 0 {
                    sized.width = (now[2] - x - win_edge() - WIN_LEFT.get()[0]).max(1);
                }
                if c.height == 0 {
                    sized.height = win_filled(window, y, WIN_LEFT.get()[1]);
                }
                (win.set_pos)(handle, 0, x, y, sized.width, sized.height, 0x4 | 0x10);
                if crate::ui_room(c.number, sized.width, sized.height) {
                    WIN_HEARD.get().hear(crate::shared::UI_RESIZED, String::new());
                }
            }
            match canvases.iter_mut().find(|e| e.0 == handle) {
                Some(there) => there.1 = sized,
                None => canvases.push((handle, sized)),
            }
            (win.invalidate)(handle, null(), 0);
        }
    }
}
