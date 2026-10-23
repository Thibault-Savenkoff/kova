// JSON's syntax (PLAN Part 4 "JSON"), shared by `kova run` (src/interp.rs) and the runtime of
// `kova build` (runtime/kova.rs): text → a flat table of nodes, and a String → its JSON text.
// Which node fills which field is the business of each type's helper, made by src/kir.rs.
// Only `core` and `alloc`.

use alloc::string::String;
use alloc::vec::Vec;

/// A node's kind, its first slot in the table (src/kir.rs reads these numbers).
pub const NULL: i64 = 0;
pub const FALSE: i64 = 1;
pub const TRUE: i64 = 2;
pub const NUMBER: i64 = 3;
pub const TEXT: i64 = 4;
pub const ARRAY: i64 = 5;
pub const OBJECT: i64 = 6;

/// A number's flags (slot 3).
pub const EXACT_INT: i64 = 1;
pub const FINITE: i64 = 2;

/// How deep arrays and objects may go: deeper texts are not JSON for Kova (a program must not
/// run out of stack because of what a site sent).
const DEEPEST: usize = 256;

/// One node, laid out in 4 slots: (kind, a, b, c).
/// - `NUMBER`: a = the Float's bits, b = the Int (when exact), c = flags.
/// - `TEXT`: a = the text (a String for Kova).
/// - `ARRAY`: a = the place of its first element, b = how many; they follow each other.
/// - `OBJECT`: the same with its members, each one 2 nodes: its name (a `TEXT`), then its value.
#[derive(Debug, PartialEq)]
pub enum Node {
    Plain(i64),
    Number { float: f64, int: i64, flags: i64 },
    Text(String),
    Array { first: usize, count: usize },
    Object { first: usize, count: usize },
}

/// The slots of a Kova List of width 4 holding `nodes` (`text` makes a String), then one more
/// `null`: what a missing field reads (src/kir.rs `json_reader`).
pub fn slots(nodes: Vec<Node>, mut text: impl FnMut(String) -> i64) -> Vec<i64> {
    let mut slots = Vec::with_capacity(4 * nodes.len() + 4);
    for n in nodes {
        slots.extend(match n {
            Node::Plain(k) => [k, 0, 0, 0],
            Node::Number { float, int, flags } => [NUMBER, float.to_bits() as i64, int, flags],
            Node::Text(t) => [TEXT, text(t), 0, 0],
            Node::Array { first, count } => [ARRAY, first as i64, count as i64, 0],
            Node::Object { first, count } => [OBJECT, first as i64, count as i64, 0],
        });
    }
    slots.extend([NULL, 0, 0, 0]);
    slots
}

