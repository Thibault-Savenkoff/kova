// HTTPS on macOS (PLAN Part 4 "The web, step 3"): Network.framework, which makes the encrypted
// connection itself (Secure Transport, the older way, is deprecated and stops at TLS 1.2). Used by
// `kova run` (src/web.rs) and by the runtime (runtime/kova.rs), so that both fail the same way.
// Only `core` and `alloc`. Its answers come later, on a queue of its own, to "blocks" (the
// closures of Apple's C), made here by hand: this thread waits for each one with a semaphore.
// It checks the certificate (its chain, its dates and the host's name) by itself.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicI32, Ordering::SeqCst};

use crate::http::{Fail, Network};

type P = *mut c_void;

// A `Block` holds a pointer to `Shared`, which the framework never reads: it only gives the
// block back to its `invoke`.
#[link(name = "Network", kind = "framework")]
#[allow(improper_ctypes)]
unsafe extern "C" {
    fn nw_endpoint_create_host(host: *const u8, port: *const u8) -> P;
    fn nw_parameters_create_secure_tcp(tls: *const Block, tcp: *const Block) -> P;
    fn nw_connection_create(endpoint: P, parameters: P) -> P;
    fn nw_connection_set_queue(c: P, queue: P);
    fn nw_connection_set_state_changed_handler(c: P, handler: *const Block);
    fn nw_connection_start(c: P);
    fn nw_connection_send(c: P, content: P, context: P, complete: bool, done: *const Block);
    fn nw_connection_receive(c: P, least: u32, most: u32, done: *const Block);
    fn nw_connection_cancel(c: P);
    fn nw_error_get_error_domain(e: P) -> i32;
    fn nw_error_get_error_code(e: P) -> i32;
    fn nw_release(o: P);
    fn nw_tcp_options_set_connection_timeout(options: P, seconds: u32);
    static _nw_content_context_default_message: P;
}

// libSystem
#[allow(improper_ctypes)]
unsafe extern "C" {
    static _NSConcreteStackBlock: u8;
    fn dispatch_queue_create(label: *const u8, attributes: P) -> P;
    fn dispatch_semaphore_create(n: isize) -> P;
    fn dispatch_semaphore_wait(s: P, until: u64) -> isize;
    fn dispatch_time(from: u64, ns: i64) -> u64;
    fn dispatch_semaphore_signal(s: P) -> isize;
    fn dispatch_data_create(b: *const u8, n: usize, queue: P, destructor: *const Block) -> P;
    fn dispatch_data_create_map(d: P, at: *mut *const u8, n: *mut usize) -> P;
    fn dispatch_release(o: P);
}

/// A block as Apple's compilers make it, holding one pointer: `Shared`. Made on the stack: the
/// framework copies it (`Block_copy`) to keep it.
#[repr(C)]
struct Block {
    isa: *const u8,
    flags: i32,
    reserved: i32,
    invoke: *const c_void,
    descriptor: *const Descriptor,
    shared: *const Shared,
}

#[repr(C)]
struct Descriptor {
    reserved: usize,
    size: usize,
}

static DESCRIPTOR: Descriptor = Descriptor { reserved: 0, size: core::mem::size_of::<Block>() };

fn block(invoke: *const c_void, shared: *const Shared) -> Block {
    Block { isa: &raw const _NSConcreteStackBlock, flags: 0, reserved: 0, invoke, descriptor: &DESCRIPTOR, shared }
}

/// What the blocks write, from the framework's queue, for this thread.
struct Shared {
    /// Signalled when the connection is ready, waiting, failed or cancelled.
    states: P,
    /// Signalled when a send or a receive is over.
    done: P,
    state: AtomicI32,
    /// The last failure: its domain (1 POSIX, 2 DNS, 3 TLS) and code; 0 = none.
    domain: AtomicI32,
    code: AtomicI32,
    /// What the last receive got, and whether the site closed.
    got: core::cell::UnsafeCell<Vec<u8>>,
    closed: AtomicI32,
    /// See `Network::open`.
    ms: u64,
}

const WAITING: i32 = 1;
const READY: i32 = 3;
const FAILED: i32 = 4;
const CANCELLED: i32 = 5;

unsafe fn failed(s: &Shared, error: P) {
    if !error.is_null() {
        unsafe {
            s.domain.store(nw_error_get_error_domain(error), SeqCst);
            s.code.store(nw_error_get_error_code(error), SeqCst);
        }
    }
}

