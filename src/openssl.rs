// HTTPS on Linux (PLAN Part 4 "The web, step 3"): the system's OpenSSL around the connection of
// any `Network`. Used by `kova run` (src/web.rs) and by the runtime (runtime/kova.rs), so that both
// fail the same way. Only `core` and `alloc`. OpenSSL is loaded at the first `https://`
// (`dlopen`): neither `kova` nor a program using only `http://` needs it. Its bytes go through two
// memory BIOs, which this file carries to the connection and back.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::null_mut;

use crate::http::{Fail, Network};

unsafe extern "C" {
    fn dlopen(name: *const u8, flags: i32) -> *mut c_void;
    fn dlsym(library: *mut c_void, name: *const u8) -> *mut c_void;
}

type P = *mut c_void;

/// The functions of OpenSSL this file calls, found by name.
struct Ssl {
    method: extern "C" fn() -> P,
    ctx_new: extern "C" fn(P) -> P,
    ctx_free: extern "C" fn(P),
    default_paths: extern "C" fn(P) -> i32,
    set_verify: extern "C" fn(P, i32, P),
    new: extern "C" fn(P) -> P,
    free: extern "C" fn(P),
    set1_host: extern "C" fn(P, *const u8) -> i32,
    ctrl: extern "C" fn(P, i32, i64, *const u8) -> i64,
    set_bio: extern "C" fn(P, P, P),
    connect_state: extern "C" fn(P),
    handshake: extern "C" fn(P) -> i32,
    read: extern "C" fn(P, *mut u8, i32) -> i32,
    write: extern "C" fn(P, *const u8, i32) -> i32,
    error: extern "C" fn(P, i32) -> i32,
    verify_result: extern "C" fn(P) -> i64,
    bio_memory: extern "C" fn() -> P,
    bio_new: extern "C" fn(P) -> P,
    bio_read: extern "C" fn(P, *mut u8, i32) -> i32,
    bio_write: extern "C" fn(P, *const u8, i32) -> i32,
    clear_errors: extern "C" fn(),
}

impl Ssl {
    /// OpenSSL 3, else 1.1: None when this system has neither.
    fn load() -> Option<Ssl> {
        let library = [&b"libssl.so.3\0"[..], b"libssl.so.1.1\0"].iter().map(|n| unsafe { dlopen(n.as_ptr(), 2) }).find(|l| !l.is_null())?;
        macro_rules! find {
            ($name:literal) => {{
                let f = unsafe { dlsym(library, concat!($name, "\0").as_ptr()) };
                if f.is_null() {
                    return None;
                }
                unsafe { core::mem::transmute::<P, _>(f) }
            }};
        }
        Some(Ssl {
            method: find!("TLS_client_method"),
            ctx_new: find!("SSL_CTX_new"),
            ctx_free: find!("SSL_CTX_free"),
            default_paths: find!("SSL_CTX_set_default_verify_paths"),
            set_verify: find!("SSL_CTX_set_verify"),
            new: find!("SSL_new"),
            free: find!("SSL_free"),
            set1_host: find!("SSL_set1_host"),
            ctrl: find!("SSL_ctrl"),
            set_bio: find!("SSL_set_bio"),
            connect_state: find!("SSL_set_connect_state"),
            handshake: find!("SSL_do_handshake"),
            read: find!("SSL_read"),
            write: find!("SSL_write"),
            error: find!("SSL_get_error"),
            verify_result: find!("SSL_get_verify_result"),
            bio_memory: find!("BIO_s_mem"),
            bio_new: find!("BIO_new"),
            bio_read: find!("BIO_read"),
            bio_write: find!("BIO_write"),
            clear_errors: find!("ERR_clear_error"),
        })
    }
}

const WANT_READ: i32 = 2;
const ZERO_RETURN: i32 = 6;

/// A `Network` whose `secure` connections go through OpenSSL.
pub struct Tls<N>(pub N);

pub enum Link<L> {
    Plain(L),
    Secure(Box<Session<L>>),
}

pub struct Session<L> {
    raw: L,
    ssl: Ssl,
    ctx: P,
    session: P,
    /// What OpenSSL reads from (the site's bytes) and writes to (the bytes for the site).
    from_site: P,
    to_site: P,
}

impl<N: Network> Network for Tls<N> {
    type Link = Link<N::Link>;