/// A value read, before it is laid out flat.
enum Value {
    Plain(i64),
    Number(f64, i64, i64),
    Text(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// The nodes of `text` (the whole value first), or the line where it stops being JSON.
pub fn parse(text: &str) -> Result<Vec<Node>, i64> {
    let mut r = Reader { b: text.as_bytes(), at: 0 };
    let line = |r: &Reader| 1 + r.b[..r.at.min(r.b.len())].iter().filter(|&&c| c == b'\n').count() as i64;
    let value = r.value(0).ok_or_else(|| line(&r))?;
    r.space();
    if r.at != r.b.len() {
        return Err(line(&r));
    }
    // Laid out level by level, so that the children of each node follow each other.
    let mut nodes = Vec::new();
    let mut waiting = alloc::collections::VecDeque::from([value]);
    let mut next = 1;
    while let Some(v) = waiting.pop_front() {
        nodes.push(match v {
            Value::Plain(k) => Node::Plain(k),
            Value::Number(float, int, flags) => Node::Number { float, int, flags },
            Value::Text(t) => Node::Text(t),
            Value::Array(items) => {
                let node = Node::Array { first: next, count: items.len() };
                next += items.len();
                waiting.extend(items);
                node
            }
            Value::Object(members) => {
                let node = Node::Object { first: next, count: members.len() };
                next += 2 * members.len();
                for (name, v) in members {
                    waiting.push_back(Value::Text(name));
                    waiting.push_back(v);
                }
                node
            }
        });
    }
    Ok(nodes)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while matches!(self.b.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn word(&mut self, w: &[u8]) -> bool {
        let found = self.b[self.at..].starts_with(w);
        if found {
            self.at += w.len();
        }
        found
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        self.space();
        match *self.b.get(self.at)? {
            b'n' => self.word(b"null").then_some(Value::Plain(NULL)),
            b'f' => self.word(b"false").then_some(Value::Plain(FALSE)),
            b't' => self.word(b"true").then_some(Value::Plain(TRUE)),
            b'"' => self.text().map(Value::Text),
            b'[' if depth < DEEPEST => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.word(b"]") {
                    return Some(Value::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    if self.word(b"]") {
                        return Some(Value::Array(items));
                    }
                    if !self.word(b",") {
                        return None;
                    }
                }
            }
            b'{' if depth < DEEPEST => {
                self.at += 1;
                let mut members = Vec::new();
                self.space();
                if self.word(b"}") {
                    return Some(Value::Object(members));
                }
                loop {
                    self.space();
                    if self.b.get(self.at) != Some(&b'"') {
                        return None;
                    }
                    let name = self.text()?;
                    self.space();
                    if !self.word(b":") {
                        return None;
                    }
                    members.push((name, self.value(depth + 1)?));
                    self.space();
                    if self.word(b"}") {
                        return Some(Value::Object(members));
                    }
                    if !self.word(b",") {
                        return None;
                    }
                }
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    /// `-`, `0` or digits not starting with 0, then maybe `.` and digits, then maybe an exponent.
    fn number(&mut self) -> Option<Value> {
        let start = self.at;
        let digits = |r: &mut Self| {
            let d = r.at;
            while r.b.get(r.at).is_some_and(u8::is_ascii_digit) {
                r.at += 1;
            }
            r.at - d
        };
        self.word(b"-");
        let whole = digits(self);
        if whole == 0 || (whole > 1 && self.b[self.at - whole] == b'0') {
            return None;
        }
        let mut exact = true;
        if self.word(b".") {
            exact = false;
            if digits(self) == 0 {
                return None;
            }
        }
        if matches!(self.b.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            exact = false;
            if matches!(self.b.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return None;
            }
        }
        let s = core::str::from_utf8(&self.b[start..self.at]).ok()?;
        let float: f64 = s.parse().ok()?;
        // The Int from the digits themselves: 9007199254740993 stays exact.
        let int = if exact { s.parse::<i64>().ok() } else { None };
        let flags = if int.is_some() { EXACT_INT } else { 0 } | if float.is_finite() { FINITE } else { 0 };
        Some(Value::Number(float, int.unwrap_or(0), flags))
    }

    /// A text between quotes, its escapes read (`\n`, `\u00e9`, and the two halves of `\ud83d\ude00`).
    fn text(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while self.b.get(self.at).is_some_and(|&c| c != b'"' && c != b'\\' && c >= 0x20) {
                self.at += 1;
            }
            // The text was a Kova String: valid UTF-8, and cut here only at an ASCII byte.
            out.push_str(core::str::from_utf8(&self.b[start..self.at]).ok()?);
            match *self.b.get(self.at)? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    let c = *self.b.get(self.at)?;
                    self.at += 1;
                    out.push(match c {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let high = self.hex()?;
                            if (0xD800..0xDC00).contains(&high) {
                                // A lone half is not a letter: not JSON for Kova.
                                if !self.word(b"\\u") {
                                    return None;
                                }
                                let low = self.hex()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return None;
                                }
                                char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00))?
                            } else {
                                char::from_u32(high)?
                            }
                        }
                        _ => return None,
                    });
                }
                // A line break or another control letter, not written `\n`.
                _ => return None,
            }
        }
    }

    fn hex(&mut self) -> Option<u32> {
        let digits = self.b.get(self.at..self.at + 4)?;
        let mut v = 0;
        for &d in digits {
            v = v * 16 + (d as char).to_digit(16)?;
        }
        self.at += 4;
        Some(v)
    }
}

/// `text` as JSON writes it: between quotes, with `"`, `\` and the control letters escaped.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let hex = b"0123456789abcdef";
                out.push_str("\\u00");
                out.push(hex[c as usize >> 4] as char);
                out.push(hex[c as usize & 15] as char);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        let n = parse(r#" {"a": [1, -2.5e1, "x\u00e9\ud83d\ude00\n"], "b": null, "c": true} "#).unwrap();
        assert_eq!(n[0], Node::Object { first: 1, count: 3 });
        assert_eq!(n[1], Node::Text("a".into()));
        assert_eq!(n[2], Node::Array { first: 7, count: 3 });
        assert_eq!(n[4], Node::Plain(NULL));
        assert_eq!(n[6], Node::Plain(TRUE));
        assert_eq!(n[7], Node::Number { float: 1.0, int: 1, flags: EXACT_INT | FINITE });
        assert_eq!(n[8], Node::Number { float: -25.0, int: 0, flags: FINITE });
        assert_eq!(n[9], Node::Text("xé😀\n".into()));
        assert_eq!(parse("9007199254740993").unwrap()[0], Node::Number { float: 9007199254740992.0, int: 9007199254740993, flags: EXACT_INT | FINITE });
        assert_eq!(parse("1e400").unwrap()[0], Node::Number { float: f64::INFINITY, int: 0, flags: 0 });
        assert_eq!(parse("99999999999999999999").unwrap()[0], Node::Number { float: 1e20, int: 0, flags: FINITE });
    }

    #[test]
    fn not_json() {
        for (text, line) in [("", 1), ("[1,]", 1), ("{\n\"a\" 1}", 2), ("01", 1), ("1.", 1), ("\"\\ud800\"", 1), ("\"a\nb\"", 1), ("[1] x", 1), ("{'a': 1}", 1), ("nul", 1)] {
            assert_eq!(parse(text), Err(line), "{text:?}");
        }
        let deep = "[".repeat(DEEPEST + 1) + &"]".repeat(DEEPEST + 1);
        assert!(parse(&deep).is_err());
        let fine = "[".repeat(DEEPEST) + &"]".repeat(DEEPEST);
        assert!(parse(&fine).is_ok());
    }

    #[test]
    fn quoted() {
        assert_eq!(quote("a\"b\\c\n\u{1}é"), r#""a\"b\\c\n\u0001é""#);
        assert_eq!(parse(&quote("x\u{1f}\"")).unwrap()[0], Node::Text("x\u{1f}\"".into()));
    }
}
