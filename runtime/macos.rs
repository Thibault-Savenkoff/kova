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
        pub fn getentropy(b: *mut u8, n: usize) -> i32;
        pub fn nanosleep(t: *const [i64; 2], rest: *mut [i64; 2]) -> i32;
        pub fn getpid() -> i32;
        // Intel Macs name the versions with 64-bit file numbers so.
        #[cfg_attr(target_arch = "x86_64", link_name = "stat$INODE64")]
        pub fn stat(path: *const u8, st: *mut u8) -> i32;
        #[cfg_attr(target_arch = "x86_64", link_name = "lstat$INODE64")]
        pub fn lstat(path: *const u8, st: *mut u8) -> i32;
        pub fn rename(a: *const u8, b: *const u8) -> i32;
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
        pub fn os_unfair_lock_lock(lock: *mut u32);
        pub fn os_unfair_lock_unlock(lock: *mut u32);
        pub fn pthread_attr_init(attributes: *mut [u64; 8]) -> i32;
        pub fn pthread_attr_setstacksize(attributes: *mut [u64; 8], n: usize) -> i32;
        pub fn pthread_create(thread: *mut usize, attributes: *const [u64; 8], start: extern "C" fn(*mut u8) -> *mut u8, arg: *mut u8) -> i32;
        pub fn pthread_join(thread: usize, result: *mut *mut u8) -> i32;
        pub fn pipe(fds: *mut [i32; 2]) -> i32;
        pub fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
        pub fn _NSGetEnviron() -> *const *const *const u8;
        pub fn posix_spawn(pid: *mut i32, path: *const u8, actions: *const usize, attributes: *const usize, argv: *const *const u8, envp: *const *const u8) -> i32;
        pub fn posix_spawn_file_actions_init(actions: *mut usize) -> i32;
        pub fn posix_spawn_file_actions_adddup2(actions: *mut usize, fd: i32, to: i32) -> i32;
        pub fn posix_spawn_file_actions_addopen(actions: *mut usize, fd: i32, path: *const u8, flags: i32, mode: u16) -> i32;
        pub fn posix_spawn_file_actions_destroy(actions: *mut usize) -> i32;
        pub fn posix_spawnattr_init(attributes: *mut usize) -> i32;
        pub fn posix_spawnattr_setflags(attributes: *mut usize, flags: i16) -> i32;
        pub fn posix_spawnattr_destroy(attributes: *mut usize) -> i32;
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

