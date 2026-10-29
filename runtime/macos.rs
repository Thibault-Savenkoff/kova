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
/// the window stays, its content is made again from `view` each time (`NSStackView`s). The
/// program runs its own event loop until a click: the button's number, or -1 once the window
/// is closed. `KOVA_UI_HIDDEN` never shows the window.
pub fn ui_show(title: &str, view: &crate::shared::UiNode, width: i32, height: i32) -> i64 {
    use crate::Global;
    static KIT: Global<Option<Kit>> = Global::new(None);
    // -2: nothing yet, -1: the window closed.
    static CLICKED: Global<i64> = Global::new(-2);
    extern "C" fn clicked(_me: Id, _message: Id, sender: Id) {
        let kit = KIT.get().as_ref().expect("made before the window");
        *CLICKED.get() = (kit.send_int)(sender, kit.sel("tag"));
    }
    extern "C" fn closing(_me: Id, _message: Id, _note: Id) {
        *CLICKED.get() = -1;
    }
    if *CLICKED.get() == -1 {
        return -1;
    }
    *CLICKED.get() = -2;
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
            });
            let kit = KIT.get().as_mut().unwrap();
            let app = kit.send(kit.class("NSApplication"), "sharedApplication");
            (kit.send_id)(app, kit.sel("setActivationPolicy:"), 0); // Regular: in the Dock, with a menu bar
            kit.send(app, "finishLaunching");
            // A class of its own: `clicked:` for the buttons, `windowWillClose:` for the window.
            let new_class: extern "C" fn(Id, *const u8, usize) -> Id = t(f("objc_allocateClassPair"));
            let add: extern "C" fn(Id, Id, usize, *const u8) -> bool = t(f("class_addMethod"));
            let register: extern "C" fn(Id) = t(f("objc_registerClassPair"));
            let class = new_class(kit.class("NSObject"), c"KovaUiTarget".as_ptr() as *const u8, 0);
            add(class, kit.sel("clicked:"), clicked as usize, c"v@:@".as_ptr() as *const u8);
            add(class, kit.sel("windowWillClose:"), closing as usize, c"v@:@".as_ptr() as *const u8);
            register(class);
            kit.target = kit.send(kit.send(class, "alloc"), "init");
            // Titled, closable, miniaturizable, resizable; buffered.
            let frame = Rect { x: 200.0, y: 200.0, width: 320.0, height: 200.0 };
            let w = (kit.window_init)(kit.send(kit.class("NSWindow"), "alloc"), kit.sel("initWithContentRect:styleMask:backing:defer:"), frame, 15, 2, false);
            (kit.send_two)(w, kit.sel("setReleasedWhenClosed:"), core::ptr::null_mut());
            (kit.send_two)(w, kit.sel("setDelegate:"), kit.target);
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
    let root = mac_make(kit, view);
    (kit.send_insets)(root, kit.sel("setEdgeInsets:"), Insets { top: 16.0, left: 16.0, bottom: 16.0, right: 16.0 });
    (kit.send_two)(w, kit.sel("setContentView:"), root);
    // The window first takes the size the program asked for, else fits its content, at least 320
    // points wide; never smaller than the content. Later the stack view's own constraints make it
    // grow when the content needs more room, so a size the user chose stays.
    if fresh {
        let size = (kit.fitting)(root, kit.sel("fittingSize"));
        let asked = Size { width: if width > 0 { width as f64 } else { 320.0 }, height: height.max(0) as f64 };
        (kit.send_size)(w, kit.sel("setContentSize:"), Size { width: size.width.max(asked.width), height: size.height.max(asked.height) });
    }
    let app = kit.send(kit.class("NSApplication"), "sharedApplication");
    let (future, mode) = (kit.send(kit.class("NSDate"), "distantFuture"), kit.text("kCFRunLoopDefaultMode"));
    while *CLICKED.get() == -2 {
        let e = (kit.event)(app, kit.sel("nextEventMatchingMask:untilDate:inMode:dequeue:"), u64::MAX, future, mode, true);
        if !e.is_null() {
            (kit.send_two)(app, kit.sel("sendEvent:"), e);
        }
        kit.send(app, "updateWindows");
    }
    (kit.pop)(pool);
    *CLICKED.get()
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

/// The views of a view: a button's tag is its number, `clicked:` reads it.
fn mac_make(kit: &Kit, node: &crate::shared::UiNode) -> Id {
    use crate::shared::UiNode::*;
    match node {
        Text(s) => (kit.send_two)(kit.class("NSTextField"), kit.sel("labelWithString:"), kit.text(s)),
        Button(s, n) => {
            let make: extern "C" fn(Id, Id, Id, Id, Id) -> Id = unsafe { core::mem::transmute(kit.send as usize) };
            let b = make(kit.class("NSButton"), kit.sel("buttonWithTitle:target:action:"), kit.text(s), kit.target, kit.sel("clicked:"));
            (kit.send_id)(b, kit.sel("setTag:"), *n as i64);
            b
        }
        // NSUserInterfaceLayoutOrientationHorizontal = 0, Vertical = 1; a column's parts on the
        // left (NSLayoutAttributeLeading = 5).
        Column(parts) | Row(parts) => {
            let stack = kit.send(kit.send(kit.class("NSStackView"), "alloc"), "init");
            let column = matches!(node, Column(_));
            (kit.send_id)(stack, kit.sel("setOrientation:"), column as i64);
            if column {
                (kit.send_id)(stack, kit.sel("setAlignment:"), 5);
            }
            (kit.send_float)(stack, kit.sel("setSpacing:"), 8.0);
            for p in parts {
                (kit.send_two)(stack, kit.sel("addArrangedSubview:"), mac_make(kit, p));
            }
            (kit.send)(stack, kit.sel("autorelease"))
        }
    }
}
