// HTTP/1.1 for `get` and `post` (PLAN Part 4 "The web"), used by `kova run` (src/interp.rs) and by
// the runtime of `kova build` (runtime/), so that both send and read exactly the same bytes. Only
// `core` and `alloc`: the runtime has no `std`. The network itself (finding the address,
// connecting, encrypting) is each system's, behind `Network`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Why there is no answer: the variants of `WebError`, in its order (`types::web_error`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// Not an address Kova can follow (`http://` or `https://`, then a host).
    BadUrl,
    /// No machine has that name.
    NoHost,
    /// The machine is there, but nothing answers on that port.
    Refused,
    /// No answer in time.
    Timeout,
    /// The connection was cut before the answer was complete.
    Broken,
    /// What came back is not HTTP.
    BadAnswer,
    /// More than 10 redirections.
    TooManyRedirects,
    /// The site's certificate cannot be trusted: expired, self-signed, made for another name.
    BadCertificate,
    /// This system cannot encrypt (a Linux without OpenSSL).
    // macOS always has Network.framework.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NoHttps,
    /// A header Kova cannot send: a line break in it, or one Kova writes itself (`Host`…).
    BadHeader,
}

impl Fail {
    /// Its number: `WebError`'s variant.
    pub fn number(self) -> i64 {
        self as i64
    }
}

/// An address, cut into its pieces.
#[derive(Debug, PartialEq)]
pub struct Url {
    pub secure: bool,
    /// For the system: `example.com`, `::1` (without its brackets).
    pub host: String,
    pub port: u16,
    /// What follows the host: `/`, `/search?q=kova` (the `#…` part stays in the browser).
    pub path: String,
    /// For the `Host` line: `example.com`, `[::1]:8080` (the port only when it is not the usual one).
    authority: String,
}

pub fn parse_url(url: &str) -> Option<Url> {
    let (secure, rest) = if let Some(r) = strip_scheme(url, "http://") {
        (false, r)
    } else {
        (true, strip_scheme(url, "https://")?)
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    // `user:password@` is refused: it would go in clear inside the address.
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(r) => {
            let close = r.find(']')?;
            let after = &r[close + 1..];
            (&r[..close], if after.is_empty() { None } else { Some(after.strip_prefix(':')?) })
        }
        None => match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        },
    };
    let usual = if secure { 443 } else { 80 };
    let port = match port {
        Some(p) => p.parse().ok().filter(|&p| p != 0)?,
        None => usual,
    };
    if host.is_empty() || host.bytes().any(|b| b <= b' ' || b == 0x7F) {
        return None;
    }
    let path = path.split('#').next().unwrap_or("");
    let mut p = String::new();
    if !path.starts_with('/') {
        p.push('/');
    }
    // Spaces and other bytes HTTP does not allow there are written `%20`…
    for b in path.bytes() {
        if b <= b' ' || b >= 0x7F {
            p += &format!("%{b:02X}");
        } else {
            p.push(b as char);
        }
    }
    let shown = if authority.starts_with('[') { format!("[{host}]") } else { host.to_string() };
    let authority = if port == usual { shown } else { format!("{shown}:{port}") };
    Some(Url { secure, host: host.to_string(), port, path: p, authority })
}

/// `url` without `scheme`, whatever the capitals (`HTTP://`).
fn strip_scheme<'a>(url: &'a str, scheme: &str) -> Option<&'a str> {
    let n = scheme.len();
    (url.len() >= n && url.is_char_boundary(n) && url[..n].eq_ignore_ascii_case(scheme)).then(|| &url[n..])
}

/// What a system gives to talk to a machine.
pub trait Network {
    type Link;
    /// Finds `host`, connects to its `port`, encrypted if `secure`. `ms`: the time limit of
    /// connecting and then of each `send` and `receive`, in milliseconds (0 = none).
    fn open(&mut self, host: &str, port: u16, secure: bool, ms: u64) -> Result<Self::Link, Fail>;
    fn send(&mut self, link: &mut Self::Link, data: &[u8]) -> Result<(), Fail>;
    /// Some bytes of the answer; 0 when the machine closed the connection.
    fn receive(&mut self, link: &mut Self::Link, buffer: &mut [u8]) -> Result<usize, Fail>;
    fn close(&mut self, link: Self::Link);
}

/// A site's answer: a `Response` in Kova.
#[derive(Debug, PartialEq)]
pub struct Answer {
    pub status: i64,
    /// Where it came from, after the redirections.
    pub url: String,
    pub body: String,
    /// The same body as it came: `body` is its text, with `�` where it was not one.
    pub data: Vec<u8>,
    /// The headers, names in small letters, in their order; one name given twice is one entry,
    /// its values joined by `, ` (as HTTP allows).
    pub headers: Vec<(String, String)>,
}

/// What to ask: a `Request` in Kova.
pub struct Request<'a> {
    /// `GET`, `POST`…
    pub method: &'a str,
    pub url: &'a str,
    /// Sent when it is not empty.
    pub body: &'a [u8],
    /// The program's own headers; they replace Kova's of the same name (`User-Agent`…).
    pub headers: Vec<(&'a str, &'a str)>,
    /// Whether the redirections are followed.
    pub follow: bool,
    /// See `Network::open`.
    pub ms: u64,
}

/// A time limit in seconds (`Request.seconds`) as milliseconds, rounded up; 0 (none) when it
/// is not above 0.
pub fn milliseconds(seconds: f64) -> u64 {
    let ms = seconds * 1000.0;
    if ms.is_nan() || ms <= 0.0 {
        return 0;
    }
    // `as` stops at the largest u64; `ceil` is not in `core`.
    let whole = ms as u64;
    if (whole as f64) < ms { whole.saturating_add(1) } else { whole }
}

