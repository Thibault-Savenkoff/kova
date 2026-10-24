// What the runtime asks of macOS: `libSystem`, which every Mac has (Apple wants programs to go
// through it, not to call the system directly). Its `main` is the program's entry point.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};

use crate::c_text;
use crate::http::{Fail, Network, Serve};

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
        pub fn getaddrinfo(host: *const u8, port: *const u8, hints: *const super::AddrInfo, found: *mut *mut super::AddrInfo) -> i32;
        pub fn freeaddrinfo(found: *mut super::AddrInfo);
        pub fn socket(family: i32, kind: i32, protocol: i32) -> i32;
        pub fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, n: u32) -> i32;
        pub fn connect(fd: i32, address: *const u8, n: u32) -> i32;
        pub fn send(fd: i32, b: *const u8, n: usize, flags: i32) -> isize;
        pub fn recv(fd: i32, b: *mut u8, n: usize, flags: i32) -> isize;
        pub fn bind(fd: i32, address: *const u8, n: u32) -> i32;
        pub fn listen(fd: i32, backlog: i32) -> i32;
        pub fn accept(fd: i32, address: *mut u8, n: *mut u32) -> i32;
        pub fn poll(fds: *mut [i32; 2], n: u32, ms: i32) -> i32;
        pub fn getifaddrs(list: *mut *mut super::IfAddrs) -> i32;
        pub fn freeifaddrs(list: *mut super::IfAddrs);
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