extern "C" fn changed(b: *const Block, state: i32, error: P) {
    let s = unsafe { &*(*b).shared };
    unsafe { failed(s, error) };
    s.state.store(state, SeqCst);
    if matches!(state, WAITING | READY | FAILED | CANCELLED) {
        unsafe { dispatch_semaphore_signal(s.states) };
    }
}

extern "C" fn sent(b: *const Block, error: P) {
    let s = unsafe { &*(*b).shared };
    unsafe { failed(s, error) };
    unsafe { dispatch_semaphore_signal(s.done) };
}

extern "C" fn received(b: *const Block, content: P, _: P, complete: bool, error: P) {
    let s = unsafe { &*(*b).shared };
    unsafe { failed(s, error) };
    if !content.is_null() {
        let (mut at, mut n) = (null(), 0);
        let map = unsafe { dispatch_data_create_map(content, &mut at, &mut n) };
        unsafe { (*s.got.get()).extend_from_slice(core::slice::from_raw_parts(at, n)) };
        unsafe { dispatch_release(map) };
    }
    if complete {
        s.closed.store(1, SeqCst);
    }
    unsafe { dispatch_semaphore_signal(s.done) };
}

/// The TLS options left as they are.
extern "C" fn as_is(_: *const Block, _: P) {}

/// The TCP options: the time limit of connecting.
extern "C" fn tcp(b: *const Block, options: P) {
    let s = unsafe { &*(*b).shared };
    if s.ms > 0 {
        unsafe { nw_tcp_options_set_connection_timeout(options, s.ms.div_ceil(1000).min(u32::MAX as u64) as u32) };
    }
}

/// The certificate failures (Security's codes, domain TLS).
const CERTIFICATE: [i32; 9] = [-9807, -9808, -9812, -9813, -9814, -9815, -9843, -67843, -67901];

impl Shared {
    fn failure(&self, connecting: bool) -> Fail {
        match (self.domain.load(SeqCst), self.code.load(SeqCst)) {
            (1, 60) => Fail::Timeout,           // ETIMEDOUT
            (1, _) if connecting => Fail::Refused, // ECONNREFUSED, EHOSTUNREACH…
            (2, _) => Fail::NoHost,
            (3, code) if CERTIFICATE.contains(&code) => Fail::BadCertificate,
            _ => Fail::Broken,
        }
    }
}

/// A `Network` whose `secure` connections are Network.framework's.
pub struct Tls<N>(pub N);

pub enum Link<L> {
    Plain(L),
    Secure(Box<Session>),
}

pub struct Session {
    connection: P,
    queue: P,
    shared: Box<Shared>,
}

/// Waits for the send or receive under way, within the time limit; false if it came first: the
/// connection is then cancelled, and its block (called all the same) waited for.
fn finished(s: &Session) -> bool {
    let ms = s.shared.ms;
    let until = if ms == 0 { u64::MAX } else { unsafe { dispatch_time(0, (ms as i64).saturating_mul(1_000_000)) } };
    if unsafe { dispatch_semaphore_wait(s.shared.done, until) } == 0 {
        return true;
    }
    unsafe {
        nw_connection_cancel(s.connection);
        dispatch_semaphore_wait(s.shared.done, u64::MAX);
    }
    false
}

impl<N: Network> Tls<N> {
    /// One try at a secure connection; `Err(None)`: the site hung up (ECONNRESET).
    fn handshake(&mut self, host: &[u8], port: &str, ms: u64) -> Result<Link<N::Link>, Option<Fail>> {
        unsafe {
            let shared = Box::new(Shared {
                states: dispatch_semaphore_create(0),
                done: dispatch_semaphore_create(0),
                state: AtomicI32::new(0),
                domain: AtomicI32::new(0),
                code: AtomicI32::new(0),
                got: core::cell::UnsafeCell::new(Vec::new()),
                closed: AtomicI32::new(0),
                ms,
            });
            let endpoint = nw_endpoint_create_host(host.as_ptr(), port.as_ptr());
            let (tls, tcp) = (block(as_is as *const c_void, null()), block(tcp as *const c_void, &*shared));
            let parameters = nw_parameters_create_secure_tcp(&tls, &tcp);
            let connection = nw_connection_create(endpoint, parameters);
            nw_release(endpoint);
            nw_release(parameters);
            let queue = dispatch_queue_create(b"kova.web\0".as_ptr(), null_mut());
            nw_connection_set_queue(connection, queue);
            let handler = block(changed as *const c_void, &*shared);
            nw_connection_set_state_changed_handler(connection, &handler);
            nw_connection_start(connection);
            let s = Box::new(Session { connection, queue, shared });
            // Waiting = it would try again later (no such site, nothing answering): a failure here.
            loop {
                dispatch_semaphore_wait(s.shared.states, u64::MAX);
                match s.shared.state.load(SeqCst) {
                    READY => return Ok(Link::Secure(s)),
                    WAITING | FAILED | CANCELLED => {
                        let reset = (s.shared.domain.load(SeqCst), s.shared.code.load(SeqCst)) == (1, 54);
                        let f = s.shared.failure(true);
                        self.close(Link::Secure(s));
                        return Err(if reset { None } else { Some(f) });
                    }
                    _ => {}
                }
            }
        }
    }
}

