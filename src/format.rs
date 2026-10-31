// `kova fmt`: lays a file out the one Kova way (PLAN Part 4, "E6"). Only the spaces between
// the tokens, the indentation and the empty lines change: the tokens, the comments and the line
// breaks stay where they are (checked at the end), so the program means the same.
//
//   fn main(){            fn main {
//   let x:Int:=-1+2   →       let x: Int := -1 + 2
//   print( x )}               print(x)
//                         }
//
// (`fn main(){` stays on its line: the formatter never moves a line break.)

use crate::lexer::{Kind, Token, comment_end, raw};

const INDENT: &str = "    ";

/// The text laid out; `None` when it doesn't read (an unclosed text or comment…) or when laying
/// it out would change its tokens: it then stays as it is.
pub fn format(src: &str) -> Option<String> {
    let (tokens, diags) = raw(src);
    if !diags.is_empty() {
        return None;
    }
    let text = |t: &Token| &src[t.span.start..t.span.end];
    let mut out = String::new();
    // Each bracket still open, with the indentation of the line that opened it, and whether it
    // is the brace of a block type (`f: {Int -> Bool}`, written without spaces inside).
    let mut open: Vec<usize> = vec![];
    let mut typed: Vec<bool> = vec![];
    // The `<` of `List<Int>` and `fn larger<T>` still open.
    let mut generics = 0;
    let mut line = 0;
    // The indentation of the line where the statement started (its continuations go one further).
    let mut statement = 0;
    // The line is a `struct` or `enum` heading: `struct Dog : Animal`.
    let mut heading = false;
    let mut prev: Option<(Token, bool)> = None; // and "is it a generic `<` or `>`?"
    for (k, t) in tokens.iter().enumerate() {
        let gap = &src[prev.map_or(0, |(p, _)| p.span.end)..t.span.start];
        let closer = matches!(t.kind, Kind::Sym("}" | ")" | "]"));
        let inside = open.last().map_or(0, |i| i + 1);
        let generic = match t.kind {
            Kind::Sym("<") => {
                let before = k.checked_sub(2).map(|j| tokens[j].kind);
                let named = prev.is_some_and(|(p, _)| p.kind == Kind::Type || (p.kind == Kind::Ident && before == Some(Kind::Keyword("fn"))));
                named && closes(&tokens[k + 1..])
            }
            Kind::Sym(">") => generics > 0,
            _ => false,
        };
        // A line continuing the statement of the line above: one step further.
        let operator = prev.is_some_and(|(p, g)| matches!(p.kind, Kind::Sym(s) if !g && !matches!(s, "{" | "(" | "[" | "," | ")" | "]" | "}" | "?" | "!" | ":")));
        let continues = operator || matches!(t.kind, Kind::Sym("." | "&&" | "||"));
        // `public:` and `private:` stand out, like the `}`; the contracts under a function's
        // heading go one step further.
        let label = matches!(t.kind, Kind::Keyword("public" | "private"));
        // (`mode build` too: `mode` followed by `run` or `build`).
        let only = t.kind == Kind::Ident
            && text(t) == "mode"
            && tokens.get(k + 1).is_some_and(|n| n.kind == Kind::Ident && matches!(text(n), "run" | "build"));
        let contract = matches!(t.kind, Kind::Keyword("require" | "ensure")) || only;
        let indent = match () {
            _ if closer || label => open.last().copied().unwrap_or(0),
            // `}` then `.on_key { … }`: a chain of blocks stays on one level.
            _ if t.kind == Kind::Sym(".") && prev.is_some_and(|(p, _)| p.kind == Kind::Sym("}")) => line,
            _ if continues => statement + 1,
            _ => inside + usize::from(contract),
        };
        let after_open = prev.is_some_and(|(p, _)| matches!(p.kind, Kind::Sym("{" | "(" | "[")));
        let blank = !(after_open || closer);
        if gap_out(&mut out, gap, inside, indent, blank) || prev.is_none() {
            line = indent;
            if !continues {
                statement = indent;
            }
            heading = matches!(t.kind, Kind::Keyword("struct" | "enum"));
        } else if let Some((p, g)) = prev
            && !(typed.last() == Some(&true) && (p.kind == Kind::Sym("{") || t.kind == Kind::Sym("}")))
            && space(src, p, g, t, generic, k.checked_sub(2).map(|j| &tokens[j]), generics > 0, heading)
        {
            out.push(' ');
        }
        out.push_str(text(t));
        match t.kind {
            Kind::Sym(s @ ("{" | "(" | "[")) => {
                open.push(line);
                // `x: {Int}`, and `{ {Int} -> Int }`'s inner block type.
                let in_type = typed.last() == Some(&true);
                typed.push(s == "{" && prev.is_some_and(|(p, _)| p.kind == Kind::Sym(":") || in_type && matches!(p.kind, Kind::Sym("{" | ","))));
                heading = false;
            }
            Kind::Sym("}" | ")" | "]") => {
                open.pop();
                typed.pop();
            }
            Kind::Sym("<") if generic => generics += 1,
            Kind::Sym(">") if generic => generics -= 1,
            _ => {}
        }
        prev = Some((*t, generic));
    }
    let end = prev.map_or(0, |(p, _)| p.span.end);
    gap_out(&mut out, &src[end..], 0, 0, false);
    let out = format!("{}\n", out.trim_end());
    // The safety net: the same tokens, in the same order.
    (words(&out) == words(src)).then_some(out)
}

