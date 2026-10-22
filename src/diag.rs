// Error messages: a position in the text + a message.
// The compiler returns a list of `Diag` instead of stopping:
// `kova run` prints them, `kova lsp` sends them to the editor.

use crate::locale::t;

/// A piece of the source text, in bytes: `source[start..end]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }
    /// From the start of `self` to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start, other.end)
    }
}

#[derive(Debug)]
pub struct Diag {
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
}

impl Diag {
    pub fn new(span: Span, message: impl Into<String>) -> Diag {
        Diag { message: message.into(), span, help: None }
    }
    pub fn help(mut self, help: impl Into<String>) -> Diag {
        self.help = Some(help.into());
        self
    }
}

/// Between two files of a program in `Sources::text`: a character no file contains.
pub const SEPARATOR: char = '\u{1}';

/// All the files of a program, one after the other in a single text, so that a position
/// (`Span`) says both the file and the place in it (as rustc does).
pub struct Sources {
    pub text: String,
    /// Each file: the path shown in messages, and where its text starts in `text`.
    pub files: Vec<(String, usize)>,
}

impl Sources {
    pub fn new() -> Sources {
        Sources { text: String::new(), files: vec![] }
    }

    /// Adds a file; returns where its text starts. The first file starts at 0.
    pub fn add(&mut self, path: &str, text: &str) -> usize {
        if !self.files.is_empty() {
            self.text.push(SEPARATOR);
        }
        let start = self.text.len();
        self.text.push_str(text);
        self.files.push((path.to_string(), start));
        start
    }

    /// The number of the file containing position `pos`.
    pub fn file_of(&self, pos: usize) -> usize {
        self.files.iter().rposition(|(_, start)| *start <= pos).unwrap_or(0)
    }

    /// The text of file `k`.
    pub fn file_text(&self, k: usize) -> &str {
        let start = self.files[k].1;
        let end = self.files.get(k + 1).map_or(self.text.len(), |(_, s)| s - SEPARATOR.len_utf8());
        &self.text[start..end]
    }
}

/// The number of the file containing `pos` in a text made by `Sources` (0 for a single file).
pub fn file_number(source: &str, pos: usize) -> usize {
    source[..pos.min(source.len())].matches(SEPARATOR).count()
}

/// Line and column (from 1, column in characters) of a byte position, in its own file.
pub fn line_col(source: &str, pos: usize) -> (usize, usize) {
    let before = &source[..pos.min(source.len())];
    let before = &before[before.rfind(SEPARATOR).map_or(0, |i| i + SEPARATOR.len_utf8())..];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].chars().count() + 1)
}

/// Renders an error the way rustc does:
///
/// ```text
/// error: `a` is declared with let
///   --> main.kova:3:5
///    |
///  3 |     a := 2
///    |     ^
///    = help: ...
/// ```
pub fn render(d: &Diag, sources: &Sources) -> String {
    let k = sources.file_of(d.span.start);
    let (file, start) = (&sources.files[k].0, sources.files[k].1);
    let source = sources.file_text(k);
    let d = &Diag { message: d.message.clone(), span: Span::new(d.span.start - start, d.span.end - start), help: d.help.clone() };
    let (line, col) = line_col(source, d.span.start);
    let line_text = source.lines().nth(line - 1).unwrap_or("");
    // Underline: the length of the span, cut at the end of the line, at least 1.
    let rest = line_text.chars().count().saturating_sub(col - 1);
    let width = source
        .get(d.span.start..d.span.end)
        .map_or(1, |s| s.chars().count())
        .clamp(1, rest.max(1));
    let margin = " ".repeat(line.to_string().len());

    let mut s = format!("{}{}\n{margin}--> {file}:{line}:{col}\n", t!("error_prefix"), d.message);
    s += &format!("{margin} |\n{line} | {line_text}\n");
    s += &format!("{margin} | {}{}\n", " ".repeat(col - 1), "^".repeat(width));
    if let Some(help) = &d.help {
        s += &format!("{margin} = {}{help}\n", t!("help_prefix"));
    }
    s
}
