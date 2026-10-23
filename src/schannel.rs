// HTTPS on Windows (PLAN Part 4 "The web, step 3"): SChannel, the encryption of Windows itself
// (`secur32.dll`), around the connection of any `Network`. Used by `kova run` (src/web.rs) and by
// the runtime (runtime/kova.rs), so that both fail the same way. Only `core` and `alloc`.
// SChannel checks the certificate (its chain, its dates and the host's name) by itself.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr::{null, null_mut};

use crate::http::{Fail, Network};

#[repr(C)]
#[derive(Clone, Copy)]
struct Handle(usize, usize);

#[repr(C)]
struct Buffer {
    size: u32,
    kind: u32,
    data: *mut u8,
}

#[repr(C)]
struct Buffers {
    version: u32,
    count: u32,
    buffers: *mut Buffer,
}

/// `SCHANNEL_CRED`: nothing but the version and the flags.
#[repr(C)]
struct Credentials {
    version: u32,
    creds: u32,
    cred: *const u8,
    root_store: *const u8,
    mappers: u32,
    mapper: *const u8,
    algorithms: u32,
    algorithm: *const u32,
    protocols: u32,
    minimum: u32,
    maximum: u32,
    lifespan: u32,
    flags: u32,
    format: u32,
}

#[link(name = "secur32")]
unsafe extern "system" {
    fn AcquireCredentialsHandleW(principal: *const u16, package: *const u16, usage: u32, logon: *const u8, data: *const Credentials, f: *const u8, a: *const u8, cred: *mut Handle, expiry: *mut i64) -> i32;
    fn InitializeSecurityContextW(cred: *const Handle, ctx: *const Handle, target: *const u16, wanted: u32, r1: u32, rep: u32, input: *const Buffers, r2: u32, new: *mut Handle, output: *mut Buffers, got: *mut u32, expiry: *mut i64) -> i32;
    fn QueryContextAttributesW(ctx: *const Handle, what: u32, into: *mut u32) -> i32;
    fn EncryptMessage(ctx: *const Handle, quality: u32, message: *mut Buffers, sequence: u32) -> i32;
    fn DecryptMessage(ctx: *const Handle, message: *mut Buffers, sequence: u32, quality: *mut u32) -> i32;
    fn FreeContextBuffer(b: *mut u8) -> i32;
    fn DeleteSecurityContext(ctx: *const Handle) -> i32;
    fn FreeCredentialsHandle(cred: *const Handle) -> i32;
}

const EMPTY: u32 = 0;
const DATA: u32 = 1;
const TOKEN: u32 = 2;
const EXTRA: u32 = 5;
const TRAILER: u32 = 6;
const HEADER: u32 = 7;

const OK: i32 = 0;
const CONTINUE: i32 = 0x0009_0312;
const EXPIRED: i32 = 0x0009_0317;
const INCOMPLETE: i32 = 0x8009_0318_u32 as i32;

/// The failures that mean "this certificate cannot be trusted".
const CERTIFICATE: [u32; 10] = [
    0x8009_0322, // SEC_E_WRONG_PRINCIPAL
    0x8009_0325, // SEC_E_UNTRUSTED_ROOT
    0x8009_0327, // SEC_E_CERT_UNKNOWN
    0x8009_0328, // SEC_E_CERT_EXPIRED
    0x800B_0101, // CERT_E_EXPIRED
    0x800B_0109, // CERT_E_UNTRUSTEDROOT
    0x800B_010A, // CERT_E_CHAINING
    0x800B_010F, // CERT_E_CN_NO_MATCH
    0x800B_0111, // CERT_E_WRONG_USAGE
    0x8009_2010, // CRYPT_E_REVOKED
];

fn failure(status: i32) -> Fail {
    if CERTIFICATE.contains(&(status as u32)) { Fail::BadCertificate } else { Fail::Broken }
}

/// A `Network` whose `secure` connections go through SChannel.
pub struct Tls<N>(pub N);

pub enum Link<L> {
    Plain(L),
    Secure(Box<Session<L>>),
}