/// The headers Kova writes itself, which say where the answer starts and ends.
const OWN: [&str; 4] = ["host", "content-length", "connection", "transfer-encoding"];

/// What HTTP allows in a header's name ("token").
fn token(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Asks `r.url` and its redirections.
pub fn exchange<N: Network>(net: &mut N, r: &Request) -> Result<Answer, Fail> {
    // A line break in a header would let it write other headers.
    if r.headers.iter().any(|(n, v)| !token(n) || OWN.iter().any(|o| n.eq_ignore_ascii_case(o)) || v.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0))) {
        return Err(Fail::BadHeader);
    }
    let mut url = r.url.to_string();
    let mut method = r.method.to_string();
    let mut body = r.body.to_vec();
    for _ in 0..=10 {
        let answer = once(net, r, &method, &url, &body)?;
        let to = answer.headers.iter().find(|(n, _)| n == "location").map(|(_, v)| v.clone());
        let Some(to) = to.filter(|_| r.follow && matches!(answer.status, 301 | 302 | 303 | 307 | 308)) else {
            return Ok(answer);
        };
        url = resolve(&url, &to).ok_or(Fail::BadAnswer)?;
        // As browsers do: after a 303, or a 301 / 302 answering a POST, the next one is a GET.
        if answer.status == 303 || (matches!(answer.status, 301 | 302) && method == "POST") {
            method = "GET".into();
            body.clear();
        }
    }
    Err(Fail::TooManyRedirects)
}

/// One request and its answer.
fn once<N: Network>(net: &mut N, r: &Request, method: &str, url: &str, body: &[u8]) -> Result<Answer, Fail> {
    let u = parse_url(url).ok_or(Fail::BadUrl)?;
    let theirs = |name: &str| r.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name));
    let mut request = format!("{method} {} HTTP/1.1\r\nHost: {}\r\n", u.path, u.authority);
    for (name, value) in [("User-Agent", "Kova"), ("Accept", "*/*"), ("Connection", "close")] {
        if !theirs(name) {
            request += &format!("{name}: {value}\r\n");
        }
    }
    if !body.is_empty() || matches!(method, "POST" | "PUT" | "PATCH") {
        if !theirs("Content-Type") {
            request += "Content-Type: text/plain; charset=utf-8\r\n";
        }
        request += &format!("Content-Length: {}\r\n", body.len());
    }
    for (name, value) in &r.headers {
        request += &format!("{name}: {value}\r\n");
    }
    request += "\r\n";
    let request = [request.as_bytes(), body].concat();
    let mut link = net.open(&u.host, u.port, u.secure, r.ms)?;
    let read = net.send(&mut link, &request).and_then(|()| read_answer(net, &mut link, method == "HEAD"));
    net.close(link);
    let (status, headers, body) = read?;
    Ok(Answer { status, url: url.to_string(), body: String::from_utf8_lossy(&body).into_owned(), data: body, headers })
}

/// Reads until the answer is complete (its length, its last piece, or the end of the connection).
fn read_answer<N: Network>(net: &mut N, link: &mut N::Link, head: bool) -> Result<(i64, Vec<(String, String)>, Vec<u8>), Fail> {
    let mut got = Vec::new();
    let mut buffer = [0u8; 16384];
    loop {
        let n = net.receive(link, &mut buffer)?;
        got.extend_from_slice(&buffer[..n]);
        match parse_answer(&got, head, n == 0)? {
            Some(answer) => return Ok(answer),
            None if n == 0 => return Err(Fail::Broken),
            None => {}
        }
    }
}

/// The answer in `got`, or `None` if more must come; `closed`: nothing more will.
fn parse_answer(got: &[u8], head: bool, closed: bool) -> Result<Option<(i64, Vec<(String, String)>, Vec<u8>)>, Fail> {
    let mut start = 0;
    loop {
        let Some(end) = find(&got[start..], b"\r\n\r\n").map(|e| start + e) else {
            return if got.len() - start > 65536 { Err(Fail::BadAnswer) } else { Ok(None) };
        };
        let text = core::str::from_utf8(&got[start..end]).map_err(|_| Fail::BadAnswer)?;
        let mut lines = text.split("\r\n");
        let status_line = lines.next().unwrap_or("");
        let mut words = status_line.splitn(3, ' ');
        if !words.next().is_some_and(|w| w.starts_with("HTTP/1.")) {
            return Err(Fail::BadAnswer);
        }
        let status: i64 = words.next().and_then(|w| w.parse().ok()).filter(|s| (100..1000).contains(s)).ok_or(Fail::BadAnswer)?;
        let after = end + 4;
        // "100 Continue" and the others below 200 announce the real answer.
        if status < 200 {
            start = after;
            continue;
        }
        let mut headers: Vec<(String, String)> = Vec::new();
        for line in lines {
            let (name, value) = line.split_once(':').ok_or(Fail::BadAnswer)?;
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
            match headers.iter_mut().find(|(n, _)| *n == name) {
                Some((_, v)) => {
                    *v += ", ";
                    *v += value;
                }
                None => headers.push((name, value.to_string())),
            }
        }
        let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        let rest = &got[after..];
        if head || status == 204 || status == 304 {
            return Ok(Some((status, headers, Vec::new())));
        }
        if header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
            return match chunks(rest)? {
                Some(body) => Ok(Some((status, headers, body))),
                None => Ok(None),
            };
        }
        if let Some(length) = header("content-length") {
            let length: usize = length.parse().map_err(|_| Fail::BadAnswer)?;
            return Ok(if rest.len() >= length { Some((status, headers, rest[..length].to_vec())) } else { None });
        }
        // Neither: the body ends with the connection.
        return Ok(if closed { Some((status, headers, rest.to_vec())) } else { None });
    }
}