impl<N: Network> Network for Tls<N> {
    type Link = Link<N::Link>;

    fn open(&mut self, host: &str, port: u16, secure: bool, ms: u64) -> Result<Self::Link, Fail> {
        if !secure {
            return self.0.open(host, port, false, ms).map(Link::Plain);
        }
        if host.contains('\0') {
            return Err(Fail::NoHost);
        }
        let (host, port) = ([host.as_bytes(), &[0]].concat(), alloc::format!("{port}\0"));
        // A site that hangs up during the handshake hides why: once a certificate is refused, the
        // site's reset often arrives first, and the framework then reports it alone (seen on
        // badssl.com, one try in three). Nothing was sent yet, so asking again is safe.
        let mut tries = 0;
        loop {
            let opened = self.handshake(&host, &port, ms);
            let reset = matches!(&opened, Err(None));
            tries += 1;
            if !reset || tries == 5 {
                return opened.map_err(|f| f.unwrap_or(Fail::Refused));
            }
        }
    }

    fn send(&mut self, link: &mut Self::Link, data: &[u8]) -> Result<(), Fail> {
        let s = match link {
            Link::Plain(raw) => return self.0.send(raw, data),
            Link::Secure(s) => s,
        };
        unsafe {
            // Copied by the framework (no destructor given).
            let content = dispatch_data_create(data.as_ptr(), data.len(), null_mut(), null());
            s.shared.domain.store(0, SeqCst);
        let done = block(sent as *const c_void, &*s.shared);
            nw_connection_send(s.connection, content, _nw_content_context_default_message, true, &done);
            let late = !finished(s);
            dispatch_release(content);
            if late {
                return Err(Fail::Timeout);
            }
        }
        if s.shared.domain.load(SeqCst) != 0 { Err(s.shared.failure(false)) } else { Ok(()) }
    }

    fn receive(&mut self, link: &mut Self::Link, buffer: &mut [u8]) -> Result<usize, Fail> {
        let s = match link {
            Link::Plain(raw) => return self.0.receive(raw, buffer),
            Link::Secure(s) => s,
        };
        if s.shared.closed.load(SeqCst) != 0 {
            return Ok(0);
        }
        s.shared.domain.store(0, SeqCst);
        let done = block(received as *const c_void, &*s.shared);
        unsafe {
            nw_connection_receive(s.connection, 1, buffer.len().min(1 << 30) as u32, &done);
            if !finished(s) {
                return Err(Fail::Timeout);
            }
        }
        let got = unsafe { &mut *s.shared.got.get() };
        let n = got.len();
        buffer[..n].copy_from_slice(got);
        got.clear();
        if n == 0 && s.shared.closed.load(SeqCst) == 0 && s.shared.domain.load(SeqCst) != 0 {
            return Err(s.shared.failure(false));
        }
        Ok(n)
    }

    fn close(&mut self, link: Self::Link) {
        match link {
            Link::Plain(raw) => self.0.close(raw),
            Link::Secure(s) => unsafe {
                // Its last answer, "cancelled", tells that no block will be called any more.
                nw_connection_cancel(s.connection);
                while s.shared.state.load(SeqCst) != CANCELLED {
                    dispatch_semaphore_wait(s.shared.states, u64::MAX);
                }
                nw_release(s.connection);
                dispatch_release(s.queue);
                dispatch_release(s.shared.states);
                dispatch_release(s.shared.done);
            },
        }
    }
}
