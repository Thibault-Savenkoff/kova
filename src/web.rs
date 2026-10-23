// The network of `kova run` (PLAN Part 4 "The web"): Rust's own, which asks the system as the
// runtime does (`getaddrinfo`, then a connection), so that the failures are the same.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

use crate::http::{Fail, Network};

pub struct System;

/// What `kova run` connects with: the system, encrypted by SChannel on Windows.
#[cfg(windows)]
pub fn network() -> crate::schannel::Tls<System> {
    crate::schannel::Tls(System)
}

#[cfg(not(windows))]
pub fn network() -> System {
    System
}

impl Network for System {
    type Link = TcpStream;

    fn open(&mut self, host: &str, port: u16, secure: bool) -> Result<TcpStream, Fail> {
        // ponytail: HTTPS on Linux and macOS comes next (PLAN "The web, step 3").
        if secure && cfg!(not(windows)) {
            return Err(Fail::BadUrl);
        }
        let addresses: Vec<_> = (host, port).to_socket_addrs().map_err(|_| Fail::NoHost)?.collect();
        let mut failure = Fail::NoHost;
        for a in addresses {
            match TcpStream::connect(a) {
                Ok(s) => return Ok(s),
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