pub struct Session<L> {
    raw: L,
    cred: Handle,
    ctx: Handle,
    /// `SecPkgContext_StreamSizes`: header, trailer, largest message, buffers, block.
    sizes: [u32; 5],
    /// Encrypted bytes received and not yet decrypted.
    incoming: Vec<u8>,
    /// Decrypted bytes not yet given.
    plain: Vec<u8>,
}

impl<N: Network> Network for Tls<N> {
    type Link = Link<N::Link>;

    fn open(&mut self, host: &str, port: u16, secure: bool, ms: u64) -> Result<Self::Link, Fail> {
        let raw = self.0.open(host, port, false, ms)?;
        if !secure {
            return Ok(Link::Plain(raw));
        }
        let mut s = Box::new(Session { raw, cred: Handle(0, 0), ctx: Handle(0, 0), sizes: [0; 5], incoming: Vec::new(), plain: Vec::new() });
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
        let [header, trailer, largest, ..] = s.sizes.map(|n| n as usize);
        for piece in data.chunks(largest.max(1)) {
            let mut message = Vec::with_capacity(header + piece.len() + trailer);
            message.resize(header, 0);
            message.extend_from_slice(piece);
            message.resize(header + piece.len() + trailer, 0);
            let at = message.as_mut_ptr();
            let mut b = [
                Buffer { size: header as u32, kind: HEADER, data: at },
                Buffer { size: piece.len() as u32, kind: DATA, data: unsafe { at.add(header) } },
                Buffer { size: trailer as u32, kind: TRAILER, data: unsafe { at.add(header + piece.len()) } },
                Buffer { size: 0, kind: EMPTY, data: null_mut() },
            ];
            let mut d = Buffers { version: 0, count: 4, buffers: b.as_mut_ptr() };
            let status = unsafe { EncryptMessage(&s.ctx, 0, &mut d, 0) };
            if status != OK {
                return Err(failure(status));
            }
            let n = (b[0].size + b[1].size + b[2].size) as usize;
            self.0.send(&mut s.raw, &message[..n])?;
        }
        Ok(())
    }

    fn receive(&mut self, link: &mut Self::Link, buffer: &mut [u8]) -> Result<usize, Fail> {
        let s = match link {
            Link::Plain(raw) => return self.0.receive(raw, buffer),
            Link::Secure(s) => s,
        };
        while s.plain.is_empty() {
            if s.incoming.is_empty() && !more(&mut self.0, s)? {
                return Ok(0);
            }
            let mut b = [
                Buffer { size: s.incoming.len() as u32, kind: DATA, data: s.incoming.as_mut_ptr() },
                Buffer { size: 0, kind: EMPTY, data: null_mut() },
                Buffer { size: 0, kind: EMPTY, data: null_mut() },
                Buffer { size: 0, kind: EMPTY, data: null_mut() },
            ];
            let mut d = Buffers { version: 0, count: 4, buffers: b.as_mut_ptr() };
            match unsafe { DecryptMessage(&s.ctx, &mut d, 0, null_mut()) } {
                OK => {
                    // Both point inside `incoming`, decrypted where it was.
                    let piece = |kind| b.iter().find(|b| b.kind == kind).map(|b| unsafe { core::slice::from_raw_parts(b.data, b.size as usize) }.to_vec());
                    s.plain = piece(DATA).unwrap_or_default();
                    s.incoming = piece(EXTRA).unwrap_or_default();
                }
                INCOMPLETE => {
                    if !more(&mut self.0, s)? {
                        return Err(Fail::Broken);
                    }
                }
                // The site said it closes ("close_notify").
                EXPIRED => return Ok(0),
                // ponytail: no renegotiation (SEC_I_RENEGOTIATE), add it if a site needs it.
                status => return Err(failure(status)),
            }
        }
        let n = buffer.len().min(s.plain.len());
        buffer[..n].copy_from_slice(&s.plain[..n]);
        s.plain.drain(..n);
        Ok(n)
    }

