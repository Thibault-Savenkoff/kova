// HTTP/1.1 for `get` and `post` (PLAN Part 4 "The web"), used by `kova run` (src/interp.rs) and by
// the runtime of `kova build` (runtime/), so that both send and read exactly the same bytes. Only
// `core` and `alloc`: the runtime has no `std`. The network itself (finding the address,
// connecting, encrypting) is each system's, behind `Network`.

use alloc::format;
use alloc::string::{String, ToString};
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
    pub body: &'a str,
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
    let mut body = r.body.to_string();
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
fn once<N: Network>(net: &mut N, r: &Request, method: &str, url: &str, body: &str) -> Result<Answer, Fail> {
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
    request += body;
    let mut link = net.open(&u.host, u.port, u.secure, r.ms)?;
    let read = net.send(&mut link, request.as_bytes()).and_then(|()| read_answer(net, &mut link, method == "HEAD"));
    net.close(link);
    let (status, headers, body) = read?;
    Ok(Answer { status, url: url.to_string(), body: String::from_utf8_lossy(&body).into_owned(), headers })
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
        super::exchange(net, &Request { method, url, body, headers: vec![], follow, ms: 0 })
    }

    #[test]
    fn headers() {
        let ok = "HTTP/1.1 204 No Content\r\n\r\n";
        let mut net = fake(&[ok]);
        let r = Request { method: "PUT", url: "http://a/", body: "", headers: vec![("user-agent", "me"), ("X-Kind", "test")], follow: true, ms: 0 };
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
}