/// A body sent in pieces ("chunked"): each piece's size in hexadecimal, its bytes, and a last
/// piece of size 0. `None` until the last one came.
fn chunks(mut rest: &[u8]) -> Result<Option<Vec<u8>>, Fail> {
    let mut body = Vec::new();
    loop {
        let Some(end) = find(rest, b"\r\n") else { return Ok(None) };
        let line = core::str::from_utf8(&rest[..end]).map_err(|_| Fail::BadAnswer)?;
        let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).map_err(|_| Fail::BadAnswer)?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Ok(Some(body));
        }
        if rest.len() < size + 2 {
            return Ok(None);
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Where a redirection goes: `to` read from `from`.
fn resolve(from: &str, to: &str) -> Option<String> {
    if to.contains("://") {
        return Some(to.to_string());
    }
    let u = parse_url(from)?;
    let scheme = if u.secure { "https:" } else { "http:" };
    if to.starts_with("//") {
        return Some(format!("{scheme}{to}"));
    }
    let origin = format!("{scheme}//{}", u.authority);
    if to.starts_with('/') {
        return Some(format!("{origin}{to}"));
    }
    // Next to the current page: `/a/b?x` + `c` → `/a/c`.
    let dir = u.path.split('?').next().unwrap_or("/");
    let dir = &dir[..dir.rfind('/').map_or(0, |i| i + 1)];
    Some(format!("{origin}{dir}{to}"))
}

// ---- The server (PLAN Part 4 "The web server"): plain HTTP on 127.0.0.1 (every address for
// `serve_network`). Several connections are read at once (a browser opens some in advance and says
// nothing on them), but the program answers one question at a time; each connection is closed
// after its answer.

/// What a system gives to wait for questions: its plain sockets (never encrypted).
pub trait Serve: Network {
    type Listener;
    type Datagram;
    /// Listens on `port` of 127.0.0.1, or of every address (0.0.0.0) for `everyone`; `None` if the
    /// port is taken or not allowed.
    fn listen(&mut self, port: u16, everyone: bool) -> Option<Self::Listener>;
    /// The next connection and the address it comes from, each `send` and `receive` limited to
    /// `ms` milliseconds.
    fn accept(&mut self, listener: &mut Self::Listener, ms: u64) -> Option<(Self::Link, [u8; 4])>;
    /// Waits until `links` or the listener can be read (`poll`): one flag per link, then the
    /// listener's. All false if the wait was cut short.
    fn ready(&mut self, listener: &Self::Listener, links: &[Self::Link]) -> Vec<bool>;
    /// This computer's IPv4 addresses, each with its interface's name ("" if the system does not
    /// say).
    fn addresses(&mut self) -> Vec<(String, [u8; 4])>;
    fn stop(&mut self, listener: Self::Listener);
    /// A UDP socket on `port` (0: any), each `receive_from` limited to `ms` milliseconds.
    fn udp(&mut self, port: u16, everyone: bool, ms: u64) -> Option<Self::Datagram>;
    fn send_to(&mut self, socket: &Self::Datagram, host: &str, port: u16, data: &[u8]) -> Result<(), Fail>;
    /// A datagram into `buffer`: its size, and who sent it.
    fn receive_from(&mut self, socket: &Self::Datagram, buffer: &mut [u8]) -> Result<(usize, [u8; 4], u16), Fail>;
    fn close_datagram(&mut self, socket: Self::Datagram);
}

// ---- `import net` (PLAN Part 4 "Sockets"): src/library/net.kova calls `net_raw` and `net_bytes` ----

/// How `net.kova` is told what went wrong.
const PORT_TAKEN: i64 = 6;
const CLOSED: i64 = 7;

fn failure(f: Fail) -> i64 {
    match f {
        Fail::NoHost => 1,
        Fail::Refused => 2,
        Fail::Timeout => 3,
        Fail::BadCertificate => 5,
        _ => 4,
    }
}

pub enum Socket<A, B> {
    Plain(A),
    Secure(B),
}

/// What a number in Kova stands for. Several threads may use it: each holds an `Arc` meanwhile,
/// and the last one out closes it (`finish`). So a thread may send while another receives.
// ponytail: `close` does not wake a thread blocked in `receive`; `shutdown` if a program needs it.
pub struct Held<X>(core::cell::UnsafeCell<Option<X>>, [u8; 4]);

impl<X> Held<X> {
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> Option<&mut X> {
        unsafe { &mut *self.0.get() }.as_mut()
    }
}

type Slots<X> = Vec<Option<Arc<Held<X>>>>;

pub struct Table<N: Sockets> {
    links: Slots<Socket<N::Link, <N::Tls as Network>::Link>>,
    listeners: Slots<N::Listener>,
    datagrams: Slots<N::Datagram>,
}

impl<N: Sockets> Table<N> {
    pub const fn new() -> Self {
        Table { links: Vec::new(), listeners: Vec::new(), datagrams: Vec::new() }
    }
}

/// The system's sockets as `kova run` and the runtime each reach them.
pub trait Sockets: Serve + Sized {
    type Tls: Network;
    fn plain() -> Self;
    fn tls() -> Self::Tls;
    fn table<R>(f: impl FnOnce(&mut Table<Self>) -> R) -> R;
}

fn add<X>(slots: &mut Slots<X>, x: X, from: [u8; 4]) -> i64 {
    slots.push(Some(Arc::new(Held(core::cell::UnsafeCell::new(Some(x)), from))));
    slots.len() as i64
}

fn get<X>(slots: &Slots<X>, id: i64) -> Option<Arc<Held<X>>> {
    slots.get((id as usize).wrapping_sub(1))?.clone()
}

fn take<X>(slots: &mut Slots<X>, id: i64) -> Option<Arc<Held<X>>> {
    slots.get_mut((id as usize).wrapping_sub(1))?.take()
}

/// The last user of a socket closes it.
fn finish<X>(held: Arc<Held<X>>, close: impl FnOnce(X)) {
    if let Some(x) = Arc::into_inner(held).and_then(|h| h.0.into_inner()) {
        close(x);
    }
}

fn finish_link<N: Sockets>(held: Arc<Held<Socket<N::Link, <N::Tls as Network>::Link>>>) {
    finish(held, |s| match s {
        Socket::Plain(l) => N::plain().close(l),
        Socket::Secure(l) => N::tls().close(l),
    });
}

/// `net.raw(kind, text, b, c)`: the operations that give a number; negative: what went wrong.
pub fn net_raw<N: Sockets>(kind: i64, text: &str, b: i64, c: i64) -> i64 {
    let port = u16::try_from(b).unwrap_or(0);
    match kind {
        // `c`: the time limit in milliseconds, then 1 if encrypted.
        0 => {
            let (secure, ms) = (c & 1 == 1, (c >> 1) as u64);
            let opened = if secure {
                N::tls().open(text, port, true, ms).map(Socket::Secure)
            } else {
                N::plain().open(text, port, false, ms).map(Socket::Plain)
            };
            match opened {
                Ok(s) => N::table(|t| add(&mut t.links, s, [0; 4])),
                Err(f) => -failure(f),
            }
        }
        1 => match N::plain().listen(port, c != 0) {
            Some(l) => N::table(|t| add(&mut t.listeners, l, [0; 4])),
            None => -PORT_TAKEN,
        },
        2 => {
            let Some(held) = N::table(|t| get(&t.listeners, b)) else { return -CLOSED };
            let got = held.get().and_then(|l| N::plain().accept(l, c as u64));
            finish(held, |l| N::plain().stop(l));
            match got {
                Some((l, from)) => N::table(|t| add(&mut t.links, Socket::Plain(l), from)),
                None => -failure(Fail::Broken),
            }
        }
        3 => {
            if let Some(held) = N::table(|t| take(&mut t.links, b)) {
                finish_link::<N>(held);
            }
            0
        }
        4 => {
            if let Some(held) = N::table(|t| take(&mut t.listeners, b)) {
                finish(held, |l| N::plain().stop(l));
            }
            0
        }
        // `c`: the time limit in milliseconds, then 1 for every address.
        5 => match N::plain().udp(port, c & 1 == 1, (c >> 1) as u64) {
            Some(s) => N::table(|t| add(&mut t.datagrams, s, [0; 4])),
            None => -PORT_TAKEN,
        },
        6 => {
            if let Some(held) = N::table(|t| take(&mut t.datagrams, b)) {
                finish(held, |s| N::plain().close_datagram(s));
            }
            0
        }
        // Who is at the other end of an accepted connection.
        _ => N::table(|t| get(&t.links, b)).map_or(0, |held| i64::from(u32::from_be_bytes(held.1))),
    }
}

/// `net.raw_bytes(kind, text, id, data, c)`: a status byte (0, or what went wrong), then the bytes.
pub fn net_bytes<N: Sockets>(kind: i64, text: &str, id: i64, data: &[u8], c: i64) -> Vec<u8> {
    let status = |r: Result<(), Fail>| alloc::vec![r.map_or_else(failure, |()| 0) as u8];
    match kind {
        0 => {
            let Some(held) = N::table(|t| get(&t.links, id)) else { return alloc::vec![CLOSED as u8] };
            let sent = match held.get() {
                Some(Socket::Plain(l)) => N::plain().send(l, data),
                Some(Socket::Secure(l)) => N::tls().send(l, data),
                None => Err(Fail::Broken),
            };
            finish_link::<N>(held);
            status(sent)
        }
        1 => {
            let Some(held) = N::table(|t| get(&t.links, id)) else { return alloc::vec![CLOSED as u8] };
            let mut out = alloc::vec![0; c as usize + 1];
            let got = match held.get() {
                Some(Socket::Plain(l)) => N::plain().receive(l, &mut out[1..]),
                Some(Socket::Secure(l)) => N::tls().receive(l, &mut out[1..]),
                None => Err(Fail::Broken),
            };
            finish_link::<N>(held);
            match got {
                Ok(n) => {
                    out.truncate(n + 1);
                    out
                }
                Err(f) => alloc::vec![failure(f) as u8],
            }
        }
        2 => {
            let Some(held) = N::table(|t| get(&t.datagrams, id)) else { return alloc::vec![CLOSED as u8] };
            let sent = held.get().map_or(Err(Fail::Broken), |s| N::plain().send_to(s, text, u16::try_from(c).unwrap_or(0), data));
            finish(held, |s| N::plain().close_datagram(s));
            status(sent)
        }
        _ => {
            let Some(held) = N::table(|t| get(&t.datagrams, id)) else { return alloc::vec![CLOSED as u8] };
            let mut out = alloc::vec![0; 65536 + 7];
            let got = held.get().map_or(Err(Fail::Broken), |s| N::plain().receive_from(s, &mut out[7..]));
            finish(held, |s| N::plain().close_datagram(s));
            match got {
                Ok((n, from, port)) => {
                    out[1..5].copy_from_slice(&from);
                    out[5..7].copy_from_slice(&port.to_be_bytes());
                    out.truncate(n + 7);
                    out
                }
                Err(f) => alloc::vec![failure(f) as u8],
            }
        }
    }
}

/// How long an answer may take to leave: a program that does not read must not stop the server.
pub const SERVER_MS: u64 = 10_000;
/// The largest head of a question (431 beyond) and the largest body (413 beyond).
const HEAD_LIMIT: usize = 65536;
const BODY_LIMIT: usize = 16 << 20;
/// ponytail: past this many connections not yet asking, the oldest is closed; a time limit per
/// connection if servers open to the network need one.
const WAITING: usize = 64;

/// A listening server, and the connections whose question is still coming.
pub struct Server<S: Serve> {
    listener: S::Listener,
    links: Vec<S::Link>,
    got: Vec<Vec<u8>>,
    from: Vec<[u8; 4]>,
    /// A line is written for each question answered (`Log.All`).
    pub logs: bool,
}

impl<S: Serve> Server<S> {
    pub fn new(listener: S::Listener, logs: bool) -> Self {
        Server { listener, links: Vec::new(), got: Vec::new(), from: Vec::new(), logs }
    }
}

/// A question received: a `Request` in Kova (its headers' names in small letters).
#[derive(Debug, PartialEq)]
pub struct Question {
    pub method: String,
    /// As written after the method: `/books?author=Herbert`.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// The same body as it came.
    pub data: Vec<u8>,
}

/// Waits for the next question that can be read, from whichever connection sends it first. The
/// others are answered here (400, 413…) and closed: the program never sees them.
pub fn next_question<S: Serve>(net: &mut S, server: &mut Server<S>) -> (S::Link, Question, [u8; 4]) {
    let mut buffer = alloc::vec![0u8; 16384];
    loop {
        let ready = net.ready(&server.listener, &server.links);
        // From the end, so that removing one leaves the others' numbers as they were.
        for i in (0..server.links.len()).rev().filter(|&i| ready[i]) {
            let parsed = match net.receive(&mut server.links[i], &mut buffer) {
                Ok(n) if n > 0 => {
                    server.got[i].extend_from_slice(&buffer[..n]);
                    parse_question(&server.got[i])
                }
                _ => Err(None),
            };
            if let Ok(None) = parsed {
                continue;
            }
            let mut link = server.links.remove(i);
            server.got.remove(i);
            let from = server.from.remove(i);
            match parsed {
                Ok(Some(q)) => return (link, q, from),
                Err(Some(status)) => {
                    let text = status_text(status);
                    let _ = net.send(&mut link, &answer_bytes(status, &[("content-type", "text/plain; charset=utf-8")], text.as_bytes()));
                }
                _ => {}
            }
            net.close(link);
        }
        if ready.last() == Some(&true) {
            if let Some((link, from)) = net.accept(&mut server.listener, SERVER_MS) {
                if server.links.len() == WAITING {
                    net.close(server.links.remove(0));
                    server.got.remove(0);
                    server.from.remove(0);
                }
                server.links.push(link);
                server.got.push(Vec::new());
                server.from.push(from);
            }
        }
    }
}

/// The question in the bytes received so far: `Ok(None)` while it is not whole, or the status
/// that refuses it (`None`: the connection broke).
fn parse_question(got: &[u8]) -> Result<Option<Question>, Option<i64>> {
    let Some(end) = find(got, b"\r\n\r\n") else {
        return if got.len() > HEAD_LIMIT { Err(Some(431)) } else { Ok(None) };
    };
    if end > HEAD_LIMIT {
        return Err(Some(431));
    }
    let head = core::str::from_utf8(&got[..end]).map_err(|_| Some(400))?;
    let mut lines = head.split("\r\n");
    let mut words = lines.next().unwrap_or("").split(' ');
    let (Some(method), Some(url), Some(version), None) = (words.next(), words.next(), words.next(), words.next()) else { return Err(Some(400)) };
    if !token(method) || !url.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Err(Some(400));
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(Some(400))?;
        if !token(name) {
            return Err(Some(400));
        }
        let (name, value) = (name.to_ascii_lowercase(), value.trim_matches([' ', '\t']));
        match headers.iter_mut().find(|(n, _)| *n == name) {
            Some((_, v)) => {
                *v += ", ";
                *v += value;
            }
            None => headers.push((name, value.to_string())),
        }
    }
    let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    // ponytail: a body sent in pieces is refused; read it with `chunks` if programs need it.
    if header("transfer-encoding").is_some() {
        return Err(Some(501));
    }
    let length = match header("content-length") {
        None => 0,
        Some(l) => l.parse::<usize>().map_err(|_| Some(400))?,
    };
    if length > BODY_LIMIT {
        return Err(Some(413));
    }
    if got.len() < end + 4 + length {
        return Ok(None);
    }
    let data = got[end + 4..end + 4 + length].to_vec();
    Ok(Some(Question { method: method.to_string(), url: url.to_string(), headers, body: String::from_utf8_lossy(&data).into_owned(), data }))
}

