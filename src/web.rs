// The network of `kova run` (PLAN Part 4 "The web"): Rust's own, which asks the system as the
// runtime does (`getaddrinfo`, then a connection), so that the failures are the same.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};

use crate::http::{Fail, Network, Serve};

pub struct System;

/// What `kova run` connects with: the system, encrypted by SChannel on Windows, OpenSSL on Linux,
/// Network.framework on macOS.
#[cfg(windows)]
type Tls = crate::schannel::Tls<System>;
#[cfg(target_os = "linux")]
type Tls = crate::openssl::Tls<System>;
#[cfg(target_os = "macos")]
type Tls = crate::network_framework::Tls<System>;

#[cfg(windows)]
pub fn network() -> crate::schannel::Tls<System> {
    crate::schannel::Tls(System)
}

#[cfg(target_os = "linux")]
pub fn network() -> crate::openssl::Tls<System> {
    crate::openssl::Tls(System)
}

#[cfg(target_os = "macos")]
pub fn network() -> crate::network_framework::Tls<System> {
    crate::network_framework::Tls(System)
}

impl Network for System {
    type Link = TcpStream;

    /// Never `secure`: each system's encryption is around it (`network`).
    fn open(&mut self, host: &str, port: u16, _: bool, ms: u64) -> Result<TcpStream, Fail> {
        let addresses: Vec<_> = (host, port).to_socket_addrs().map_err(|_| Fail::NoHost)?.collect();
        let limit = (ms > 0).then(|| std::time::Duration::from_millis(ms));
        let mut failure = Fail::NoHost;
        for a in addresses {
            let link = match limit {
                Some(d) => TcpStream::connect_timeout(&a, d),
                None => TcpStream::connect(a),
            };
            match link {
                Ok(s) => {
                    s.set_read_timeout(limit).and_then(|()| s.set_write_timeout(limit)).map_err(cut)?;
                    return Ok(s);
                }
                Err(e) => failure = if e.kind() == ErrorKind::TimedOut { Fail::Timeout } else { Fail::Refused },
            }
        }
        Err(failure)
    }

    fn send(&mut self, link: &mut TcpStream, data: &[u8]) -> Result<(), Fail> {
        link.write_all(data).map_err(cut)
    }

    fn receive(&mut self, link: &mut TcpStream, buffer: &mut [u8]) -> Result<usize, Fail> {
        loop {
            match link.read(buffer) {
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                r => return r.map_err(cut),
            }
        }
    }

    fn wake(&mut self, link: &TcpStream) {
        let _ = link.shutdown(std::net::Shutdown::Both);
    }

    fn close(&mut self, _: TcpStream) {}
}

impl crate::http::Sockets for System {
    type Tls = Tls;
    fn plain() -> System {
        System
    }
    fn tls() -> Self::Tls {
        network()
    }
    fn table<R>(f: impl FnOnce(&mut crate::http::Table<System>) -> R) -> R {
        struct Sockets(crate::http::Table<System>);
        // Reached through this lock only.
        unsafe impl Send for Sockets {}
        static TABLE: std::sync::Mutex<Sockets> = std::sync::Mutex::new(Sockets(crate::http::Table::new()));
        f(&mut TABLE.lock().unwrap().0)
    }
    fn nap() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

impl Serve for System {
    type Listener = TcpListener;
    type Datagram = std::net::UdpSocket;

    fn stop(&mut self, _: TcpListener) {}

    fn port(&mut self, listener: &TcpListener) -> u16 {
        listener.local_addr().map_or(0, |a| a.port())
    }

    fn datagram_port(&mut self, socket: &std::net::UdpSocket) -> u16 {
        socket.local_addr().map_or(0, |a| a.port())
    }

    fn udp(&mut self, port: u16, everyone: bool, ms: u64) -> Option<std::net::UdpSocket> {
        let socket = std::net::UdpSocket::bind((if everyone { "0.0.0.0" } else { "127.0.0.1" }, port)).ok()?;
        socket.set_read_timeout((ms > 0).then(|| std::time::Duration::from_millis(ms))).ok()?;
        Some(socket)
    }

    fn send_to(&mut self, socket: &std::net::UdpSocket, host: &str, port: u16, data: &[u8]) -> Result<(), Fail> {
        let address = (host, port).to_socket_addrs().map_err(|_| Fail::NoHost)?.find(|a| a.is_ipv4()).ok_or(Fail::NoHost)?;
        socket.send_to(data, address).map(|_| ()).map_err(cut)
    }

    fn receive_from(&mut self, socket: &std::net::UdpSocket, buffer: &mut [u8]) -> Result<(usize, [u8; 4], u16), Fail> {
        let (n, from) = socket.recv_from(buffer).map_err(cut)?;
        let std::net::SocketAddr::V4(from) = from else { return Err(Fail::Broken) };
        Ok((n, from.ip().octets(), from.port()))
    }

    fn close_datagram(&mut self, _: std::net::UdpSocket) {}

