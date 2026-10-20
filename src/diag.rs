// Error messages: a position in the text + a message.
// The compiler returns a list of `Diag` instead of stopping:
// `kova run` prints them, `kova lsp` sends them to the editor.

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

/// Line and column (from 1, column in characters) of a byte position.
pub fn line_col(source: &str, pos: usize) -> (usize, usize) {
    let before = &source[..pos.min(source.len())];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].chars().count() + 1)
}

/// Renders an error the way rustc does:
///
/// ```text
/// erreur: `a` est déclarée avec let
///   --> main.kova:3:5
///    |
///  3 |     a := 2
///    |     ^
///    = aide : ...
/// ```
pub fn render(d: &Diag, file: &str, source: &str) -> String {
    let (line, col) = line_col(source, d.span.start);
    let line_text = source.lines().nth(line - 1).unwrap_or("");
    // Underline: the length of the span, cut at the end of the line, at least 1.
    let rest = line_text.chars().count().saturating_sub(col - 1);
    let width = source
        .get(d.span.start..d.span.end)
        .map_or(1, |s| s.chars().count())
        .clamp(1, rest.max(1));
    let margin = " ".repeat(line.to_string().len());

    let mut s = format!("erreur: {}\n{margin}--> {file}:{line}:{col}\n", d.message);
    s += &format!("{margin} |\n{line} | {line_text}\n");
    s += &format!("{margin} | {}{}\n", " ".repeat(col - 1), "^".repeat(width));
    if let Some(help) = &d.help {
        s += &format!("{margin} = aide : {help}\n");
    }
    s
}