/// The bytes of an answer. Kova writes its length and `Connection: close` itself (the program's
/// own of those are left out); a header it cannot send (a line break in it…) or a status that
/// is not one makes it a 500.
pub fn answer_bytes(status: i64, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let bad = |(n, v): &&(&str, &str)| !token(n) || v.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0));
    if !(200..600).contains(&status) || headers.iter().any(|h| bad(&h)) {
        return answer_bytes(500, &[("content-type", "text/plain; charset=utf-8")], status_text(500).as_bytes());
    }
    let mut out = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (name, value) in headers.iter().filter(|(n, _)| !OWN.iter().any(|o| n.eq_ignore_ascii_case(o))) {
        out += &format!("{name}: {value}\r\n");
    }
    out += &format!("content-length: {}\r\nconnection: close\r\n\r\n", body.len());
    [out.as_bytes(), body].concat()
}

/// Sends `serve`'s answer on `link`, then closes it. A file (`Answer.file`) is read now, its
/// `content-type` found by its extension unless the program gave one; `read` reads it.
/// Returns the status sent (a 500 if the answer could not be sent as it was).
pub fn reply<N: Network>(net: &mut N, mut link: N::Link, status: i64, headers: &[(&str, &str)], body: &[u8], file: &str, read: impl Fn(&str) -> Option<Vec<u8>>) -> i64 {
    let bytes = if file.is_empty() {
        answer_bytes(status, headers, body)
    } else if let Some(content) = read(file) {
        let mut headers = headers.to_vec();
        if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-type")) {
            headers.push(("content-type", content_type(file)));
        }
        answer_bytes(status, &headers, &content)
    } else {
        answer_bytes(500, &[("content-type", "text/plain; charset=utf-8")], status_text(500).as_bytes())
    };
    let _ = net.send(&mut link, &bytes);
    net.close(link);
    // `HTTP/1.1 200 …`: always three digits (`answer_bytes`).
    core::str::from_utf8(&bytes[9..12]).ok().and_then(|s| s.parse().ok()).unwrap_or(500)
}