/// The tokens of `src` with their text.
fn words(src: &str) -> Vec<(Kind, &str)> {
    raw(src).0.iter().map(|t| (t.kind, &src[t.span.start..t.span.end])).collect()
}

/// After `Type<`: is there a `>` closing it, with only a type in between (`Map<String, Int?>`)?
/// Otherwise it is a comparison: `Size.Small < Size.Big`.
fn closes(after: &[Token]) -> bool {
    let mut depth = 1;
    for t in after {
        match t.kind {
            Kind::Sym("<") => depth += 1,
            Kind::Sym(">") if depth == 1 => return true,
            Kind::Sym(">") => depth -= 1,
            Kind::Type | Kind::Ident | Kind::Sym("." | "," | "(" | ")" | "?" | ":" | "+") | Kind::Keyword("or" | "any") => {}
            _ => return false,
        }
    }
    false
}

/// Writes the gap between two tokens: its comments and its line breaks (at most one empty line
/// in a row, none if `blank` is false). `inside`: the indentation of a comment line, `indent`:
/// of the next token's line. True if the next token starts a line.
fn gap_out(out: &mut String, gap: &str, inside: usize, indent: usize, blank: bool) -> bool {
    let max = if blank { 2 } else { 1 };
    let mut breaks = 0;
    let mut i = 0;
    while let Some(c) = gap[i..].chars().next() {
        let rest = &gap[i..];
        let n = if rest.starts_with("//") {
            rest.find('\n').unwrap_or(rest.len())
        } else if rest.starts_with("/*") {
            comment_end(rest).unwrap_or(rest.len())
        } else {
            breaks += usize::from(c == '\n');
            i += c.len_utf8();
            continue;
        };
        if breaks == 0 && !out.is_empty() {
            out.push(' ');
        } else if !out.is_empty() {
            out.push_str(&"\n".repeat(breaks.min(2)));
            out.push_str(&INDENT.repeat(inside));
        }
        out.push_str(rest[..n].trim_end());
        breaks = 0;
        i += n;
    }
    if breaks > 0 && !out.is_empty() {
        out.push_str(&"\n".repeat(breaks.min(max)));
        out.push_str(&INDENT.repeat(indent));
        return true;
    }
    out.is_empty()
}