    /// Rust sets `SO_REUSEADDR` on Linux and macOS, as the runtime does, and not on Windows.
    fn listen(&mut self, port: u16, everyone: bool) -> Option<TcpListener> {
        TcpListener::bind((if everyone { "0.0.0.0" } else { "127.0.0.1" }, port)).ok()
    }

    fn accept(&mut self, listener: &mut TcpListener, ms: u64) -> Option<(TcpStream, [u8; 4])> {
        let (s, from) = listener.accept().ok()?;
        let limit = (ms > 0).then(|| std::time::Duration::from_millis(ms));
        s.set_read_timeout(limit).and_then(|()| s.set_write_timeout(limit)).ok()?;
        let from = match from.ip() {
            std::net::IpAddr::V4(ip) => ip.octets(),
            std::net::IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or([0; 4], |ip| ip.octets()),
        };
        Some((s, from))
    }

    /// As the runtime finds them: `getifaddrs` on Linux and macOS (the runtime asks Linux
    /// directly), the addresses of this computer's name on Windows.
    #[cfg(unix)]
    fn addresses(&mut self) -> Vec<(String, [u8; 4])> {
        #[repr(C)]
        struct IfAddrs {
            next: *mut IfAddrs,
            name: *const std::ffi::c_char,
            flags: u32,
            addr: *const u8,
        }
        unsafe extern "C" {
            fn getifaddrs(list: *mut *mut IfAddrs) -> i32;
            fn freeifaddrs(list: *mut IfAddrs);
        }
        // Where a `sockaddr` says its family (2 = IPv4): Linux's is 2 bytes first, BSD's comes
        // after a length byte.
        let family = if cfg!(target_os = "macos") { 1 } else { 0 };
        let mut list = std::ptr::null_mut();
        let mut found = vec![];
        if unsafe { getifaddrs(&mut list) } != 0 {
            return found;
        }
        let mut at = list;
        while let Some(i) = unsafe { at.as_ref() } {
            if !i.addr.is_null() && unsafe { *i.addr.add(family) } == 2 {
                let ip = unsafe { std::slice::from_raw_parts(i.addr.add(4), 4) };
                let name = unsafe { std::ffi::CStr::from_ptr(i.name) }.to_string_lossy().into_owned();
                found.push((name, [ip[0], ip[1], ip[2], ip[3]]));
            }
            at = i.next;
        }
        unsafe { freeifaddrs(list) };
        found
    }

    #[cfg(windows)]
    fn addresses(&mut self) -> Vec<(String, [u8; 4])> {
        #[link(name = "ws2_32")]
        unsafe extern "system" {
            fn gethostname(name: *mut u8, n: i32) -> i32;
        }
        let mut name = [0u8; 256];
        if unsafe { gethostname(name.as_mut_ptr(), 256) } != 0 {
            return vec![];
        }
        let name = String::from_utf8_lossy(&name[..name.iter().position(|&b| b == 0).unwrap_or(0)]).into_owned();
        let found = (name.as_str(), 0).to_socket_addrs().map(|a| a.collect::<Vec<_>>()).unwrap_or_default();
        found.into_iter().filter_map(|a| if let std::net::IpAddr::V4(ip) = a.ip() { Some((String::new(), ip.octets())) } else { None }).collect()
    }

    fn ready(&mut self, listener: &TcpListener, links: &[TcpStream]) -> Vec<bool> {
        let mut fds: Vec<Poll> = links.iter().map(raw).chain([raw(listener)]).map(|fd| Poll { fd, events: READ, revents: 0 }).collect();
        if unsafe { poll(fds.as_mut_ptr(), fds.len() as _, -1) } <= 0 {
            return vec![false; fds.len()];
        }
        // Whatever it says (data, closed, failed), `receive` tells which.
        fds.iter().map(|p| p.revents != 0).collect()
    }
}

// `poll` as each system has it: std links its library already.
#[cfg(unix)]
#[repr(C)]
struct Poll {
    fd: i32,
    events: i16,
    revents: i16,
}

#[cfg(windows)]
#[repr(C)]
struct Poll {
    fd: usize,
    events: i16,
    revents: i16,
}

#[cfg(unix)]
const READ: i16 = 1;
/// POLLRDNORM: Windows refuses POLLIN's other half.
#[cfg(windows)]
const READ: i16 = 0x100;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn poll(fds: *mut Poll, n: u64, ms: i32) -> i32;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn poll(fds: *mut Poll, n: u32, ms: i32) -> i32;
}

#[cfg(windows)]
#[link(name = "ws2_32")]
unsafe extern "system" {
    #[link_name = "WSAPoll"]
    fn poll(fds: *mut Poll, n: u32, ms: i32) -> i32;
}

#[cfg(unix)]
fn raw(s: &impl std::os::fd::AsRawFd) -> i32 {
    s.as_raw_fd()
}

#[cfg(windows)]
fn raw(s: &impl std::os::windows::io::AsRawSocket) -> usize {
    s.as_raw_socket() as usize
}

fn cut(e: std::io::Error) -> Fail {
    if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) { Fail::Timeout } else { Fail::Broken }
}