    fn close(&mut self, link: Self::Link) {
        match link {
            Link::Plain(raw) => self.0.close(raw),
            Link::Secure(s) => {
                unsafe {
                    if s.ctx.0 != 0 || s.ctx.1 != 0 {
                        DeleteSecurityContext(&s.ctx);
                    }
                    if s.cred.0 != 0 || s.cred.1 != 0 {
                        FreeCredentialsHandle(&s.cred);
                    }
                }
                self.0.close(s.raw);
            }
        }
    }
}

/// More encrypted bytes from the site, after those already there; false when it closed.
fn more<N: Network>(net: &mut N, s: &mut Session<N::Link>) -> Result<bool, Fail> {
    let mut buffer = [0u8; 16384];
    let n = net.receive(&mut s.raw, &mut buffer)?;
    s.incoming.extend_from_slice(&buffer[..n]);
    Ok(n > 0)
}

/// The greetings: each side's messages until SChannel says the connection is encrypted.
fn handshake<N: Network>(net: &mut N, s: &mut Session<N::Link>, host: &str) -> Result<(), Fail> {
    let wide = |t: &str| -> Vec<u16> { t.encode_utf16().chain([0]).collect() };
    // `SCH_CRED_AUTO_CRED_VALIDATION`, `SCH_CRED_NO_DEFAULT_CREDS`, `SCH_USE_STRONG_CRYPTO`.
    let c = Credentials { version: 4, creds: 0, cred: null(), root_store: null(), mappers: 0, mapper: null(), algorithms: 0, algorithm: null(), protocols: 0, minimum: 0, maximum: 0, lifespan: 0, flags: 0x20 | 0x10 | 0x40_0000, format: 0 };
    let mut expiry = 0;
    let status = unsafe { AcquireCredentialsHandleW(null(), wide("Microsoft Unified Security Protocol Provider").as_ptr(), 2, null(), &c, null(), null(), &mut s.cred, &mut expiry) };
    if status != OK {
        return Err(Fail::NoHttps);
    }
    let target = wide(host);
    // Sequence and replay detection, confidentiality, memory given by SChannel, a stream.
    let wanted = 0x8 | 0x4 | 0x10 | 0x100 | 0x8000;
    let mut first = true;
    loop {
        let mut input = [
            Buffer { size: s.incoming.len() as u32, kind: TOKEN, data: s.incoming.as_mut_ptr() },
            Buffer { size: 0, kind: EMPTY, data: null_mut() },
        ];
        let input_d = Buffers { version: 0, count: 2, buffers: input.as_mut_ptr() };
        let mut output = [Buffer { size: 0, kind: TOKEN, data: null_mut() }];
        let mut output_d = Buffers { version: 0, count: 1, buffers: output.as_mut_ptr() };
        let mut got = 0;
        let ctx: *const Handle = if first { null() } else { &s.ctx };
        let status = unsafe { InitializeSecurityContextW(&s.cred, ctx, target.as_ptr(), wanted, 0, 0, if first { null() } else { &input_d }, 0, &mut s.ctx, &mut output_d, &mut got, &mut expiry) };
        first = false;
        if !output[0].data.is_null() {
            let token = unsafe { core::slice::from_raw_parts(output[0].data, output[0].size as usize) }.to_vec();
            unsafe { FreeContextBuffer(output[0].data) };
            net.send(&mut s.raw, &token)?;
        }
        match status {
            OK | CONTINUE => {
                // What SChannel did not read yet stays, at the end of `incoming`.
                let extra = if input[1].kind == EXTRA { input[1].size as usize } else { 0 };
                s.incoming.drain(..s.incoming.len() - extra);
                if status == OK {
                    break;
                }
                if extra == 0 && !more(net, s)? {
                    return Err(Fail::Broken);
                }
            }
            INCOMPLETE => {
                if !more(net, s)? {
                    return Err(Fail::Broken);
                }
            }
            status => return Err(failure(status)),
        }
    }
    // SECPKG_ATTR_STREAM_SIZES
    let status = unsafe { QueryContextAttributesW(&s.ctx, 4, s.sizes.as_mut_ptr()) };
    if status != OK { Err(failure(status)) } else { Ok(()) }
}