/// Is there a space between `a` and `b` on the same line? `g`: `a` is a generic `<` or `>`;
/// `generic`: `b` is one; `before`: the token before `a`; `in_generics`: between `<` and `>`;
/// `heading`: on a `struct` or `enum` heading.
fn space(source: &str, a: Token, g: bool, b: &Token, generic: bool, before: Option<&Token>, in_generics: bool, heading: bool) -> bool {
    // `-x`, `!done`: a sign, not a subtraction.
    let value_before = |t: Option<&Token>| match t.map(|t| t.kind) {
        Some(Kind::Sym(s)) => matches!(s, ")" | "]" | "}" | "?"),
        Some(Kind::Keyword(k)) => matches!(k, "self" | "result" | "true" | "false" | "none"),
        Some(_) => true,
        None => false,
    };
    let sign = match a.kind {
        Kind::Sym("!") => true,
        // `for i in 3..=1 step -1`: `step - 1` stays a subtraction on a variable.
        Kind::Sym("-") => !value_before(before) || (a.span.end == b.span.start && before.is_some_and(|t| &source[t.span.start..t.span.end] == "step" && t.span.end < a.span.start)),
        _ => false,
    };
    // `r.header("Content-Type")`, `files.move(a)`: after a dot a reserved word is a name; and
    // `to(b)` after a `)` is a part of a keyword message (`files.copy(a) to(b)`).
    let method = matches!(a.kind, Kind::Keyword(_)) && before.is_some_and(|t| t.kind == Kind::Sym("."))
        || a.kind == Kind::Keyword("to") && b.kind == Kind::Sym("(") && before.is_some_and(|t| t.kind == Kind::Sym(")"));
    let named = matches!(a.kind, Kind::Ident | Kind::Type) || g || method || matches!(a.kind, Kind::Sym(")" | "]"));
    match (a.kind, b.kind) {
        _ if sign || g && a.kind == Kind::Sym("<") || generic => false,
        (_, Kind::Sym("," | ")" | "]" | "?" | "." | ".." | "..=")) | (Kind::Sym("(" | "[" | "." | ".." | "..=" | "..."), _) => false,
        // `struct Dog : Animal`, but `x: Int` and `<T: Comparable>`.
        (_, Kind::Sym(":")) => heading && !in_generics,
        // `print(x)`, `ok(value)`, `List<Int>()`; `l[0]`, `self.items[0]`.
        (_, Kind::Sym("(")) => !(named || matches!(a.kind, Kind::Keyword("ok" | "err"))),
        (_, Kind::Sym("[")) => !(named || matches!(a.kind, Kind::Str | Kind::Int(_)) || matches!(a.kind, Kind::Keyword("self" | "result"))),
        (Kind::Sym("{"), Kind::Sym("}")) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn layout() {
        let messy = "fn main(){\nlet x:Int:=-1+2\n  print( x )\n\n\n\nlet l:List<Int>:=[1,-x]\nprint(l[0]+l.length)\n}\n";
        let tidy = "fn main() {\n    let x: Int := -1 + 2\n    print(x)\n\n    let l: List<Int> := [1, -x]\n    print(l[0] + l.length)\n}\n";
        // `fn main()` is an error for the compiler, not for the layout; line breaks never move.
        assert_eq!(format(messy).as_deref(), Some(tidy));
        assert_eq!(format(tidy).as_deref(), Some(tidy));
    }

    #[test]
    fn reserved_words_as_names() {
        let src = "fn main {\n    f(files.copy (a) to (b))\n    files.move (a) to(b)\n    from i := 1 to 3 {\n    }\n    f(move a)\n}\n";
        let tidy = "fn main {\n    f(files.copy(a) to(b))\n    files.move(a) to(b)\n    from i := 1 to 3 {\n    }\n    f(move a)\n}\n";
        assert_eq!(format(src).as_deref(), Some(tidy));
    }

    #[test]
    fn comments_and_continuations() {
        let src = "// top\n\n\nstruct Dog:Animal{   // a dog\n    name:String\n\n\n    /* old */ age:Int\n// end\n}\nfn f<T:Comparable>(a:T)->T{\nreturn a\n    .clone\n}\nfn g -> Int {\n    return 1 +\n2\n}\n";
        let tidy = "// top\n\nstruct Dog : Animal { // a dog\n    name: String\n\n    /* old */ age: Int\n    // end\n}\nfn f<T: Comparable>(a: T) -> T {\n    return a\n        .clone\n}\nfn g -> Int {\n    return 1 +\n        2\n}\n";
        assert_eq!(format(src).as_deref(), Some(tidy));
    }

    #[test]
    fn unreadable_stays() {
        assert_eq!(format("fn main {\n  print(\"oops)\n}\n"), None);
    }

    /// Every program of the tests: the layout changes nothing a second time, and the programs
    /// that compile are already laid out (they show the style).
    #[test]
    fn test_programs() {
        let mut changed = vec![];
        for dir in ["tests/ok", "tests/error", "tests/web", "tests/modes", "tests/parallel", "locales", "editors/vscode"] {
            // The colors' test: its comments point at columns.
            for f in walk(std::path::Path::new(dir)).into_iter().filter(|f| !f.ends_with("colors.kova")) {
                let src = std::fs::read_to_string(&f).unwrap();
                let Some(once) = format(&src) else { continue };
                assert_eq!(format(&once).as_deref(), Some(once.as_str()), "{}", f.display());
                if once != src && !dir.starts_with("tests/error") {
                    changed.push(f.display().to_string());
                }
            }
        }
        assert!(changed.is_empty(), "not laid out: {changed:?}");
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files = vec![];
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() && !p.ends_with("node_modules") {
                files.extend(walk(&p));
            } else if p.extension().is_some_and(|x| x == "kova") {
                files.push(p);
            }
        }
        files
    }
}