/// Fills `b` with the system's unguessable chance (`secure.random`), 256 bytes at a time at most.
pub fn chance(b: &mut [u8]) {
    for part in b.chunks_mut(256) {
        if unsafe { c::getentropy(part.as_mut_ptr(), part.len()) } != 0 {
            panic!("getentropy");
        }
    }
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

/// What `files.size` and `files.modified` say; `follow` = a link's target rather than the link.
pub fn info(path: &str, follow: bool) -> Result<crate::Info, i64> {
    let mut st = [0u8; 256];
    let p = c_path(path);
    if unsafe { if follow { c::stat(p.as_ptr(), st.as_mut_ptr()) } else { c::lstat(p.as_ptr(), st.as_mut_ptr()) } } != 0 {
        return Err(failure());
    }
    let kind = u16::from_ne_bytes([st[4], st[5]]) & 0o170000;
    let number = |at: usize| i64::from_ne_bytes(st[at..at + 8].try_into().unwrap());
    Ok(crate::Info { folder: kind == 0o040000, link: kind == 0o120000, size: number(96), modified: number(48) })
}

/// `a` named `b` from now on; Err(0) if they are on two disks (EXDEV).
pub fn rename(a: &str, b: &str) -> Result<(), i64> {
    if unsafe { c::rename(c_path(a).as_ptr(), c_path(b).as_ptr()) } == 0 {
        return Ok(());
    }
    if errno() == 18 { Err(0) } else { Err(failure()) }
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

// ---- Other programs ----

/// `system.run`: runs `program` (a path) with `args`, its input empty, and gives what it wrote,
/// what it wrote on stderr, and its exit status (`shared::exit_code`); or the `RunError` variant
/// + 1. The new process keeps only its three first files (POSIX_SPAWN_CLOEXEC_DEFAULT): a pipe
/// made meanwhile by another thread does not stay open in it.
pub fn run(program: &str, args: &[&str]) -> Result<(Vec<u8>, Vec<u8>, i64), i64> {
    let words: Vec<Vec<u8>> = [program].iter().chain(args).map(|w| c_path(w)).collect();
    let mut argv: Vec<*const u8> = words.iter().map(|w| w.as_ptr()).collect();
    argv.push(core::ptr::null());
    let (mut out, mut err) = ([0i32; 2], [0i32; 2]);
    if unsafe { c::pipe(&mut out) } != 0 {
        return Err(3);
    }
    if unsafe { c::pipe(&mut err) } != 0 {
        unsafe { c::close(out[0]) };
        unsafe { c::close(out[1]) };
        return Err(3);
    }
    let (mut actions, mut attributes, mut pid) = (0usize, 0usize, 0i32);
    let e = unsafe {
        c::posix_spawn_file_actions_init(&mut actions);
        c::posix_spawn_file_actions_addopen(&mut actions, 0, c"/dev/null".as_ptr() as *const u8, 0, 0);
        c::posix_spawn_file_actions_adddup2(&mut actions, out[1], 1);
        c::posix_spawn_file_actions_adddup2(&mut actions, err[1], 2);
        c::posix_spawnattr_init(&mut attributes);
        c::posix_spawnattr_setflags(&mut attributes, 0x4000); // POSIX_SPAWN_CLOEXEC_DEFAULT
        let e = c::posix_spawn(&mut pid, argv[0], &actions, &attributes, argv.as_ptr(), *c::_NSGetEnviron());
        c::posix_spawn_file_actions_destroy(&mut actions);
        c::posix_spawnattr_destroy(&mut attributes);
        c::close(out[1]);
        c::close(err[1]);
        e
    };
    if e != 0 {
        unsafe { c::close(out[0]) };
        unsafe { c::close(err[0]) };
        return Err(match e {
            2 | 20 => 1, // ENOENT, ENOTDIR
            1 | 13 => 2, // EPERM, EACCES
            _ => 3,
        });
    }
    // Both at once: a program filling one pipe while this one waits on the other would stop.
    let mut texts = [Vec::new(), Vec::new()];
    let mut open = [true, true];
    let mut buffer = [0u8; 1 << 14];
    while open.contains(&true) {
        let mut fds: Vec<[i32; 2]> = [out[0], err[0]].iter().zip(open).filter(|(_, o)| *o).map(|(&fd, _)| [fd, 1]).collect();
        let n = unsafe { c::poll(fds.as_mut_ptr(), fds.len() as u32, -1) };
        if n < 0 && errno() == EINTR {
            continue;
        }
        for p in &fds {
            if p[1] >> 16 == 0 && n >= 0 {
                continue;
            }
            let k = if p[0] == out[0] { 0 } else { 1 };
            match unsafe { c::read(p[0], buffer.as_mut_ptr(), buffer.len()) } {
                r if r > 0 => texts[k].extend_from_slice(&buffer[..r as usize]),
                r if r < 0 && errno() == EINTR => {}
                _ => open[k] = false,
            }
        }
    }
    unsafe { c::close(out[0]) };
    unsafe { c::close(err[0]) };
    let mut status = 0i32;
    while unsafe { c::waitpid(pid, &mut status, 0) } < 0 && errno() == EINTR {}
    let [output, errors] = texts;
    Ok((output, errors, crate::shared::exit_code(status)))
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

// ---- Threads (`serve(…) workers(…)`) ----

/// What one thread at a time may do (`crate::Locked`).
pub struct Lock(core::cell::UnsafeCell<u32>);

unsafe impl Sync for Lock {}

impl Lock {
    pub const fn new() -> Lock {
        Lock(core::cell::UnsafeCell::new(0))
    }
    pub fn lock(&self) {
        unsafe { c::os_unfair_lock_lock(self.0.get()) }
    }
    pub fn unlock(&self) {
        unsafe { c::os_unfair_lock_unlock(self.0.get()) }
    }
}

/// A thread `spawn` started.
pub struct Thread(usize);

/// `f(arg)` on a new thread. None if the system cannot start it.
pub fn spawn(f: extern "C" fn(i64), arg: i64) -> Option<Thread> {
    extern "C" fn begin(p: *mut u8) -> *mut u8 {
        let (f, arg) = *unsafe { alloc::boxed::Box::from_raw(p as *mut (extern "C" fn(i64), i64)) };
        f(arg);
        core::ptr::null_mut()
    }
    let (mut attributes, mut thread) = ([0u64; 8], 0);
    unsafe {
        c::pthread_attr_init(&mut attributes);
        // As big as the main thread's: a thread has 512 KB otherwise.
        c::pthread_attr_setstacksize(&mut attributes, 8 << 20);
        let started = c::pthread_create(&mut thread, &attributes, begin, alloc::boxed::Box::into_raw(alloc::boxed::Box::new((f, arg))) as *mut u8) == 0;
        started.then_some(Thread(thread))
    }
}

/// Waits for the end of `t`.
pub fn join(t: Thread) {
    unsafe { c::pthread_join(t.0, core::ptr::null_mut()) };
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

/// `ui.show` (src/ui.rs) with AppKit, through the Objective-C runtime (found at the first call):
/// the window stays, its content is made again from `view` (`NSStackView`s) when the view's shape
/// changed, else its views only take the new texts, so a field being typed into keeps its
/// cursor. The program runs its own event loop until something happens: the button clicked, a
/// field changed (`shared::ui_changed`, its text in `typed`), or -1 once the window is closed.
/// `KOVA_UI_HIDDEN` never shows the window.
pub fn ui_show(title: &str, view: &crate::shared::UiNode, width: i32, height: i32, acts: &[i64], typed: &mut String) -> i64 {
    use crate::Global;
    // `WAITING`: nothing yet, -1: the window closed.
    static CLICKED: Global<i64> = Global::new(WAITING);
    static TYPED: Global<String> = Global::new(String::new());
    // The view's shape and its texts, buttons and fields, to change them in place.
    static SHAPE: Global<String> = Global::new(String::new());
    static MADE: Global<Vec<Id>> = Global::new(Vec::new());
    const WAITING: i64 = i64::MIN;
    extern "C" fn clicked(_me: Id, _message: Id, sender: Id) {
        let kit = KIT.get().as_ref().expect("made before the window");
        *CLICKED.get() = (kit.send_int)(sender, kit.sel("tag"));
    }
    // A field's delegate hears each change the user makes (`setStringValue:` says nothing).
    extern "C" fn changed(_me: Id, _message: Id, note: Id) {
        let kit = KIT.get().as_ref().expect("made before the window");
        let field = kit.send(note, "object");
        *TYPED.get() = mac_text(kit, kit.send(field, "stringValue"));
        *CLICKED.get() = crate::shared::ui_changed((kit.send_int)(field, kit.sel("tag")) as usize);
    }
    extern "C" fn closing(_me: Id, _message: Id, _note: Id) {
        *CLICKED.get() = -1;
    }
    if *CLICKED.get() == -1 {
        return -1;
    }
    *CLICKED.get() = WAITING;
    let fresh = KIT.get().is_none();
    if fresh {
        let (Some(objc), Some(_)) = (library("/usr/lib/libobjc.A.dylib"), library("/System/Library/Frameworks/AppKit.framework/AppKit")) else { return -1 };
        let f = |name: &str| symbol(objc, name).unwrap_or(0);
        let send = f("objc_msgSend");
        unsafe {
            use core::mem::transmute as t;
            *KIT.get() = Some(Kit {
                class: t(f("objc_getClass")),
                selector: t(f("sel_registerName")),
                send: t(send),
                send_int: t(send),
                send_id: t(send),
                send_two: t(send),
                send_text: t(send),
                send_float: t(send),
                send_size: t(send),
                send_insets: t(send),
                fitting: t(send),
                window_init: t(send),
                event: t(send),
                push: t(f("objc_autoreleasePoolPush")),
                pop: t(f("objc_autoreleasePoolPop")),
                window: core::ptr::null_mut(),
                target: core::ptr::null_mut(),
                canvas: core::ptr::null_mut(),
            });
            let kit = KIT.get().as_mut().unwrap();
            let app = kit.send(kit.class("NSApplication"), "sharedApplication");
            (kit.send_id)(app, kit.sel("setActivationPolicy:"), 0); // Regular: in the Dock, with a menu bar
            // The `header`'s icon, in the Dock while the program runs.
            let icon = crate::icon();
            if !icon.is_empty() {
                let data_with: extern "C" fn(Id, Id, *const u8, u64) -> Id = t(send);
                let data = data_with(kit.class("NSData"), kit.sel("dataWithBytes:length:"), icon.as_ptr(), icon.len() as u64);
                let image = (kit.send_two)(kit.send(kit.class("NSImage"), "alloc"), kit.sel("initWithData:"), data);
                if !image.is_null() {
                    (kit.send_two)(app, kit.sel("setApplicationIconImage:"), image);
                }
            }
            kit.send(app, "finishLaunching");
            // A class of its own: `clicked:` for the buttons, `windowWillClose:` for the window,
            // `controlTextDidChange:` for the fields.
            let new_class: extern "C" fn(Id, *const u8, usize) -> Id = t(f("objc_allocateClassPair"));
            let add: extern "C" fn(Id, Id, usize, *const u8) -> bool = t(f("class_addMethod"));
            let register: extern "C" fn(Id) = t(f("objc_registerClassPair"));
            let class = new_class(kit.class("NSObject"), c"KovaUiTarget".as_ptr() as *const u8, 0);
            add(class, kit.sel("clicked:"), clicked as usize, c"v@:@".as_ptr() as *const u8);
            add(class, kit.sel("windowWillClose:"), closing as usize, c"v@:@".as_ptr() as *const u8);
            add(class, kit.sel("controlTextDidChange:"), changed as usize, c"v@:@".as_ptr() as *const u8);
            register(class);
            kit.target = kit.send(kit.send(class, "alloc"), "init");
            // The view of a `ui.canvas`: y going down, as everywhere else.
            let canvas = new_class(kit.class("NSView"), c"KovaUiCanvas".as_ptr() as *const u8, 0);
            let (yes, hears) = (c"c@:".as_ptr() as *const u8, c"v@:@".as_ptr() as *const u8);
            add(canvas, kit.sel("isFlipped"), mac_yes as usize, yes);
            add(canvas, kit.sel("acceptsFirstResponder"), mac_yes as usize, yes);
            add(canvas, kit.sel("acceptsFirstMouse:"), mac_yes as usize, c"c@:@".as_ptr() as *const u8);
            add(canvas, kit.sel("drawRect:"), mac_draw as usize, c"v@:{CGRect={CGPoint=dd}{CGSize=dd}}".as_ptr() as *const u8);
            add(canvas, kit.sel("mouseDown:"), mac_down as usize, hears);
            add(canvas, kit.sel("rightMouseDown:"), mac_right_down as usize, hears);
            add(canvas, kit.sel("mouseUp:"), mac_up as usize, hears);
            add(canvas, kit.sel("rightMouseUp:"), mac_right_up as usize, hears);
            add(canvas, kit.sel("mouseMoved:"), mac_moved as usize, hears);
            add(canvas, kit.sel("mouseDragged:"), mac_moved as usize, hears);
            add(canvas, kit.sel("rightMouseDragged:"), mac_moved as usize, hears);
            add(canvas, kit.sel("keyDown:"), mac_key as usize, hears);
            register(canvas);
            kit.canvas = canvas;
            // A window without a title bar refuses the keyboard unless a class of its own says yes.
            let bare = new_class(kit.class("NSWindow"), c"KovaUiWindow".as_ptr() as *const u8, 0);
            add(bare, kit.sel("canBecomeKeyWindow"), mac_yes as usize, yes);
            add(bare, kit.sel("canBecomeMainWindow"), mac_yes as usize, yes);
            register(bare);
            // Titled, closable, miniaturizable, resizable; or only the last three; buffered.
            *MAC_BARE.get() = acts.contains(&0);
            let (class, mask) = if *MAC_BARE.get() { (bare, 14) } else { (kit.class("NSWindow"), 15) };
            let frame = Rect { x: 200.0, y: 200.0, width: 320.0, height: 200.0 };
            let w = (kit.window_init)(kit.send(class, "alloc"), kit.sel("initWithContentRect:styleMask:backing:defer:"), frame, mask, 2, false);
            (kit.send_two)(w, kit.sel("setReleasedWhenClosed:"), core::ptr::null_mut());
            (kit.send_two)(w, kit.sel("setDelegate:"), kit.target);
            // A window without a title bar has no full screen unless it asks for it.
            (kit.send_id)(w, kit.sel("setCollectionBehavior:"), 1 << 7);
            kit.send(w, "center");
            kit.window = w;
            if var("KOVA_UI_HIDDEN").is_none() {
                (kit.send_two)(w, kit.sel("makeKeyAndOrderFront:"), core::ptr::null_mut());
                (kit.send_two)(app, kit.sel("activateIgnoringOtherApps:"), 1 as Id);
            }
        }
    }
    let kit = KIT.get().as_ref().unwrap();
    let pool = (kit.push)();
    let w = kit.window;
    (kit.send_two)(w, kit.sel("setTitle:"), kit.text(title));
    let shape = crate::shared::ui_shape(view);
    if shape == *SHAPE.get() {
        // The same views: only the texts that differ change.
        for ((leaf, _), &v) in crate::shared::ui_leaves(view).into_iter().zip(MADE.get().iter()) {
            use crate::shared::UiNode::*;
            let (s, setter) = match leaf {
                Text(s) | Input(s, _) => (s, "setStringValue:"),
                Button(s, _) => (s, "setTitle:"),
                Canvas(c) => {
                    if let Some(there) = MAC_CANVASES.get().iter_mut().find(|e| e.0 == v) {
                        there.1 = c.clone();
                    }
                    (kit.send_id)(v, kit.sel("setNeedsDisplay:"), 1);
                    kit.send(v, "displayIfNeeded");
                    continue;
                }
                _ => continue, // an image: its path is in the shape
            };
            let now = kit.send(v, if setter == "setTitle:" { "title" } else { "stringValue" });
            if mac_text(kit, now) != *s {
                (kit.send_two)(v, kit.sel(setter), kit.text(s));
            }
        }
    } else {
        let made = MADE.get();
        made.clear();
        MAC_CANVASES.get().clear();
        // 16 points around it; none in a window that has given up its frame.
        let edge = if *MAC_BARE.get() { 0.0 } else { 16.0 };
        let root = mac_make(kit, view, made, 2.0 * edge);
        // A view alone has no room around it: the window's content is always a stack view.
        let is_a: extern "C" fn(Id, Id, Id) -> bool = unsafe { core::mem::transmute(kit.send as usize) };
        let root = if is_a(root, kit.sel("isKindOfClass:"), kit.class("NSStackView")) {
            root
        } else {
            let stack = kit.send(kit.send(kit.send(kit.class("NSStackView"), "alloc"), "init"), "autorelease");
            (kit.send_id)(stack, kit.sel("setOrientation:"), 1);
            (kit.send_id)(stack, kit.sel("setAlignment:"), 5);
            (kit.send_two)(stack, kit.sel("addArrangedSubview:"), root);
            stack
        };
        (kit.send_insets)(root, kit.sel("setEdgeInsets:"), Insets { top: edge, left: edge, bottom: edge, right: edge });
        (kit.send_two)(w, kit.sel("setContentView:"), root);
        *SHAPE.get() = shape;
        if let Some((hearing, _)) = MAC_CANVASES.get().iter().find(|(_, c)| c.key.is_some()) {
            (kit.send_two)(w, kit.sel("makeFirstResponder:"), *hearing);
        }
        let now = mac_now(kit);
        *MAC_CLOCKS.get() = crate::shared::ui_clocks(view).into_iter().map(|(ms, n)| (ms as f64 / 1000.0, n, now + ms as f64 / 1000.0)).collect();
    }
    for ((leaf, style), &v) in crate::shared::ui_leaves(view).into_iter().zip(MADE.get().iter()) {
        mac_style(kit, v, leaf, style);
    }
    let root = kit.send(w, "contentView");
    // The window first takes the size the program asked for, else fits its content, at least 320
    // points wide; never smaller than the content. Later the stack view's own constraints make it
    // grow when the content needs more room, so a size the user chose stays.
    if fresh {
        let size = (kit.fitting)(root, kit.sel("fittingSize"));
        let asked = Size { width: if width > 0 { width as f64 } else { 320.0 }, height: height.max(0) as f64 };
        (kit.send_size)(w, kit.sel("setContentSize:"), Size { width: size.width.max(asked.width), height: size.height.max(asked.height) });
    }
    for act in acts {
        match act {
            1 => {
                kit.send(w, "close");
                (kit.pop)(pool);
                *CLICKED.get() = -1;
                return -1;
            }
            2 => {
                (kit.send_two)(w, kit.sel("miniaturize:"), core::ptr::null_mut());
            }
            // The system moves the window until the button is released.
            3 if !MAC_PRESS.get().is_null() => {
                (kit.send_two)(w, kit.sel("performWindowDragWithEvent:"), *MAC_PRESS.get());
            }
            4 => {
                (kit.send_two)(w, kit.sel("toggleFullScreen:"), core::ptr::null_mut());
            }
            _ => {}
        }
    }
    let app = kit.send(kit.class("NSApplication"), "sharedApplication");
    let (future, mode) = (kit.send(kit.class("NSDate"), "distantFuture"), kit.text("kCFRunLoopDefaultMode"));
    while *CLICKED.get() == WAITING && MAC_HEARD.get().is_empty() {
        // Waits for the next event or the next tick, whichever comes first.
        let now = mac_now(kit);
        for clock in MAC_CLOCKS.get().iter_mut().filter(|clock| clock.2 <= now) {
            clock.2 = now + clock.0;
            MAC_HEARD.get().hear(clock.1 as i64, String::new());
        }
        if !MAC_HEARD.get().is_empty() {
            break;
        }
        let next = MAC_CLOCKS.get().iter().map(|clock| clock.2).fold(f64::INFINITY, f64::min);
        let until = if next.is_finite() { (kit.send_float)(kit.class("NSDate"), kit.sel("dateWithTimeIntervalSinceNow:"), next - now) } else { future };
        let e = (kit.event)(app, kit.sel("nextEventMatchingMask:untilDate:inMode:dequeue:"), u64::MAX, until, mode, true);
        if !e.is_null() {
            (kit.send_two)(app, kit.sel("sendEvent:"), e);
        }
        kit.send(app, "updateWindows");
    }
    (kit.pop)(pool);
    if *CLICKED.get() == WAITING {
        let (answer, text) = MAC_HEARD.get().next().unwrap_or((-1, String::new()));
        *typed = text;
        return answer;
    }
    if *CLICKED.get() <= -2 {
        *typed = TYPED.get().clone();
    }
    *CLICKED.get()
}

static KIT: crate::Global<Option<Kit>> = crate::Global::new(None);
/// `ui.run(…) frame(false)`.
static MAC_BARE: crate::Global<bool> = crate::Global::new(false);
/// What a canvas or a clock heard, for `ui_show`.
static MAC_HEARD: crate::Global<crate::shared::UiQueue> = crate::Global::new(crate::shared::UiQueue::new());
/// The clocks: (seconds between ticks, button number, next tick).
static MAC_CLOCKS: crate::Global<Vec<(f64, usize, f64)>> = crate::Global::new(Vec::new());
/// Each canvas now on screen.
static MAC_CANVASES: crate::Global<Vec<(Id, crate::shared::UiCanvas)>> = crate::Global::new(Vec::new());

/// Seconds, from `NSDate`.
fn mac_now(kit: &Kit) -> f64 {
    let seconds: extern "C" fn(Id, Id) -> f64 = unsafe { core::mem::transmute(kit.send as usize) };
    seconds(kit.class("NSDate"), kit.sel("timeIntervalSinceReferenceDate"))
}

extern "C" fn mac_yes(_me: Id, _message: Id) -> bool {
    true
}

fn mac_mouse(me: Id, event: Id, kind: &str, button: i32) {
    let Some(kit) = KIT.get().as_ref() else { return };
    let Some(n) = MAC_CANVASES.get().iter().find(|e| e.0 == me).and_then(|e| e.1.mouse) else { return };
    let convert: extern "C" fn(Id, Id, Size, Id) -> Size = unsafe { core::mem::transmute(kit.send as usize) };
    let at = convert(me, kit.sel("convertPoint:fromView:"), (kit.fitting)(event, kit.sel("locationInWindow")), core::ptr::null_mut());
    MAC_HEARD.get().hear(crate::shared::ui_changed(n), crate::shared::ui_mouse(kind, at.width as i32, at.height as i32, button));
}

/// The last press, kept for `ui.drag`.
static MAC_PRESS: crate::Global<Id> = crate::Global::new(core::ptr::null_mut());

fn mac_pressed(me: Id, event: Id, button: i32) {
    if let Some(kit) = KIT.get().as_ref() {
        (kit.send_two)(kit.window, kit.sel("makeFirstResponder:"), me);
        if !MAC_PRESS.get().is_null() {
            kit.send(*MAC_PRESS.get(), "release");
        }
        *MAC_PRESS.get() = kit.send(event, "retain");
    }
    mac_mouse(me, event, "press", button);
}

extern "C" fn mac_down(me: Id, _message: Id, event: Id) {
    mac_pressed(me, event, 1);
}

extern "C" fn mac_right_down(me: Id, _message: Id, event: Id) {
    mac_pressed(me, event, 2);
}

extern "C" fn mac_up(me: Id, _message: Id, event: Id) {
    mac_mouse(me, event, "release", 1);
}

extern "C" fn mac_right_up(me: Id, _message: Id, event: Id) {
    mac_mouse(me, event, "release", 2);
}

extern "C" fn mac_moved(me: Id, _message: Id, event: Id) {
    mac_mouse(me, event, "move", 0);
}

/// `ui.Key`'s name for a key that writes nothing: the same word on every system. `keyCode`s are
/// places on the keyboard, the same on every Mac.
fn mac_key_name(code: i64) -> Option<&'static str> {
    Some(match code {
        36 | 76 => "Enter",
        48 => "Tab",
        49 => "Space",
        51 => "Backspace",
        53 => "Escape",
        115 => "Home",
        116 => "PageUp",
        117 => "Delete",
        119 => "End",
        121 => "PageDown",
        123 => "Left",
        124 => "Right",
        125 => "Down",
        126 => "Up",
        122 => "F1",
        120 => "F2",
        99 => "F3",
        118 => "F4",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        100 => "F8",
        101 => "F9",
        109 => "F10",
        103 => "F11",
        111 => "F12",
        _ => return None,
    })
}

extern "C" fn mac_key(me: Id, _message: Id, event: Id) {
    let Some(kit) = KIT.get().as_ref() else { return };
    let Some(n) = MAC_CANVASES.get().iter().find(|e| e.0 == me).and_then(|e| e.1.key) else { return };
    let code = (kit.send_int)(event, kit.sel("keyCode")) & 0xffff;
    let name = match mac_key_name(code) {
        Some(name) => name.into(),
        None => mac_text(kit, kit.send(event, "characters")),
    };
    if !name.is_empty() {
        MAC_HEARD.get().hear(crate::shared::ui_changed(n), crate::shared::ui_key(&name));
    }
}

/// Paints a canvas, from what `ui.canvas` drew. Thickness stays inside a shape, as on every system.
extern "C" fn mac_draw(me: Id, _message: Id, _dirty: Rect) {
    use crate::shared::UiOp as Op;
    let Some(kit) = KIT.get().as_ref() else { return };
    let Some((_, canvas)) = MAC_CANVASES.get().iter().find(|e| e.0 == me) else { return };
    let bounds = mac_bounds(kit, me);
    if (canvas.width == 0 || canvas.height == 0) && crate::ui_room(canvas.number, bounds.width as i32, bounds.height as i32) {
        MAC_HEARD.get().hear(crate::shared::UI_RESIZED, String::new());
    }
    let Some(graphics) = library("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics") else { return };
    let Some(appkit) = library("/System/Library/Frameworks/AppKit.framework/AppKit") else { return };
    let f = |name: &str| symbol(graphics, name).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let context = kit.send(kit.send(kit.class("NSGraphicsContext"), "currentContext"), "CGContext");
        let fill_color: extern "C" fn(Id, f64, f64, f64, f64) = t(f("CGContextSetRGBFillColor"));
        let stroke_color: extern "C" fn(Id, f64, f64, f64, f64) = t(f("CGContextSetRGBStrokeColor"));
        let fill_rect: extern "C" fn(Id, Rect) = t(f("CGContextFillRect"));
        let fill_oval: extern "C" fn(Id, Rect) = t(f("CGContextFillEllipseInRect"));
        let stroke_rect: extern "C" fn(Id, Rect) = t(f("CGContextStrokeRect"));
        let stroke_oval: extern "C" fn(Id, Rect) = t(f("CGContextStrokeEllipseInRect"));
        let thickness: extern "C" fn(Id, f64) = t(f("CGContextSetLineWidth"));
        let move_to: extern "C" fn(Id, f64, f64) = t(f("CGContextMoveToPoint"));
        let line_to: extern "C" fn(Id, f64, f64) = t(f("CGContextAddLineToPoint"));
        let stroke: extern "C" fn(Id) = t(f("CGContextStrokePath"));
        let part = |c: u32, shift: u32| (c >> shift & 255) as f64 / 255.0;
        let fill = |c: u32| fill_color(context, part(c, 16), part(c, 8), part(c, 0), 1.0);
        let line = |c: u32, n: i32| {
            stroke_color(context, part(c, 16), part(c, 8), part(c, 0), 1.0);
            thickness(context, n as f64);
        };
        let inside = |x: i32, y: i32, w: i32, h: i32, n: i32| {
            let inset = n as f64 / 2.0;
            Rect { x: x as f64 + inset, y: y as f64 + inset, width: w as f64 - 2.0 * inset, height: h as f64 - 2.0 * inset }
        };
        let whole = Rect { x: 0.0, y: 0.0, width: bounds.width, height: bounds.height };
        fill(0xffffff);
        fill_rect(context, whole);
        for op in &canvas.ops {
            match op {
                Op::Clear(c) => {
                    fill(*c);
                    fill_rect(context, whole);
                }
                Op::Rect { x, y, w, h, color, outline: 0 } => {
                    fill(*color);
                    fill_rect(context, inside(*x, *y, *w, *h, 0));
                }
                Op::Rect { x, y, w, h, color, outline } => {
                    line(*color, *outline);
                    stroke_rect(context, inside(*x, *y, *w, *h, *outline));
                }
                Op::Circle { x, y, r, color, outline: 0 } => {
                    fill(*color);
                    fill_oval(context, inside(x - r, y - r, 2 * r, 2 * r, 0));
                }
                Op::Circle { x, y, r, color, outline } => {
                    line(*color, *outline);
                    stroke_oval(context, inside(x - r, y - r, 2 * r, 2 * r, *outline));
                }
                Op::Line { x, y, x2, y2, color, thickness } => {
                    line(*color, *thickness);
                    move_to(context, *x as f64, *y as f64);
                    line_to(context, *x2 as f64, *y2 as f64);
                    stroke(context);
                }
                Op::Text { x, y, size, color, text } => {
                    let named = |name: &str| symbol(appkit, name).map_or(core::ptr::null_mut(), |p| *(p as *const Id));
                    let keys = [named("NSFontAttributeName"), named("NSForegroundColorAttributeName")];
                    let make_color: extern "C" fn(Id, Id, f64, f64, f64, f64) -> Id = t(kit.send as usize);
                    let values = [
                        (kit.send_float)(kit.class("NSFont"), kit.sel("systemFontOfSize:"), *size as f64),
                        make_color(kit.class("NSColor"), kit.sel("colorWithSRGBRed:green:blue:alpha:"), part(*color, 16), part(*color, 8), part(*color, 0), 1.0),
                    ];
                    let dictionary: extern "C" fn(Id, Id, *const Id, *const Id, u64) -> Id = t(kit.send as usize);
                    let attributes = dictionary(kit.class("NSDictionary"), kit.sel("dictionaryWithObjects:forKeys:count:"), values.as_ptr(), keys.as_ptr(), 2);
                    let draw: extern "C" fn(Id, Id, Size, Id) = t(kit.send as usize);
                    draw(kit.text(text), kit.sel("drawAtPoint:withAttributes:"), Size { width: *x as f64, height: *y as f64 }, attributes);
                }
            }
        }
    }
}

/// A view's `bounds`. A struct this big comes back through memory on Intel, a variant of
/// `objc_msgSend` that arm64 does not have.
fn mac_bounds(kit: &Kit, view: Id) -> Rect {
    #[cfg(target_arch = "x86_64")]
    {
        let mut out = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
        if let Some(send) = library("/usr/lib/libobjc.A.dylib").and_then(|l| symbol(l, "objc_msgSend_stret")) {
            let send: extern "C" fn(*mut Rect, Id, Id) = unsafe { core::mem::transmute(send) };
            send(&mut out, view, kit.sel("bounds"));
        }
        out
    }
    #[cfg(target_arch = "aarch64")]
    {
        let send: extern "C" fn(Id, Id) -> Rect = unsafe { core::mem::transmute(kit.send as usize) };
        send(view, kit.sel("bounds"))
    }
}

/// A canvas and its constraints; it hears the mouse moving wherever it is, a field having the keys.
fn mac_canvas(kit: &Kit, c: &crate::shared::UiCanvas) -> Id {
    let view = kit.send(kit.send(kit.canvas, "alloc"), "init");
    let give_way: extern "C" fn(Id, Id, f32, i64) = unsafe { core::mem::transmute(kit.send as usize) };
    for (orientation, anchor, length) in [(0, "widthAnchor", c.width), (1, "heightAnchor", c.height)] {
        if length == 0 {
            // `ui.fill`: it takes the room the stack view has left, before any other part.
            give_way(view, kit.sel("setContentHuggingPriority:forOrientation:"), 1.0, orientation);
            continue;
        }
        let fixed = (kit.send_float)(kit.send(view, anchor), kit.sel("constraintEqualToConstant:"), length as f64);
        (kit.send_id)(fixed, kit.sel("setActive:"), 1);
    }
    if c.mouse.is_some() {
        // NSTrackingMouseMoved | NSTrackingActiveInKeyWindow | NSTrackingInVisibleRect.
        let init: extern "C" fn(Id, Id, Rect, u64, Id, Id) -> Id = unsafe { core::mem::transmute(kit.send as usize) };
        let nothing = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
        let area = init(kit.send(kit.class("NSTrackingArea"), "alloc"), kit.sel("initWithRect:options:owner:userInfo:"), nothing, 0x02 | 0x20 | 0x200, view, core::ptr::null_mut());
        (kit.send_two)(view, kit.sel("addTrackingArea:"), area);
    }
    MAC_CANVASES.get().push((view, c.clone()));
    (kit.send)(view, kit.sel("autorelease"))
}

/// An `NSString`'s text, copied (its bytes read one by one: no `strlen`).
fn mac_text(kit: &Kit, s: Id) -> String {
    if s.is_null() {
        return String::new();
    }
    let p = kit.send(s, "UTF8String") as *const u8;
    let mut bytes = Vec::new();
    while !p.is_null() && unsafe { p.add(bytes.len()).read_volatile() } != 0 {
        bytes.push(unsafe { p.add(bytes.len()).read_volatile() });
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

type Id = *mut u8;

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Insets {
    top: f64,
    left: f64,
    bottom: f64,
    right: f64,
}

/// The Objective-C runtime: `objc_msgSend` once per prototype, as an `extern` declares it.
struct Kit {
    class: extern "C" fn(*const u8) -> Id,
    selector: extern "C" fn(*const u8) -> Id,
    send: extern "C" fn(Id, Id) -> Id,
    send_int: extern "C" fn(Id, Id) -> i64,
    send_id: extern "C" fn(Id, Id, i64) -> Id,
    send_two: extern "C" fn(Id, Id, Id) -> Id,
    send_text: extern "C" fn(Id, Id, *const u8) -> Id,
    send_float: extern "C" fn(Id, Id, f64) -> Id,
    send_size: extern "C" fn(Id, Id, Size) -> Id,
    send_insets: extern "C" fn(Id, Id, Insets) -> Id,
    fitting: extern "C" fn(Id, Id) -> Size,
    window_init: extern "C" fn(Id, Id, Rect, u64, u64, bool) -> Id,
    event: extern "C" fn(Id, Id, u64, Id, Id, bool) -> Id,
    push: extern "C" fn() -> Id,
    pop: extern "C" fn(Id),
    window: Id,
    target: Id,
    canvas: Id,
}

impl Kit {
    fn class(&self, name: &str) -> Id {
        (self.class)([name.as_bytes(), &[0]].concat().as_ptr())
    }
    fn sel(&self, name: &str) -> Id {
        (self.selector)([name.as_bytes(), &[0]].concat().as_ptr())
    }
    fn send(&self, to: Id, message: &str) -> Id {
        (self.send)(to, self.sel(message))
    }
    /// An `NSString` (autoreleased).
    fn text(&self, t: &str) -> Id {
        (self.send_text)(self.class("NSString"), self.sel("stringWithUTF8String:"), [t.as_bytes(), &[0]].concat().as_ptr())
    }
}

/// Gives a text, button or field its style; without one, the system's font and color.
fn mac_style(kit: &Kit, view: Id, leaf: &crate::shared::UiNode, style: crate::shared::UiStyle) {
    use crate::shared::UiNode::*;
    if matches!(leaf, Image(..) | Canvas(_)) {
        return;
    }
    let size = if style.size > 0 { style.size as f64 } else { 13.0 }; // `NSFont.systemFontSize`
    let font = (kit.send_float)(kit.class("NSFont"), kit.sel(if style.bold { "boldSystemFontOfSize:" } else { "systemFontOfSize:" }), size);
    (kit.send_two)(view, kit.sel("setFont:"), font);
    if matches!(leaf, Button(..)) {
        return;
    }
    let color = match style.color {
        Some(c) => {
            let channel = |shift: u32| ((c >> shift) & 0xff) as f64 / 255.0;
            let make: extern "C" fn(Id, Id, f64, f64, f64, f64) -> Id = unsafe { core::mem::transmute(kit.send as usize) };
            make(kit.class("NSColor"), kit.sel("colorWithSRGBRed:green:blue:alpha:"), channel(16), channel(8), channel(0), 1.0)
        }
        None => kit.send(kit.class("NSColor"), if matches!(leaf, Input(..)) { "textColor" } else { "labelColor" }),
    };
    (kit.send_two)(view, kit.sel("setTextColor:"), color);
}

/// A stack view's part that fills its width, `insets` kept from its edges.
fn mac_fill(kit: &Kit, part: Id, stack: Id, insets: f64) {
    let equal: extern "C" fn(Id, Id, Id, f64) -> Id = unsafe { core::mem::transmute(kit.send as usize) };
    let width = equal(kit.send(part, "widthAnchor"), kit.sel("constraintEqualToAnchor:constant:"), kit.send(stack, "widthAnchor"), -insets);
    (kit.send_id)(width, kit.sel("setActive:"), 1);
}

/// The views of a view: a button's tag is its number, `clicked:` reads it; a field's too, for
/// `controlTextDidChange:`. Each text, button and field goes into `made`, in order. `insets`:
/// the width its own edges take from its parts.
fn mac_make(kit: &Kit, node: &crate::shared::UiNode, made: &mut Vec<Id>, insets: f64) -> Id {
    use crate::shared::UiNode::*;
    let leaf = match node {
        Text(s) => (kit.send_two)(kit.class("NSTextField"), kit.sel("labelWithString:"), kit.text(s)),
        Button(s, n) => {
            let make: extern "C" fn(Id, Id, Id, Id, Id) -> Id = unsafe { core::mem::transmute(kit.send as usize) };
            let b = make(kit.class("NSButton"), kit.sel("buttonWithTitle:target:action:"), kit.text(s), kit.target, kit.sel("clicked:"));
            (kit.send_id)(b, kit.sel("setTag:"), *n as i64);
            b
        }
        // At least 200 points wide, even empty.
        Input(s, n) => {
            let field = (kit.send_two)(kit.class("NSTextField"), kit.sel("textFieldWithString:"), kit.text(s));
            (kit.send_id)(field, kit.sel("setTag:"), *n as i64);
            (kit.send_two)(field, kit.sel("setDelegate:"), kit.target);
            let wide = (kit.send_float)(kit.send(field, "widthAnchor"), kit.sel("constraintGreaterThanOrEqualToConstant:"), 200.0);
            (kit.send_id)(wide, kit.sel("setActive:"), 1);
            field
        }
        // NSUserInterfaceLayoutOrientationHorizontal = 0, Vertical = 1. `NSLayoutAttribute`:
        // Top 3, Bottom 4, Leading 5, Trailing 6, CenterX 9, CenterY 10.
        Column(parts, a) | Row(parts, a) => {
            use crate::shared::UiAlign::*;
            let stack = kit.send(kit.send(kit.class("NSStackView"), "alloc"), "init");
            let column = matches!(node, Column(..));
            (kit.send_id)(stack, kit.sel("setOrientation:"), column as i64);
            let place = match (column, a) {
                (true, Start) => 5,
                (true, Center) => 9,
                (true, End) => 6,
                (false, Start) => 3,
                (false, Center) => 10,
                (false, End) => 4,
            };
            (kit.send_id)(stack, kit.sel("setAlignment:"), place);
            (kit.send_float)(stack, kit.sel("setSpacing:"), 8.0);
            if crate::shared::ui_fills_down(node) && column || crate::shared::ui_fills(node) && !column {
                (kit.send_id)(stack, kit.sel("setDistribution:"), 0); // NSStackViewDistributionFill
            }
            for p in parts {
                let part = mac_make(kit, p, made, 0.0);
                (kit.send_two)(stack, kit.sel("addArrangedSubview:"), part);
                if column && crate::shared::ui_fills(p) {
                    mac_fill(kit, part, stack, insets);
                }
            }
            return (kit.send)(stack, kit.sel("autorelease"));
        }
        Image(path, width) => {
            let image = (kit.send_two)(kit.send(kit.class("NSImage"), "alloc"), kit.sel("initWithContentsOfFile:"), kit.text(path));
            if image.is_null() {
                (kit.send_two)(kit.class("NSTextField"), kit.sel("labelWithString:"), kit.text(""))
            } else {
                let v = (kit.send_two)(kit.class("NSImageView"), kit.sel("imageViewWithImage:"), image);
                if *width > 0 {
                    let natural = (kit.fitting)(image, kit.sel("size"));
                    let width = *width as f64;
                    (kit.send_id)(v, kit.sel("setImageScaling:"), 3); // NSImageScaleProportionallyUpOrDown
                    for (anchor, length) in [("widthAnchor", width), ("heightAnchor", width * natural.height / natural.width.max(1.0))] {
                        let fixed = (kit.send_float)(kit.send(v, anchor), kit.sel("constraintEqualToConstant:"), length);
                        (kit.send_id)(fixed, kit.sel("setActive:"), 1);
                    }
                }
                v
            }
        }
        Styled(_, inner) => return mac_make(kit, inner, made, insets),
        Every(_, _, inner) => return mac_make(kit, inner, made, insets),
        Canvas(c) => mac_canvas(kit, c),
        // A stack view of one part, with room around it.
        Spaced(m, inner) => {
            let stack = kit.send(kit.send(kit.class("NSStackView"), "alloc"), "init");
            (kit.send_id)(stack, kit.sel("setOrientation:"), 1);
            (kit.send_id)(stack, kit.sel("setAlignment:"), 5);
            let m = *m as f64;
            (kit.send_insets)(stack, kit.sel("setEdgeInsets:"), Insets { top: m, left: m, bottom: m, right: m });
            let part = mac_make(kit, inner, made, 0.0);
            (kit.send_two)(stack, kit.sel("addArrangedSubview:"), part);
            if crate::shared::ui_fills(inner) {
                mac_fill(kit, part, stack, 2.0 * m);
            }
            return (kit.send)(stack, kit.sel("autorelease"));
        }
    };
    made.push(leaf);
    leaf
}