/// `public` → `public/`: a folder, as `serve_files` names it when it starts.
pub fn shown_folder(folder: &str) -> String {
    if folder.ends_with('/') || folder.ends_with('\\') { folder.to_string() } else { format!("{folder}/") }
}

/// The words of the lines `serve` writes when it starts, in the language of whoever runs it.
pub struct Said<'a> {
    /// `Serving on port 8080`, or `Serving public/ on port 8080`.
    pub title: &'a str,
    pub this_computer: &'a str,
    pub this_network: &'a str,
    pub stop: &'a str,
}

/// What `serve` writes when it starts (PLAN Part 4 "The web server, what it says"): the addresses
/// to open, each with who can use it. `addresses` is `Serve::addresses`, read only for `everyone`.
pub fn banner(port: u16, everyone: bool, addresses: &[(String, [u8; 4])], said: &Said) -> String {
    let mut rows = alloc::vec![(format!("http://localhost:{port}/"), said.this_computer.to_string())];
    // ponytail: interfaces only this computer reaches are known by their names (Docker, VMs); on
    // Windows, which gives no names, they are shown.
    let inside = ["docker", "br-", "veth", "virbr", "lxc", "podman", "cni", "vmnet", "bridge", "vboxnet"];
    for (_, ip) in addresses.iter().filter(|(name, ip)| everyone && ip[0] != 127 && !inside.iter().any(|i| name.starts_with(i))) {
        // 100.64.0.0/10: the addresses Tailscale gives.
        let who = if ip[0] == 100 && ip[1] & 0xC0 == 64 { "Tailscale" } else { said.this_network };
        rows.push((format!("http://{}.{}.{}.{}:{port}/", ip[0], ip[1], ip[2], ip[3]), who.to_string()));
    }
    let width = rows.iter().map(|(url, _)| url.len()).max().unwrap_or(0) + 4;
    let mut out = format!("\u{2713} {}\n", said.title);
    for (url, who) in rows {
        out += &format!("  {url:width$}{who}\n");
    }
    out + "  " + said.stop + "\n"
}

