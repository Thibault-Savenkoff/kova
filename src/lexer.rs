// Lexer: cuts the text into tokens (words, numbers, symbols).
//
//   "let x: Int := 5"  →  [Keyword(let)] [Ident] [Sym(:)] [Type] [Sym(:=)] [Int(5)]
//
// It also adds the line ends (`Newline`) that end statements,
// following the rules of docs/PLAN.md 2.1 ("Line ends").

use crate::diag::{Diag, Span};
use crate::locale::t;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Lowercase name: variable, function (`total`, `carré`).
    Ident,
    /// Uppercase name: type (`Int`, `Chien`).
    Type,
    Int(u64),
    /// A decimal number too big even for 64 bits: a `BigInt`, read back from its text.
    Big,
    Float(f64),
    Char(char),
    /// `"..."`; the content is read back with `string_content`.
    Str,
    /// Keyword (`fn`, `let`, …).
    Keyword(&'static str),
    /// Symbol (`:=`, `+`, `{`, …).
    Sym(&'static str),
    /// End of statement.
    Newline,
    /// End of file.
    Eof,
}

#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub kind: Kind,
    pub span: Span,
}

pub const KEYWORDS: &[&str] = &[
    "fn", "let", "var", "move", "return", "if", "else", "while", "for", "in", "from", "match",
    "struct", "enum", "interface", "import", "public", "private", "require", "ensure", "check",
    "result", "self", "true", "false", "none", "ok", "err", "and", "or", "copy", "any", "break",
    "continue", "header", "test",
];

/// Longest first, so that `..=` wins over `..` then `.`.
const SYMBOLS: &[&str] = &[
    "...", "..=", ":=", "+=", "-=", "*=", "/=", "->", "=>", "!=", "<=", ">=", "&&", "||", "..", "+", "-", "*",
    "/", "%", "=", "<", ">", "!", "(", ")", "[", "]", "{", "}", ",", ":", ".", "?",
];

/// A line ending with one of these tokens continues on the next one.
fn continues_after(k: Kind) -> bool {
    matches!(k, Kind::Sym(s) if !matches!(s, ")" | "]" | "}" | "?" | "!"))
}

/// A line starting with these tokens continues the previous one: `.trim`, `&& ok`, `)`;
/// but `.Red` is a case of an enum, which starts its own line.
pub fn continues_before(k: Kind, next: Option<Kind>) -> bool {
    match k {
        Kind::Sym(".") => next != Some(Kind::Type),
        _ => matches!(k, Kind::Sym("&&" | "||" | ")" | "]" | "}")),
    }
}

pub fn lex(source: &str) -> (Vec<Token>, Vec<Diag>) {
    let (raw, diags) = raw_tokens(source);
    (add_newlines(raw), diags)
}

/// The tokens as written, without `Newline` nor `Eof` (for `kova fmt`).
pub fn raw(source: &str) -> (Vec<Token>, Vec<Diag>) {
    let (mut raw, diags) = raw_tokens(source);
    raw.pop();
    (raw.into_iter().map(|(t, _)| t).collect(), diags)
}