    fn open(&mut self, host: &str, port: u16, secure: bool, ms: u64) -> Result<Self::Link, Fail> {
        if !secure {
            return self.0.open(host, port, false, ms).map(Link::Plain);
        }
        let ssl = Ssl::load().ok_or(Fail::NoHttps)?;
        let raw = self.0.open(host, port, false, ms)?;
        let ctx = (ssl.ctx_new)((ssl.method)());
        let session = if ctx.is_null() {
            null_mut()
        } else {
            // Before `SSL_new`, which copies them: the system's authorities, and SSL_VERIFY_PEER
            // (a certificate that cannot be trusted stops the greetings).
            (ssl.default_paths)(ctx);
            (ssl.set_verify)(ctx, 1, null_mut());
            (ssl.new)(ctx)
        };
        let mut s = Box::new(Session { raw, ssl, ctx, session, from_site: null_mut(), to_site: null_mut() });
        match handshake(&mut self.0, &mut s, host) {
            Ok(()) => Ok(Link::Secure(s)),
            Err(f) => {
                self.close(Link::Secure(s));
                Err(f)
            }
        }
    }

    fn send(&mut self, link: &mut Self::Link, data: &[u8]) -> Result<(), Fail> {
        let s = match link {
            Link::Plain(raw) => return self.0.send(raw, data),
            Link::Secure(s) => s,
        };
        for piece in data.chunks(1 << 30) {
            (s.ssl.clear_errors)();
            // A memory BIO takes everything at once.
            if (s.ssl.write)(s.session, piece.as_ptr(), piece.len() as i32) <= 0 {
                return Err(Fail::Broken);
            }
            flush(&mut self.0, s)?;
        }
        Ok(())
    }

    fn receive(&mut self, link: &mut Self::Link, buffer: &mut [u8]) -> Result<usize, Fail> {
        let s = match link {
            Link::Plain(raw) => return self.0.receive(raw, buffer),
            Link::Secure(s) => s,
        };
        loop {
            (s.ssl.clear_errors)();
            let n = (s.ssl.read)(s.session, buffer.as_mut_ptr(), buffer.len().min(1 << 30) as i32);
            flush(&mut self.0, s)?;
            if n > 0 {
                return Ok(n as usize);
            }
            match (s.ssl.error)(s.session, n) {
                WANT_READ => {
                    if !more(&mut self.0, s)? {
                        return Ok(0);
                    }
                }
                // The site said it closes ("close_notify").
                ZERO_RETURN => return Ok(0),
                _ => return Err(Fail::Broken),
            }
        }
    }

    fn close(&mut self, link: Self::Link) {
        match link {
            Link::Plain(raw) => self.0.close(raw),
            Link::Secure(s) => {
                // The session frees its two BIOs.
                if !s.session.is_null() {
                    (s.ssl.free)(s.session);
                }
                if !s.ctx.is_null() {
                    (s.ssl.ctx_free)(s.ctx);
                }
                self.0.close(s.raw);
            }
        }
    }
}

/// Sends what OpenSSL wrote for the site.
fn flush<N: Network>(net: &mut N, s: &mut Session<N::Link>) -> Result<(), Fail> {
    let mut buffer = [0u8; 16384];
    loop {
        let n = (s.ssl.bio_read)(s.to_site, buffer.as_mut_ptr(), buffer.len() as i32);
        if n <= 0 {
            return Ok(());
        }
        net.send(&mut s.raw, &buffer[..n as usize])?;
    }
}

/// More bytes from the site, given to OpenSSL; false when it closed.
fn more<N: Network>(net: &mut N, s: &mut Session<N::Link>) -> Result<bool, Fail> {
    let mut buffer = [0u8; 16384];
    let n = net.receive(&mut s.raw, &mut buffer)?;
    if n > 0 {
        (s.ssl.bio_write)(s.from_site, buffer.as_ptr(), n as i32);
    }
    Ok(n > 0)
}

/// The greetings, checking the certificate: its chain to the system's authorities, its dates
/// and the host's name.
fn handshake<N: Network>(net: &mut N, s: &mut Session<N::Link>, host: &str) -> Result<(), Fail> {
    if s.session.is_null() || host.contains('\0') {
        return Err(Fail::NoHttps);
    }
    let ssl = &s.ssl;
    let name: Vec<u8> = host.bytes().chain([0]).collect();
    // The name the certificate must have, and the one told to the site (SNI: SSL_CTRL_SET_TLSEXT_HOSTNAME).
    if (ssl.set1_host)(s.session, name.as_ptr()) != 1 {
        return Err(Fail::BadUrl);
    }
    (ssl.ctrl)(s.session, 55, 0, name.as_ptr());
    s.from_site = (ssl.bio_new)((ssl.bio_memory)());
    s.to_site = (ssl.bio_new)((ssl.bio_memory)());
    (ssl.set_bio)(s.session, s.from_site, s.to_site);
    (ssl.connect_state)(s.session);
    loop {
        (s.ssl.clear_errors)();
        let r = (s.ssl.handshake)(s.session);
        flush(net, s)?;
        if r == 1 {
            return Ok(());
        }
        if (s.ssl.error)(s.session, r) != WANT_READ {
            return Err(if (s.ssl.verify_result)(s.session) != 0 { Fail::BadCertificate } else { Fail::Broken });
        }
        if !more(net, s)? {
            return Err(Fail::Broken);
        }
    }
}