/// The environment's variables say it (`LANG`).
pub fn system_language() -> Option<String> {
    None
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

// ---- The network ----

/// As macOS lays it out: not in Linux's order.
#[repr(C)]
pub struct AddrInfo {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addrlen: u32,
    canonname: *const u8,
    addr: *const u8,
    next: *mut AddrInfo,
}

const SOCK_STREAM: i32 = 1;
const ETIMEDOUT: i32 = 60;
const EAGAIN: i32 = 35;

pub struct Net;

impl Network for Net {
    type Link = i32;

    /// Never `secure`: src/network_framework.rs makes those.
    fn open(&mut self, host: &str, port: u16, _: bool, ms: u64) -> Result<i32, Fail> {
        if host.contains('\0') {
            return Err(Fail::NoHost);
        }
        let (host, port) = (c_path(host), c_path(&alloc::format!("{port}")));
        let null = core::ptr::null();
        let hints = AddrInfo { flags: 0, family: 0, socktype: SOCK_STREAM, protocol: 0, addrlen: 0, canonname: null, addr: null, next: core::ptr::null_mut() };
        let mut found = core::ptr::null_mut();
        if unsafe { c::getaddrinfo(host.as_ptr(), port.as_ptr(), &hints, &mut found) } != 0 {
            return Err(Fail::NoHost);
        }
        let (mut failure, mut a) = (Err(Fail::NoHost), found);
        while let Some(i) = unsafe { a.as_ref() } {
            let fd = unsafe { c::socket(i.family, SOCK_STREAM, i.protocol) };
            if fd >= 0 {
                // SO_NOSIGPIPE: a closed connection is a failure, not the end of the program.
                unsafe { c::setsockopt(fd, 0xFFFF, 0x1022, &1, 4) };
                if ms > 0 {
                    // SO_RCVTIMEO, SO_SNDTIMEO: seconds, microseconds; TCP_CONNECTIONTIMEOUT: seconds.
                    let limit = [(ms / 1000) as i64, (ms % 1000 * 1000) as i64];
                    for option in [0x1006, 0x1005] {
                        unsafe { c::setsockopt(fd, 0xFFFF, option, limit.as_ptr() as *const i32, 16) };
                    }
                    unsafe { c::setsockopt(fd, 6, 0x20, &(ms.div_ceil(1000) as i32), 4) };
                }
                if unsafe { c::connect(fd, i.addr, i.addrlen) } == 0 {
                    failure = Ok(fd);
                    break;
                }
                failure = Err(if errno() == ETIMEDOUT { Fail::Timeout } else { Fail::Refused });
                unsafe { c::close(fd) };
            }
            a = i.next;
        }
        unsafe { c::freeaddrinfo(found) };
        failure
    }

    fn send(&mut self, fd: &mut i32, mut data: &[u8]) -> Result<(), Fail> {
        while !data.is_empty() {
            match unsafe { c::send(*fd, data.as_ptr(), data.len(), 0) } {
                n if n >= 0 => data = &data[n as usize..],
                _ if errno() == EINTR => {}
                _ => return Err(cut()),
            }
        }
        Ok(())
    }

    fn receive(&mut self, fd: &mut i32, buffer: &mut [u8]) -> Result<usize, Fail> {
        loop {
            match unsafe { c::recv(*fd, buffer.as_mut_ptr(), buffer.len(), 0) } {
                n if n >= 0 => return Ok(n as usize),
                _ if errno() == EINTR => {}
                _ => return Err(cut()),
            }
        }
    }

    fn close(&mut self, fd: i32) {
        unsafe { c::close(fd) };
    }
}

impl Serve for Net {
    type Listener = i32;

    fn listen(&mut self, port: u16, everyone: bool) -> Option<i32> {
        let fd = unsafe { c::socket(2, SOCK_STREAM, 0) };
        if fd < 0 {
            return None;
        }
        // SO_REUSEADDR: the port can be taken again right after a server stops.
        unsafe { c::setsockopt(fd, 0xFFFF, 4, &1, 4) };
        // As BSD lays it out: its length first, then family, port (big end first), address.
        let mut a = [0u8; 16];
        a[0] = 16;
        a[1] = 2;
        a[2..4].copy_from_slice(&port.to_be_bytes());
        if !everyone {
            a[4..8].copy_from_slice(&[127, 0, 0, 1]);
        }
        if unsafe { c::bind(fd, a.as_ptr(), 16) } != 0 || unsafe { c::listen(fd, 128) } != 0 {
            unsafe { c::close(fd) };
            return None;
        }
        Some(fd)
    }

    fn accept(&mut self, listener: &mut i32, ms: u64) -> Option<(i32, [u8; 4])> {
        let (mut from, mut n) = ([0u8; 16], 16u32);
        let fd = unsafe { c::accept(*listener, from.as_mut_ptr(), &mut n) };
        if fd < 0 {
            return None;
        }
        // An accepted socket does not inherit SO_NOSIGPIPE.
        unsafe { c::setsockopt(fd, 0xFFFF, 0x1022, &1, 4) };
        let limit = [(ms / 1000) as i64, (ms % 1000 * 1000) as i64];
        for option in [0x1006, 0x1005] {
            unsafe { c::setsockopt(fd, 0xFFFF, option, limit.as_ptr() as *const i32, 16) };
        }
        Some((fd, [from[4], from[5], from[6], from[7]]))
    }

    /// `getifaddrs`: each address with its interface's name; a BSD `sockaddr` starts with its
    /// length, then its family (2 = IPv4).
    fn addresses(&mut self) -> Vec<(String, [u8; 4])> {
        let mut list = core::ptr::null_mut();
        let mut found = Vec::new();
        if unsafe { c::getifaddrs(&mut list) } != 0 {
            return found;
        }
        let mut at = list;
        while let Some(i) = unsafe { at.as_ref() } {
            if !i.addr.is_null() && unsafe { *i.addr.add(1) } == 2 {
                let ip = unsafe { core::slice::from_raw_parts(i.addr.add(4), 4) };
                let name = unsafe { core::ffi::CStr::from_ptr(i.name) }.to_string_lossy().into_owned();
                found.push((name, [ip[0], ip[1], ip[2], ip[3]]));
            }
            at = i.next;
        }
        unsafe { c::freeifaddrs(list) };
        found
    }

    /// `poll` without a time limit, as runtime/linux.rs: `struct pollfd`, POLLIN.
    fn ready(&mut self, listener: &i32, links: &[i32]) -> Vec<bool> {
        let mut fds: Vec<[i32; 2]> = links.iter().chain([listener]).map(|&fd| [fd, 1]).collect();
        if unsafe { c::poll(fds.as_mut_ptr(), fds.len() as u32, -1) } <= 0 {
            return alloc::vec![false; fds.len()];
        }
        fds.iter().map(|p| p[1] >> 16 != 0).collect()
    }
}

/// The first fields of `struct ifaddrs`.
#[repr(C)]
pub struct IfAddrs {
    next: *mut IfAddrs,
    name: *const core::ffi::c_char,
    flags: u32,
    addr: *const u8,
}

/// Like `cut` in src/web.rs.
fn cut() -> Fail {
    if matches!(errno(), ETIMEDOUT | EAGAIN) { Fail::Timeout } else { Fail::Broken }
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