/// The tokens without `Newline`, each with "is there a line break just before?".
fn raw_tokens(source: &str) -> (Vec<(Token, bool)>, Vec<Diag>) {
    let mut tokens = vec![];
    let mut diags = vec![];
    let mut new_line = false;
    let mut i = 0;

    while let Some(c) = source[i..].chars().next() {
        let start = i;
        let rest = &source[i..];

        if c == '\n' {
            new_line = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            match comment_end(rest) {
                Some(n) => {
                    new_line |= rest[..n].contains('\n');
                    i += n;
                }
                None => {
                    diags.push(Diag::new(Span::new(i, i + 2), t!("comment_unclosed")));
                    i = source.len();
                }
            }
            continue;
        }

        let kind = if c.is_alphabetic() || c == '_' {
            let n = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
            let word = &rest[..n];
            i += n;
            // `l.any { … }`: after a `.`, `any` is the List method, not `any Animal`.
            let method = word == "any" && matches!(tokens.last(), Some((Token { kind: Kind::Sym("."), .. }, _)));
            if let Some(k) = KEYWORDS.iter().find(|&&k| k == word && !method) {
                Kind::Keyword(k)
            } else if c.is_uppercase() {
                Kind::Type
            } else {
                Kind::Ident
            }
        } else if c.is_ascii_digit() {
            // After a `.`, no decimal point: `t.0.1` = two field numbers, not `0.1`.
            let after_dot = matches!(tokens.last(), Some((Token { kind: Kind::Sym("."), .. }, _)));
            let n = number_len(rest, !after_dot);
            i += n;
            match read_number(&rest[..n]) {
                Ok(k) => k,
                Err(msg) => {
                    diags.push(Diag::new(Span::new(start, i), msg));
                    Kind::Int(0)
                }
            }
        } else if c == '\'' {
            let (n, result) = read_char(rest);
            i += n;
            match result {
                Ok(c) => Kind::Char(c),
                Err(msg) => {
                    diags.push(Diag::new(Span::new(start, i), msg));
                    Kind::Char(' ')
                }
            }
        } else if c == '"' {
            let (n, errors) = if rest.starts_with("\"\"\"") { read_raw(rest) } else { read_string(rest) };
            for (k, msg) in errors {
                diags.push(Diag::new(Span::new(start + k, start + k + 1), msg));
            }
            i += n;
            Kind::Str
        } else if let Some(s) = SYMBOLS.iter().find(|s| rest.starts_with(**s)) {
            i += s.len();
            Kind::Sym(s)
        } else {
            i += c.len_utf8();
            let d = Diag::new(Span::new(start, i), t!("unexpected_char", char = c));
            diags.push(if c == ';' { d.help(t!("habit_semicolon")) } else { d });
            continue;
        };

        tokens.push((Token { kind, span: Span::new(start, i) }, new_line));
        new_line = false;
    }
    tokens.push((Token { kind: Kind::Eof, span: Span::new(source.len(), source.len()) }, true));
    (tokens, diags)
}

/// Length of a (nestable) `/* ... */` comment at the start of `s`.
pub fn comment_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with("/*") {
            depth += 1;
            i += 2;
        } else if s[i..].starts_with("*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Some(i);
            }
        } else {
            i += s[i..].chars().next().unwrap().len_utf8();
        }
    }
    None
}

/// Length of the number at the start of `s`: `42`, `0xFF`, `3.14`, `1e-9`, `2.5E+3`.
/// With `fraction` = false, stops before a `.` (tuple field number).
fn number_len(s: &str, fraction: bool) -> usize {
    let b = s.as_bytes();
    let digits = |mut i: usize| {
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') && !matches!(b[i], b'e' | b'E') {
            i += 1;
        }
        i
    };
    if s.starts_with("0x") || s.starts_with("0b") {
        return s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(s.len());
    }
    let mut i = digits(0);
    // `1..10` stays a range: a digit is needed right after the `.`.
    if fraction && i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
        i = digits(i + 1);
    }
    if i < b.len() && matches!(b[i], b'e' | b'E') {
        let j = if matches!(b.get(i + 1), Some(b'+' | b'-')) { i + 2 } else { i + 1 };
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            i = digits(j);
        }
    }
    // Glued letters (`12abc`): taken too, `read_number` reports the error.
    i + s[i..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(s.len() - i)
}

fn read_number(text: &str) -> Result<Kind, String> {
    let decimal = !text.starts_with("0x") && !text.starts_with("0b");
    if decimal && text.contains(['.', 'e', 'E']) {
        let clean = text.replace('_', "");
        return match clean.parse::<f64>() {
            Ok(v) if v.is_infinite() => Err(t!("number_too_big", text = text)),
            Ok(v) => Ok(Kind::Float(v)),
            Err(_) => Err(t!("number_malformed", text = text)),
        };
    }
    match read_int(text) {
        Err(_) if decimal && text.replace('_', "").bytes().all(|b| b.is_ascii_digit()) => Ok(Kind::Big),
        other => other.map(Kind::Int),
    }
}