/// The line `serve` writes for each question answered: its local time (seconds since 1970, wall
/// clock), who asked, what, and the status sent.
pub fn logged(now: i64, from: [u8; 4], method: &str, url: &str, status: i64) -> String {
    let t = now.rem_euclid(86400);
    format!("{:02}:{:02}:{:02}  {}.{}.{}.{}  {method} {url}  {status}\n", t / 3600, t / 60 % 60, t % 60, from[0], from[1], from[2], from[3])
}

/// What `Answer.status(404)` says: `404 Not Found`.
pub fn status_text(status: i64) -> String {
    format!("{status} {}", reason(status)).trim_end().to_string()
}

fn reason(status: i64) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "",
    }
}

/// `%20` → a space (and `+` too in a query); `None` if it is not UTF-8 or a `%` is not followed
/// by two hexadecimal digits.
fn decode(text: &str, plus: bool) -> Option<String> {
    let (b, mut out, mut i) = (text.as_bytes(), Vec::new(), 0);
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = core::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
                continue;
            }
            b'+' if plus => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// `q.path`: the path of `url` without `?…`, decoded (as written if it cannot be).
pub fn url_path(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    decode(path, false).unwrap_or_else(|| path.to_string())
}

/// `q.query(name)`: the first value of `name` after the `?`, decoded.
pub fn url_query(url: &str, name: &str) -> Option<String> {
    let query = url.split('#').next()?.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (n, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(n, true)? == name).then(|| decode(v, true)).flatten()
    })
}

