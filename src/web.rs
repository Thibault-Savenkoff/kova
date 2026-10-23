// The network of `kova run` (PLAN Part 4 "The web"): Rust's own, which asks the system as the
// runtime does (`getaddrinfo`, then a connection), so that the failures are the same.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

use crate::http::{Fail, Network};

pub struct System;

/// What `kova run` connects with: the system, encrypted by SChannel on Windows, OpenSSL on Linux,
/// Network.framework on macOS.
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

    fn close(&mut self, _: TcpStream) {}
}

fn cut(e: std::io::Error) -> Fail {
    if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) { Fail::Timeout } else { Fail::Broken }
}