/// `'a'`, `'\n'`, `'\u{1F600}'`. Returns the length read and the character.
fn read_char(s: &str) -> (usize, Result<char, String>) {
    let line_end = s.find('\n').unwrap_or(s.len());
    let mut it = s[1..line_end].char_indices();
    let result = match it.next() {
        None | Some((_, '\'')) => Err(t!("char_empty")),
        Some((_, '\\')) => escape(&mut it),
        Some((_, c)) => Ok(c),
    };
    match it.next() {
        Some((k, '\'')) => (k + 2, result),
        _ => {
            let n = s[1..line_end].find('\'').map_or(line_end, |k| k + 2);
            (n, result.and(Err(t!("char_single"))))
        }
    }
}

/// `"..."` at the start of `s`: its length, and the errors (position in `s`, message).
fn read_string(s: &str) -> (usize, Vec<(usize, String)>) {
    let mut errors = vec![];
    let mut it = s.char_indices().skip(1).peekable();
    while let Some((k, c)) = it.next() {
        match c {
            '"' => return (k + 1, errors),
            '\n' => break,
            // `{code}`: skipped here (the parser reads it); it may hold texts and braces.
            '{' => match code_end(&s[k + 1..]) {
                Some(n) => {
                    while it.peek().is_some_and(|&(j, _)| j <= k + n) {
                        it.next();
                    }
                }
                None => {
                    errors.push((k, t!("interpolation_unclosed")));
                    return (s.find('\n').unwrap_or(s.len()), errors);
                }
            },
            '\\' => {
                let mut rest = s[k + 1..].char_indices();
                if let Err(msg) = escape(&mut rest) {
                    errors.push((k, msg));
                }
                // Skip as many characters as the escape read.
                let read = rest.next().map_or(s.len() - k - 1, |(j, _)| j);
                while it.peek().is_some_and(|&(j, _)| j <= k + read) {
                    it.next();
                }
            }
            _ => {}
        }
    }
    let n = s.find('\n').unwrap_or(s.len());
    errors.push((0, t!("string_unclosed")));
    (n, errors)
}

/// `"""..."""` at the start of `s` (PLAN Part 4 "Raw texts"): its length, and the errors. It may
/// hold quotes and line breaks; it ends at the last quote of the first run of three or more.
fn read_raw(s: &str) -> (usize, Vec<(usize, String)>) {
    let Some(mut end) = s[3..].find("\"\"\"").map(|k| k + 3) else {
        return (s.len(), vec![(0, t!("raw_unclosed"))]);
    };
    while s[end + 3..].starts_with('"') {
        end += 1;
    }
    let mut errors = vec![];
    if let Err((k, msg)) = raw_content(&s[..end + 3]) {
        errors.push((k, msg));
    }
    (end + 3, errors)
}

/// What a `"""..."""` token holds, kept as written but for its layout: on several lines, the
/// line of the opening `"""` must be empty and is left out, and the closing `"""` sits alone on
/// its line, whose indentation is taken off every line. An error: its position in the token.
pub fn raw_content(token: &str) -> Result<String, (usize, String)> {
    let inner = &token[3..token.len().saturating_sub(3).max(3)];
    if !inner.contains('\n') {
        return Ok(inner.to_string());
    }
    let lines: Vec<&str> = inner.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let (first, last) = (lines[0], lines[lines.len() - 1]);
    if !first.trim().is_empty() || !last.trim().is_empty() {
        return Err((0, t!("raw_layout")));
    }
    let mut out = vec![];
    let mut at = 3 + lines[0].len() + 1;
    for line in &lines[1..lines.len() - 1] {
        match line.strip_prefix(last) {
            Some(rest) => out.push(rest),
            None if line.trim().is_empty() => out.push(""),
            None => return Err((at, t!("raw_indent"))),
        }
        at += line.len() + 1;
    }
    Ok(out.join("\n"))
}

/// The end of the code of a `{code}` in a text: `code` starts `s`; returns the position of
/// the closing `}` (`None` if the line ends first). Texts and characters inside are skipped.
pub fn code_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut k = 0;
    while let Some(c) = s[k..].chars().next() {
        match c {
            '\n' => return None,
            '"' => {
                k += read_string(&s[k..]).0;
                continue;
            }
            '\'' => {
                k += read_char(&s[k..]).0;
                continue;
            }
            '{' => depth += 1,
            '}' if depth == 0 => return Some(k),
            '}' => depth -= 1,
            _ => {}
        }
        k += c.len_utf8();
    }
    None
}