/// `Answer.file(path)`: the file to send (a folder's `index.html`), or `None` (a 404). A path
/// going up (`..`) or holding a 0 is refused whatever made it: it is often built from what a
/// question asked. `kind`: `Some(true)` for a folder, `Some(false)` for a file, `None` if absent.
pub fn file_to_serve(path: &str, kind: impl Fn(&str) -> Option<bool>) -> Option<String> {
    // A drive (`C:`) only at the start: elsewhere `:` names a hidden stream on Windows.
    let rest = if path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic() { &path[2..] } else { path };
    if path.is_empty() || path.contains('\0') || rest.contains(':') || path.split(['/', '\\']).any(|part| part == "..") {
        return None;
    }
    match kind(path)? {
        false => Some(path.to_string()),
        true => {
            let index = format!("{}/index.html", path.trim_end_matches(['/', '\\']));
            (kind(&index) == Some(false)).then_some(index)
        }
    }
}

/// The `content-type` of a file, by its extension.
pub fn content_type(path: &str) -> &'static str {
    let extension = path.rsplit_once('.').map_or("", |(_, e)| e).to_ascii_lowercase();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "txt" | "kova" | "md" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::VecDeque;

    /// Answers each connection with the next canned answer, a few bytes at a time.
    struct Fake {
        answers: VecDeque<&'static str>,
        sent: Vec<(String, u16, String)>,
    }

    impl Network for Fake {
        type Link = (Vec<u8>, usize);
        fn open(&mut self, host: &str, port: u16, _: bool, _: u64) -> Result<Self::Link, Fail> {
            if host == "nowhere" {
                return Err(Fail::NoHost);
            }
            self.sent.push((host.into(), port, String::new()));
            Ok((self.answers.pop_front().unwrap().as_bytes().to_vec(), 0))
        }
        fn send(&mut self, _: &mut Self::Link, data: &[u8]) -> Result<(), Fail> {
            self.sent.last_mut().unwrap().2 += core::str::from_utf8(data).unwrap();
            Ok(())
        }
        fn receive(&mut self, link: &mut Self::Link, buffer: &mut [u8]) -> Result<usize, Fail> {
            let n = (link.0.len() - link.1).min(7).min(buffer.len());
            buffer[..n].copy_from_slice(&link.0[link.1..link.1 + n]);
            link.1 += n;
            Ok(n)
        }
        fn close(&mut self, _: Self::Link) {}
    }

    fn fake(answers: &[&'static str]) -> Fake {
        Fake { answers: answers.iter().copied().collect(), sent: vec![] }
    }

    fn exchange(net: &mut Fake, method: &str, url: &str, body: &str, follow: bool) -> Result<Answer, Fail> {
        super::exchange(net, &Request { method, url, body: body.as_bytes(), headers: vec![], follow, ms: 0 })
    }

    #[test]
    fn headers() {
        let ok = "HTTP/1.1 204 No Content\r\n\r\n";
        let mut net = fake(&[ok]);
        let r = Request { method: "PUT", url: "http://a/", body: b"", headers: vec![("user-agent", "me"), ("X-Kind", "test")], follow: true, ms: 0 };
        assert_eq!(super::exchange(&mut net, &r).unwrap().status, 204);
        assert_eq!(net.sent[0].2, "PUT / HTTP/1.1\r\nHost: a\r\nAccept: */*\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 0\r\nuser-agent: me\r\nX-Kind: test\r\n\r\n");
        for bad in [("X", "a\r\nEvil: 1"), ("Host", "b"), ("a b", "c"), ("", "d")] {
            let r = Request { headers: vec![bad], ..r };
            assert_eq!(super::exchange(&mut net, &r), Err(Fail::BadHeader), "{bad:?}");
        }
    }

    #[test]
    fn urls() {
        let u = parse_url("http://example.com").unwrap();
        assert_eq!((u.secure, u.host.as_str(), u.port, u.path.as_str(), u.authority.as_str()), (false, "example.com", 80, "/", "example.com"));
        let u = parse_url("HTTPS://[::1]:8443/a b?q=1#top").unwrap();
        assert_eq!((u.secure, u.host.as_str(), u.port, u.path.as_str(), u.authority.as_str()), (true, "::1", 8443, "/a%20b?q=1", "[::1]:8443"));
        assert_eq!(parse_url("http://localhost:8080?x").unwrap().path, "/?x");
        for bad in ["ftp://x", "http://", "http://a:0", "http://a:99999", "http://u:p@x", "example.com", "http://[::1"] {
            assert_eq!(parse_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn answers() {
        let mut net = fake(&["HTTP/1.1 200 OK\r\nContent-Length: 5\r\nSet-Cookie: a\r\nset-cookie: b\r\n\r\nhello and more"]);
        let a = exchange(&mut net, "GET", "http://example.com/x", "", true).unwrap();
        assert_eq!((a.status, a.body.as_str()), (200, "hello"));
        assert_eq!(a.headers, vec![("content-length".into(), "5".into()), ("set-cookie".into(), "a, b".into())]);
        assert_eq!(net.sent[0].2, "GET /x HTTP/1.1\r\nHost: example.com\r\nUser-Agent: Kova\r\nAccept: */*\r\nConnection: close\r\n\r\n");
        // In pieces, then until the end of the connection.
        let mut net = fake(&["HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n6;x=1\r\npedia!\r\n0\r\n\r\n", "HTTP/1.0 200 OK\r\n\r\nall of it"]);
        let a = exchange(&mut net, "GET", "http://a/", "", true).unwrap();
        assert_eq!((a.status, a.body.as_str()), (404, "Wikipedia!"));
        assert_eq!(exchange(&mut net, "GET", "http://a/", "", true).unwrap().body, "all of it");
        let mut net = fake(&["HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n"]);
        let a = exchange(&mut net, "POST", "http://a:8080/notes", "bonjour", true).unwrap();
        assert_eq!(a.status, 201);
        assert!(net.sent[0].2.ends_with("Host: a:8080\r\nUser-Agent: Kova\r\nAccept: */*\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 7\r\n\r\nbonjour"));
    }

    #[test]
    fn failures() {
        let mut net = fake(&["nonsense\r\n\r\n", "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort"]);
        assert_eq!(exchange(&mut net, "GET", "http://a/", "", true), Err(Fail::BadAnswer));
        assert_eq!(exchange(&mut net, "GET", "http://a/", "", true), Err(Fail::Broken));
        assert_eq!(exchange(&mut net, "GET", "http://nowhere/", "", true), Err(Fail::NoHost));
        assert_eq!(exchange(&mut net, "GET", "mailto:x", "", true), Err(Fail::BadUrl));
    }

    #[test]
    fn redirections() {
        let moved = "HTTP/1.1 303 See Other\r\nLocation: ../c?d\r\nContent-Length: 0\r\n\r\n";
        let mut net = fake(&[moved, "HTTP/1.1 302 Found\r\nLocation: //b.org/e\r\n\r\n", "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"]);
        let a = exchange(&mut net, "POST", "http://a.org/x/y", "data", true).unwrap();
        assert_eq!((a.status, a.url.as_str(), a.body.as_str()), (200, "http://b.org/e", "ok"));
        assert!(net.sent[1].2.starts_with("GET /x/../c?d HTTP/1.1\r\nHost: a.org\r\n"));
        // Not followed: the redirection itself.
        let mut net = fake(&[moved]);
        assert_eq!(exchange(&mut net, "GET", "http://a.org/", "", false).unwrap().status, 303);
        let mut net = fake(&["HTTP/1.1 301 Moved\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n"; 11]);
        assert_eq!(exchange(&mut net, "GET", "http://a.org/", "", true), Err(Fail::TooManyRedirects));
    }
    #[test]
    fn questions() {
        let read = |text: &'static str| parse_question(text.as_bytes());
        let q = read("POST /books?a=1 HTTP/1.1\r\nHost: x\r\nX-A: 1\r\nx-a: 2\r\nContent-Length: 5\r\n\r\nhello").unwrap().unwrap();
        assert_eq!(q, Question { method: "POST".into(), url: "/books?a=1".into(), headers: vec![("host".into(), "x".into()), ("x-a".into(), "1, 2".into()), ("content-length".into(), "5".into())], body: "hello".into(), data: "hello".as_bytes().to_vec() });
        assert_eq!(read("GET / HTTP/1.1\r\n\r\n").unwrap().unwrap().body, "");
        for (text, status) in [("GET http://x/ HTTP/1.1\r\n\r\n", 400), ("GET / HTTP/1.1\r\nBad Name: 1\r\n\r\n", 400), ("GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n", 501), ("POST / HTTP/1.1\r\nContent-Length: 99999999\r\n\r\n", 413)] {
            assert_eq!(read(text), Err(Some(status)), "{text:?}");
        }
        // Not whole yet.
        for text in ["POST / HTTP/1.1\r\nContent-Length: 9\r\n\r\nhi", "GET /", ""] {
            assert_eq!(read(text), Ok(None), "{text:?}");
        }
    }

    #[test]
    fn answers_written() {
        let a = answer_bytes(404, &[("content-type", "text/plain"), ("Content-Length", "9")], b"no");
        assert_eq!(a, b"HTTP/1.1 404 Not Found\r\ncontent-type: text/plain\r\ncontent-length: 2\r\nconnection: close\r\n\r\nno");
        assert!(answer_bytes(200, &[("x", "a\r\nb")], b"").starts_with(b"HTTP/1.1 500 Internal Server Error\r\n"));
        assert!(answer_bytes(42, &[], b"").starts_with(b"HTTP/1.1 500"));
        assert_eq!(status_text(299), "299");
    }

    #[test]
    fn paths() {
        assert_eq!(url_path("/a%20b/c?x=1"), "/a b/c");
        assert_eq!(url_path("/%zz"), "/%zz");
        assert_eq!(url_query("/s?q=a+b%26c&n=&q=2", "q").as_deref(), Some("a b&c"));
        assert_eq!(url_query("/s?n", "n").as_deref(), Some(""));
        assert_eq!(url_query("/s?a=1", "b"), None);
        let kind = |p: &str| match p {
            "site" | "site/docs" => Some(true),
            "site/index.html" | "site/a.png" | "C:/site/a.png" => Some(false),
            _ => None,
        };
        assert_eq!(file_to_serve("site/a.png", kind).as_deref(), Some("site/a.png"));
        assert_eq!(file_to_serve("site", kind).as_deref(), Some("site/index.html"));
        assert_eq!(file_to_serve("site/docs", kind), None);
        assert_eq!(file_to_serve("C:/site/a.png", kind).as_deref(), Some("C:/site/a.png"));
        for bad in ["site/../site/a.png", "site\\..\\x", "site/a.png\0", "site/a.png::$DATA", "", "site/nothing"] {
            assert_eq!(file_to_serve(bad, kind), None, "{bad:?}");
        }
        assert_eq!(content_type("A.PNG"), "image/png");
        assert_eq!(content_type("x"), "application/octet-stream");
    }

}