/// The position of the first `{` or `}` of a `Str` token (a test name has no interpolation);
/// none in a raw text.
pub fn interpolation(token: &str) -> Option<usize> {
    if token.starts_with("\"\"\"") {
        return None;
    }
    let mut it = token.char_indices();
    while let Some((k, c)) = it.next() {
        match c {
            // `\{` is a brace, `\u{1F600}` a character.
            '\\' => {
                if let Some((_, 'u')) = it.next() {
                    it.by_ref().find(|&(_, c)| c == '}');
                }
            }
            '{' | '}' => return Some(k),
            _ => {}
        }
    }
    None
}

/// The content of a `Str` token (`"a\nb"` → `a`, line break, `b`), already checked by the lexer.
pub fn string_content(token: &str) -> String {
    if token.starts_with("\"\"\"") {
        return raw_content(token).unwrap_or_default();
    }
    let inner = token.strip_prefix('"').unwrap_or(token);
    let inner = inner.strip_suffix('"').unwrap_or(inner);
    let mut text = String::new();
    let mut it = inner.char_indices();
    while let Some((_, c)) = it.next() {
        text.push(if c == '\\' { escape(&mut it).unwrap_or('?') } else { c });
    }
    text
}

/// Reads what follows a `\`: `n t r 0 \\ " ' { }` or `u{1F600}`.
pub fn escape(it: &mut std::str::CharIndices) -> Result<char, String> {
    Ok(match it.next().map(|(_, c)| c) {
        Some('n') => '\n',
        Some('t') => '\t',
        Some('r') => '\r',
        Some('0') => '\0',
        Some(c @ ('\\' | '"' | '\'' | '{' | '}')) => c,
        Some('u') => {
            let mut hex = String::new();
            if it.next().map(|(_, c)| c) != Some('{') {
                return Err(t!("escape_unicode_form"));
            }
            for (_, c) in it.by_ref() {
                if c == '}' {
                    break;
                }
                hex.push(c);
            }
            let code = u32::from_str_radix(&hex, 16).map_err(|_| t!("escape_bad_code", hex = hex))?;
            char::from_u32(code).ok_or_else(|| t!("escape_not_unicode", hex = hex))?
        }
        Some(c) => return Err(t!("escape_unknown", char = c)),
        None => return Err(t!("escape_incomplete")),
    })
}

/// `42`, `1_000_000`, `0xFF`, `0b1010`.
fn read_int(text: &str) -> Result<u64, String> {
    let (digits, base) = if let Some(h) = text.strip_prefix("0x") {
        (h, 16)
    } else if let Some(b) = text.strip_prefix("0b") {
        (b, 2)
    } else {
        (text, 10)
    };
    let digits = digits.replace('_', "");
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(base)) {
        return Err(t!("number_malformed", text = text));
    }
    u64::from_str_radix(&digits, base).map_err(|_| t!("number_too_big", text = text))
}

/// Adds the `Newline` tokens that end statements (PLAN 2.1, rules 1 to 3).
fn add_newlines(raw: Vec<(Token, bool)>) -> Vec<Token> {
    let mut out: Vec<Token> = vec![];
    // Open brackets: inside `(` and `[`, line breaks don't count.
    let mut open: Vec<&str> = vec![];

    let next: Vec<Option<Kind>> = raw.iter().skip(1).map(|(t, _)| Some(t.kind)).chain([None]).collect();
    for ((t, new_line), next) in raw.into_iter().zip(next) {
        let in_braces = matches!(open.last(), None | Some(&"{"));
        if let Some(prev) = out.last()
            && new_line
            && in_braces
            && prev.kind != Kind::Newline
            && !continues_after(prev.kind)
            && !continues_before(t.kind, next)
        {
            out.push(Token { kind: Kind::Newline, span: Span::new(prev.span.end, prev.span.end) });
        }
        match t.kind {
            Kind::Sym(s @ ("(" | "[" | "{")) => open.push(s),
            Kind::Sym(")" | "]" | "}") => {
                open.pop();
            }
            _ => {}
        }
        out.push(t);
    }
    out
}
