// Parser: turns the token list into a tree (AST).
//
// It doesn't stop at the first error: it records it, skips to the end of the
// statement, then continues. So all errors are seen at once.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{Kind, Token, code_end, escape, interpolation, lex, string_content};
use crate::locale::t;

/// `Err(())` = an error was already added to `diags`; the caller resynchronizes.
type R<T> = Result<T, ()>;

/// Binary operators, from lowest to highest precedence (PLAN 2.4).
/// `false` = not chainable (`a < b < c` is refused).
const LEVELS: &[(&[(&str, BinOp)], bool)] = &[
    (&[("||", BinOp::Or)], true),
    (&[("&&", BinOp::And)], true),
    (&[("=", BinOp::Eq), ("!=", BinOp::Ne)], false),
    (&[("<", BinOp::Lt), ("<=", BinOp::Le), (">", BinOp::Gt), (">=", BinOp::Ge)], false),
    (&[("..", BinOp::Range), ("..=", BinOp::RangeInclusive)], false),
    (&[("+", BinOp::Plus), ("-", BinOp::Minus)], true),
    (&[("*", BinOp::Times), ("/", BinOp::Div), ("%", BinOp::Rem)], true),
];

const ASSIGNMENTS: &[(&str, Option<BinOp>)] = &[
    (":=", None),
    ("+=", Some(BinOp::Plus)),
    ("-=", Some(BinOp::Minus)),
    ("*=", Some(BinOp::Times)),
    ("/=", Some(BinOp::Div)),
];

/// Returns the tree, the number of expressions (for the type table) and the errors.
/// The interfaces every program knows (PLAN Part 4, "J5"). `Equal` without `equals` compares
/// the fields one by one (`types::Checker::conforms`).
const PRELUDE: &str = "interface Equal {
    fn equals(self, other: Self) -> Bool
}
interface Comparable {
    fn less(self, other: Self) -> Bool
}
interface Display {
    fn to_string(self) -> String
}
";

fn prelude() -> Vec<Interface> {
    let (tokens, _) = lex(PRELUDE);
    let mut p = Parser {
        t: tokens,
        pos: 0,
        src: PRELUDE,
        diags: vec![],
        ids: 0,
        no_params: HashSet::new(),
        no_literal: false,
        in_interface: false,
        bodiless: false,
        module: None,
        own: HashSet::new(),
        imports: HashMap::new(),
    };
    let mut interfaces = p.file().interfaces;
    debug_assert!(p.diags.is_empty() && p.ids == 0, "the prelude has no expressions");
    interfaces.iter_mut().for_each(|i| i.builtin = true);
    interfaces
}

pub fn parse(tokens: Vec<Token>, source: &str) -> (File, usize, Vec<Diag>) {
    parse_in(tokens, source, 0, None)
}

/// Reads one file of a program. `first_id`: the number of its first expression (the numbers
/// go on from file to file). `module`: the file's name when it is imported (`geometry`); its
/// own names are then written with it (`geometry.Point`), so that two files never share one.
pub fn parse_in(tokens: Vec<Token>, source: &str, first_id: usize, module: Option<&str>) -> (File, usize, Vec<Diag>) {
    // The functions without parameters, wherever they are declared: a bare `main` calls `fn main`.
    // ponytail: a scan of `fn name` tokens outside any `{ }` (a struct's methods are not in it).
    let mut depth = 0;
    let outside: Vec<bool> = tokens
        .iter()
        .map(|t| {
            match t.kind {
                Kind::Sym("{") => depth += 1,
                Kind::Sym("}") => depth -= 1,
                _ => {}
            }
            depth == 0
        })
        .collect();
    let no_params = tokens
        .windows(4)
        .zip(&outside)
        .filter(|(w, out)| **out && w[0].kind == Kind::Keyword("fn") && w[1].kind == Kind::Ident)
        .filter(|(w, _)| !matches!(w[2].kind, Kind::Sym("(" | "<")) || w[3].kind == Kind::Sym(")"))
        .map(|(w, _)| source[w[1].span.start..w[1].span.end].to_string())
        .chain(["read_line".to_string()])
        .collect();
    let text = |t: &Token| source[t.span.start..t.span.end].to_string();
    let at_top = |i: usize| i == 0 || outside[i - 1];
    // `import geometry as geo`: `geo.Point` means `geometry.Point` in this file.
    let mut imports = HashMap::new();
    for (i, w) in tokens.windows(4).enumerate() {
        if at_top(i) && w[0].kind == Kind::Keyword("import") && w[1].kind == Kind::Ident {
            let alias = if w[2].kind == Kind::Ident && text(&w[2]) == "as" && w[3].kind == Kind::Ident { &w[3] } else { &w[1] };
            imports.insert(text(alias), text(&w[1]));
        }
    }
    // The names this file declares at its top: `fn distance`, `struct Point`, `copy struct`, `enum`, `interface`.
    let own = (0..tokens.len().saturating_sub(1))
        .filter(|&i| at_top(i))
        .filter(|&i| match (tokens[i].kind, tokens[i + 1].kind) {
            (Kind::Keyword("fn"), Kind::Ident) => true,
            (Kind::Keyword("struct" | "enum" | "interface"), Kind::Type) => true,
            _ => false,
        })
        .map(|i| text(&tokens[i + 1]))
        .collect();
    let module = module.map(str::to_string);
    let mut p = Parser {
        t: tokens,
        pos: 0,
        src: source,
        diags: vec![],
        ids: first_id,
        no_params,
        no_literal: false,
        in_interface: false,
        bodiless: false,
        own: if module.is_some() { own } else { HashSet::new() },
        module,
        imports,
    };
    let mut file = p.file();
    if p.module.is_none() {
        file.interfaces.splice(0..0, prelude());
    }
    (file, p.ids, p.diags)
}

struct Parser<'a> {
    t: Vec<Token>,
    pos: usize,
    src: &'a str,
    diags: Vec<Diag>,
    ids: usize,
    no_params: HashSet<String>,
    /// In a heading (`if`, `while`…): `Dog {` is not a struct literal, the `{` opens the body (PLAN 2.3).
    no_literal: bool,
    /// The name of this file when it is imported (`geometry`), and the names it declares:
    /// inside it, `Point` is `geometry.Point`.
    module: Option<String>,
    own: HashSet<String>,
    /// `import geometry as geo`: `geo` → `geometry`.
    imports: HashMap<String, String>,
    /// In an `interface`: a method may have no body (`function_rest` then sets `bodiless`).
    in_interface: bool,
    bodiless: bool,
}

impl Parser<'_> {
    // ---- Tools ----

    fn token(&self) -> Token {
        self.t[self.pos]
    }
    fn kind(&self) -> Kind {
        self.token().kind
    }
    fn advance(&mut self) -> Token {
        let t = self.token();
        if t.kind != Kind::Eof {
            self.pos += 1;
        }
        t
    }
    fn is_sym(&self, s: &str) -> bool {
        matches!(self.kind(), Kind::Sym(x) if x == s)
    }
    fn is_keyword(&self, k: &str) -> bool {
        matches!(self.kind(), Kind::Keyword(x) if x == k)
    }
    fn skip_newlines(&mut self) {
        while self.kind() == Kind::Newline {
            self.pos += 1;
        }
    }
    fn text(&self, span: Span) -> String {
        self.src[span.start..span.end].to_string()
    }
    /// How to name the current token in a message.
    fn describe(&self) -> String {
        match self.kind() {
            Kind::Newline => t!("found_newline"),
            Kind::Eof => t!("found_eof"),
            _ => format!("`{}`", self.text(self.token().span)),
        }
    }
    fn error<T>(&mut self, span: Span, msg: impl Into<String>) -> R<T> {
        self.diags.push(Diag::new(span, msg));
        Err(())
    }
    fn expect(&mut self, s: &str) -> R<Span> {
        if self.is_sym(s) {
            return Ok(self.advance().span);
        }
        let msg = t!("expected_symbol", symbol = s, found = self.describe());
        self.error(self.token().span, msg)
    }
    fn expr(&mut self, span: Span, kind: ExprKind) -> Expr {
        self.ids += 1;
        Expr { id: self.ids - 1, span, kind }
    }

    fn alias_taken(&mut self, name: &Name, what: &str) {
        if self.imports.contains_key(&name.text) {
            self.diags.push(Diag::new(name.span, t!("module_name_taken", name = name.text, what = what)));
        }
    }

    /// A variable or function name: lowercase required. `what` = the key of its kind (`what_variable`).
    fn value_name(&mut self, key: &str) -> R<Name> {
        let what = crate::locale::text(key, &[]);
        let t = self.token();
        match t.kind {
            Kind::Ident => {
                self.advance();
                let text = self.text(t.span);
                // `geo.area` must stay the module's: a variable or a parameter cannot take
                // the name of an import (a function: see `function`).
                if matches!(key, "what_variable" | "what_parameter") {
                    self.alias_taken(&Name { text: text.clone(), span: t.span }, &what);
                }
                Ok(Name { text, span: t.span })
            }
            Kind::Type => {
                let text = self.text(t.span);
                let mut c = text.chars();
                let lowercase: String = c.next().unwrap().to_lowercase().chain(c).collect();
                self.diags.push(
                    Diag::new(t.span, t!("name_lowercase", what = what)).help(t!("name_lowercase_help", name = lowercase)),
                );
                Err(())
            }
            Kind::Keyword(k) => {
                let msg = t!("reserved_word", word = k);
                self.error(t.span, msg)
            }
            _ => {
                let msg = t!("name_expected", what = what, found = self.describe());
                self.error(t.span, msg)
            }
        }
    }

    /// A name declared at this file's top, as the whole program knows it: `geometry.Point`.
    fn qualified(&self, text: String) -> String {
        match &self.module {
            Some(m) if self.own.contains(&text) => format!("{m}.{text}"),
            _ => text,
        }
    }

    /// `geo.` before a name: the file imported as `geo`, whose name is returned (`geometry`).
    fn import_prefix(&self) -> Option<String> {
        let t = self.token();
        if t.kind != Kind::Ident || self.t[self.pos + 1].kind != Kind::Sym(".") {
            return None;
        }
        self.imports.get(&self.text(t.span)).cloned()
    }

    /// A type name: uppercase required. `Point` declared here, or `geo.Point` from an import,
    /// as the whole program knows it (`geometry.Point`).
    fn type_name(&mut self) -> R<Name> {
        if let Some(module) = self.import_prefix()
            && self.t[self.pos + 2].kind == Kind::Type
        {
            let start = self.advance().span;
            self.advance(); // .
            let t = self.advance();
            return Ok(Name { text: format!("{module}.{}", self.text(t.span)), span: start.to(t.span) });
        }
        let name = self.plain_type_name()?;
        Ok(Name { text: self.qualified(name.text), span: name.span })
    }

    /// A type name as written, never qualified: an enum's variant (`Circle`), a type parameter (`T`).
    fn plain_type_name(&mut self) -> R<Name> {
        let t = self.token();
        match t.kind {
            Kind::Type => {
                self.advance();
                Ok(Name { text: self.text(t.span), span: t.span })
            }
            Kind::Ident => self.error(t.span, t!("type_uppercase")),
            _ => {
                let msg = t!("type_expected", found = self.describe());
                self.error(t.span, msg)
            }
        }
    }

    /// A type, maybe a result: `String or IoError` (a single `or`).
    fn type_(&mut self) -> R<TypeAst> {
        let t = self.maybe_type()?;
        if !self.is_keyword("or") {
            return Ok(t);
        }
        self.advance();
        let e = self.maybe_type()?;
        if self.is_keyword("or") {
            let span = self.token().span;
            return self.error(span, t!("result_chain"));
        }
        Ok(TypeAst::Result(Box::new(t), Box::new(e)))
    }

    /// `Int`, `(Int, Float)`, `List<Int>`, each maybe followed by `?` (`Int?`).
    fn maybe_type(&mut self) -> R<TypeAst> {
        let t = self.base_type()?;
        if !self.is_sym("?") {
            return Ok(t);
        }
        let mark = self.advance().span;
        if self.is_sym("?") {
            let span = self.token().span;
            return self.error(mark.to(span), t!("optional_twice", type = t.text()));
        }
        Ok(TypeAst::Optional(Box::new(t)))
    }

    fn base_type(&mut self) -> R<TypeAst> {
        // `any Animal`: a value of any type following the interface `Animal`.
        if self.is_keyword("any") {
            self.advance();
            return Ok(TypeAst::Any(self.type_name()?));
        }
        if !self.is_sym("(") {
            let name = self.type_name()?;
            return if self.is_sym("<") { self.type_arguments(name) } else { Ok(TypeAst::Name(name)) };
        }
        let open = self.advance().span;
        let mut types = vec![self.type_()?];
        while self.is_sym(",") {
            self.advance();
            types.push(self.type_()?);
        }
        let close = self.expect(")")?;
        if types.len() < 2 {
            return self.error(open.to(close), t!("tuple_too_small"));
        }
        Ok(TypeAst::Tuple(types))
    }

    /// `<Int>` after `List`: a capitalized name followed by `<` opens generics (PLAN 2.3).
    fn type_arguments(&mut self, name: Name) -> R<TypeAst> {
        self.advance(); // <
        let mut types = vec![self.type_()?];
        while self.is_sym(",") {
            self.advance();
            types.push(self.type_()?);
        }
        self.expect(">")?;
        Ok(TypeAst::Generic(name, types))
    }

    /// `x`, `_` or `(a, b)`.
    fn pattern(&mut self) -> R<Pattern> {
        if self.kind() == Kind::Ident && self.text(self.token().span) == "_" {
            return Ok(Pattern::Ignore(self.advance().span));
        }
        if !self.is_sym("(") {
            return Ok(Pattern::Name(self.value_name("what_variable")?));
        }
        let open = self.advance().span;
        let mut patterns = vec![self.pattern()?];
        while self.is_sym(",") {
            self.advance();
            patterns.push(self.pattern()?);
        }
        let close = self.expect(")")?;
        if patterns.len() < 2 {
            return self.error(open.to(close), t!("tuple_too_small"));
        }
        Ok(Pattern::Tuple(patterns, open.to(close)))
    }

    /// `x`, `t.0`, `rex.age`, `l[i]` or `(a, b)` left of `:=`: re-reads the expression already parsed.
    fn target(&mut self, e: Expr) -> R<Target> {
        let span = e.span;
        let (base, step) = match e.kind {
            ExprKind::Var(name) => return Ok(Target::Place { name: Name { text: name, span }, fields: vec![], span }),
            ExprKind::Field(base, i) => (base, Step::Index(i)),
            ExprKind::Index(base, i) => (base, Step::At(*i)),
            // `rex.age`: a field (checked by the type checker); `rex.age(1)` is a call.
            ExprKind::Method { receiver, name, args } if args.is_empty() => (receiver, Step::Name(name)),
            ExprKind::Tuple(elems) => {
                let targets = elems.into_iter().map(|e| self.target(e)).collect::<R<Vec<_>>>()?;
                return Ok(Target::Tuple(targets, span));
            }
            _ => return self.error(span, t!("assign_target")),
        };
        match self.target(*base)? {
            Target::Place { name, mut fields, .. } => {
                fields.push(step);
                Ok(Target::Place { name, fields, span })
            }
            Target::Tuple(..) => self.error(span, t!("assign_target")),
        }
    }

    /// `while cond {`: the expression before a body.
    fn heading_expr(&mut self) -> R<Expr> {
        let saved = std::mem::replace(&mut self.no_literal, true);
        let e = self.expression();
        self.no_literal = saved;
        e
    }

    /// An expression between delimiters (`( )`, `{ }`): struct literals are allowed again.
    fn inner_expr(&mut self) -> R<Expr> {
        let saved = std::mem::replace(&mut self.no_literal, false);
        let e = self.expression();
        self.no_literal = saved;
        e
    }

    // ---- File and functions ----

    fn file(&mut self) -> File {
        let (mut header, mut functions, mut structs, mut tests, mut langs) = (None, vec![], vec![], vec![], vec![]);
        let (mut interfaces, mut imports) = (vec![], vec![]);
        loop {
            self.skip_newlines();
            let read = match self.kind() {
                Kind::Eof => break,
                Kind::Keyword("header") => {
                    let first = header.is_none() && functions.is_empty() && structs.is_empty() && tests.is_empty();
                    if !first {
                        let span = self.token().span;
                        self.diags.push(Diag::new(span, t!("header_first")));
                    }
                    self.header().map(|h| {
                        if first {
                            header = Some(h);
                        }
                    })
                }
                Kind::Keyword("import") => self.import().map(|i| imports.push(i)),
                // `public:` / `private:` are read but change nothing yet (J6, step 2).
                Kind::Keyword("public" | "private") if self.t[self.pos + 1].kind == Kind::Sym(":") => {
                    self.advance();
                    self.advance();
                    Ok(())
                }
                Kind::Keyword("fn") => self.function(false).map(|f| functions.push(f)),
                Kind::Keyword("struct" | "copy" | "enum") => self.struct_().map(|s| structs.push(s)),
                Kind::Keyword("interface") => self.interface().map(|i| interfaces.push(i)),
                Kind::Keyword("test") => self.test().map(|t| tests.push(t)),
                // `lang` is not a reserved word: it only means something here.
                Kind::Ident if self.text(self.token().span) == "lang" => self.lang().map(|l| langs.push(l)),
                _ => {
                    let msg = t!("declaration_expected", found = self.describe());
                    self.error(self.token().span, msg)
                }
            };
            if read.is_err() {
                self.skip_to_next_fn();
            }
        }
        File { header, imports, functions, structs, interfaces, tests, langs }
    }

    /// `import geometry` or `import geometry as geo`.
    fn import(&mut self) -> R<Import> {
        self.advance(); // import
        let module = self.value_name("what_module")?;
        if self.is_sym(".") {
            let span = self.token().span;
            return self.error(module.span.to(span), t!("import_folder_later"));
        }
        let alias = if self.kind() == Kind::Ident && self.text(self.token().span) == "as" {
            self.advance();
            self.value_name("what_module")?
        } else {
            module.clone()
        };
        Ok(Import { module, alias })
    }

    /// `header { key := value ... }`; keys and values are checked by the type checker.
    fn header(&mut self) -> R<Header> {
        self.advance(); // header
        self.skip_newlines();
        let open = self.expect("{")?;
        let mut settings = vec![];
        loop {
            self.skip_newlines();
            if self.is_sym("}") {
                break;
            }
            if matches!(self.kind(), Kind::Eof | Kind::Keyword("fn" | "test")) {
                let span = self.token().span;
                self.diags.push(Diag::new(span, t!("header_unclosed")));
                return Ok(Header { open, close: span, settings });
            }
            match self.setting() {
                Ok(s) => settings.push(s),
                Err(()) => self.skip_statement(),
            }
        }
        let close = self.advance().span;
        Ok(Header { open, close, settings })
    }

    /// `mode := build`: a key, `:=`, a single word.
    fn setting(&mut self) -> R<(Name, Name)> {
        let key = self.value_name("what_setting")?;
        self.expect(":=")?;
        let t = self.token();
        if !matches!(t.kind, Kind::Ident | Kind::Keyword(_)) {
            let msg = t!("setting_value_expected", found = self.describe());
            return self.error(t.span, msg);
        }
        self.advance();
        if !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
            let msg = t!("one_setting_per_line", found = self.describe());
            return self.error(self.token().span, msg);
        }
        Ok((key, Name { text: self.text(t.span), span: t.span }))
    }

    fn skip_to_next_fn(&mut self) {
        self.advance();
        while !matches!(self.kind(), Kind::Keyword("fn" | "test" | "struct" | "copy" | "enum" | "interface") | Kind::Eof)
            && !self.at_lang()
        {
            self.advance();
        }
    }

    fn at_lang(&self) -> bool {
        self.kind() == Kind::Ident && self.text(self.token().span) == "lang"
    }

    /// Texts are plain until J3: `{x}` is refused in a test name or a contract message.
    fn no_interpolation(&mut self, t: Token) {
        if let Some(k) = interpolation(&self.text(t.span)) {
            let at = t.span.start + k;
            self.diags.push(Diag::new(Span::new(at, at + 1), t!("test_name_fixed")));
        }
    }

    /// The parts of a `Str` token: `"a{x}b"` → the text `a`, the expression `x`, the text `b`.
    fn text_parts(&mut self, t: Token) -> Vec<Part> {
        let start = t.span.start + 1;
        let end = if t.span.end > start && self.src[..t.span.end].ends_with('"') { t.span.end - 1 } else { t.span.end };
        let inner = &self.src[start..end.max(start)];
        let (mut parts, mut text) = (vec![], String::new());
        let mut it = inner.char_indices();
        while let Some((k, c)) = it.next() {
            match c {
                '\\' => text.push(escape(&mut it).unwrap_or('?')),
                '}' => self.diags.push(Diag::new(Span::new(start + k, start + k + 1), t!("brace_alone"))),
                '{' => {
                    // Unclosed: already reported by the lexer.
                    let Some(n) = code_end(&inner[k + 1..]) else { break };
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    if let Some(e) = self.code(start + k + 1, start + k + 1 + n) {
                        parts.push(Part::Code(e));
                    }
                    while it.clone().next().is_some_and(|(j, _)| j <= k + 1 + n) {
                        it.next();
                    }
                }
                c => text.push(c),
            }
        }
        if !text.is_empty() || parts.is_empty() {
            parts.push(Part::Text(text));
        }
        parts
    }

    /// The expression of a `{code}` in a text, at `src[a..b]`: read with its own tokens.
    fn code(&mut self, a: usize, b: usize) -> Option<Expr> {
        if self.src[a..b].trim().is_empty() {
            self.diags.push(Diag::new(Span::new(a - 1, b + 1), t!("interpolation_empty")));
            return None;
        }
        let shift = |s: Span| Span::new(s.start + a, s.end + a);
        let (tokens, diags) = lex(&self.src[a..b]);
        self.diags.extend(diags.into_iter().map(|d| Diag { span: shift(d.span), ..d }));
        // The end of the code is the closing `}` (so that messages say "found `}`").
        let tokens = tokens
            .into_iter()
            .map(|t| match t.kind {
                Kind::Eof => Token { kind: Kind::Sym("}"), span: Span::new(b, b + 1) },
                _ => Token { span: shift(t.span), ..t },
            })
            .chain([Token { kind: Kind::Eof, span: Span::new(b + 1, b + 1) }])
            .collect();
        let saved = (std::mem::replace(&mut self.t, tokens), self.pos);
        self.pos = 0;
        let e = self.inner_expr().ok();
        self.skip_newlines();
        if e.is_some() && !self.is_sym("}") {
            let msg = t!("expected_symbol", symbol = "}", found = self.describe());
            self.diags.push(Diag::new(self.token().span, msg));
        }
        (self.t, self.pos) = saved;
        e
    }

    /// `lang fr { key := "text" ... }`: one translation per line (PLAN 2.13).
    fn lang(&mut self) -> R<Lang> {
        self.advance(); // lang
        let name = self.value_name("what_language")?;
        self.skip_newlines();
        self.expect("{")?;
        let mut entries = vec![];
        loop {
            self.skip_newlines();
            if self.is_sym("}") {
                self.advance();
                return Ok(Lang { name, entries });
            }
            if self.kind() == Kind::Eof {
                let span = self.token().span;
                self.diags.push(Diag::new(span, t!("lang_unclosed")));
                return Ok(Lang { name, entries });
            }
            match self.entry() {
                Ok(e) => entries.push(e),
                Err(()) => self.skip_statement(),
            }
        }
    }

    /// `already_exists := "`{name}` already exists"`.
    fn entry(&mut self) -> R<(Name, Span)> {
        let key = self.value_name("what_key")?;
        self.expect(":=")?;
        let t = self.token();
        if t.kind != Kind::Str {
            let msg = t!("lang_text_expected", found = self.describe());
            return self.error(t.span, msg);
        }
        self.advance();
        if !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
            let msg = t!("lang_one_per_line", found = self.describe());
            return self.error(self.token().span, msg);
        }
        Ok((key, t.span))
    }

    /// `struct Dog { name: String, age: Int }`: fields separated by `,` or a line break.
    fn struct_(&mut self) -> R<Struct> {
        let copy = self.is_keyword("copy");
        if copy {
            self.advance();
            if !self.is_keyword("struct") {
                let msg = t!("expected_symbol", symbol = "struct", found = self.describe());
                return self.error(self.token().span, msg);
            }
        }
        // An enum lists variants where a struct lists fields.
        let mut variants = self.is_keyword("enum").then(Vec::new);
        self.advance(); // struct, enum
        let name = self.type_name()?;
        let generics = self.generics()?;
        if variants.is_some() && !generics.is_empty() {
            self.diags.push(Diag::new(generics[0].name.span, t!("generic_enum_later")));
        }
        let mut interfaces = vec![];
        if self.is_sym(":") {
            self.advance();
            interfaces.push(self.type_name()?);
            while self.is_sym(",") {
                self.advance();
                interfaces.push(self.type_name()?);
            }
        }
        self.skip_newlines();
        self.expect("{")?;
        let (mut fields, mut methods) = (vec![], vec![]);
        loop {
            self.skip_newlines();
            match self.kind() {
                Kind::Sym("}") => break,
                Kind::Eof | Kind::Keyword("test" | "struct" | "copy" | "enum" | "interface") => {
                    let span = self.token().span;
                    self.diags.push(Diag::new(span, t!("struct_unclosed", name = name.text)));
                    return Ok(Struct { copy, name, generics, interfaces, fields, variants, methods });
                }
                // `public:` / `private:` are read but change nothing yet: a program is a single file (J6).
                Kind::Keyword("public" | "private") if self.t[self.pos + 1].kind == Kind::Sym(":") => {
                    self.advance();
                    self.advance();
                }
                Kind::Keyword("fn") => match self.function(true) {
                    Ok(f) => methods.push(f),
                    Err(()) => self.skip_statement(),
                },
                _ if variants.is_some() => match self.variant() {
                    Ok(v) => variants.as_mut().unwrap().push(v),
                    Err(()) => self.skip_statement(),
                },
                _ => match self.field() {
                    Ok(f) => fields.push(f),
                    Err(()) => self.skip_statement(),
                },
            }
        }
        self.advance(); // }
        Ok(Struct { copy, name, generics, interfaces, fields, variants, methods })
    }

    /// `interface Animal { fn sound(self) -> String … }`: signatures, and default methods with a body.
    fn interface(&mut self) -> R<Interface> {
        self.advance(); // interface
        let name = self.type_name()?;
        self.skip_newlines();
        self.expect("{")?;
        let mut methods = vec![];
        loop {
            self.skip_newlines();
            match self.kind() {
                Kind::Sym("}") => break,
                Kind::Keyword("fn") => {
                    self.in_interface = true;
                    let f = self.function(true);
                    self.in_interface = false;
                    match f {
                        Ok(f) => methods.push((f, !std::mem::take(&mut self.bodiless))),
                        Err(()) => self.skip_statement(),
                    }
                }
                Kind::Eof | Kind::Keyword("test" | "struct" | "copy" | "enum" | "interface") => {
                    let span = self.token().span;
                    self.diags.push(Diag::new(span, t!("struct_unclosed", name = name.text)));
                    return Ok(Interface { name, builtin: false, methods });
                }
                _ => {
                    let msg = t!("interface_item", found = self.describe());
                    self.diags.push(Diag::new(self.token().span, msg));
                    self.skip_statement();
                }
            }
        }
        self.advance(); // }
        Ok(Interface { name, builtin: false, methods })
    }

    /// `Circle(radius: Float)` or `Red`, followed by `,`, a line break or the `}`.
    fn variant(&mut self) -> R<Variant> {
        let name = self.plain_type_name()?;
        let mut fields = vec![];
        if self.is_sym("(") {
            let open = self.advance().span;
            while !self.is_sym(")") {
                let name = self.value_name("what_field")?;
                self.expect(":")?;
                fields.push(Param { name, mode: Mode::Read, ty: self.type_()? });
                if !self.is_sym(")") {
                    self.expect(",")?;
                }
            }
            let close = self.advance().span;
            if fields.is_empty() {
                self.diags.push(Diag::new(open.to(close), t!("empty_parens_variant", name = name.text)));
            }
        }
        self.separator()?;
        Ok(Variant { name, fields })
    }

    /// After an item of a list between `{ }` (variant, `match` arm): `,`, a line break or the `}`.
    fn separator(&mut self) -> R<()> {
        if self.is_sym(",") {
            self.advance();
        } else if !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
            let msg = t!("expected_symbol", symbol = ",", found = self.describe());
            return self.error(self.token().span, msg);
        }
        Ok(())
    }

    /// `name: String`, followed by `,`, a line break or the `}`.
    fn field(&mut self) -> R<Param> {
        let name = self.value_name("what_field")?;
        self.expect(":")?;
        let ty = self.type_()?;
        // `tricks: List<String>` at the end of a line: the lexer took `>` for an operator
        // that continues the line, so the line break is only in the source.
        let line_break = self.src[self.t[self.pos - 1].span.end..self.token().span.start].contains('\n');
        if self.is_sym(",") {
            self.advance();
        } else if !line_break && !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
            let msg = t!("expected_symbol", symbol = ",", found = self.describe());
            return self.error(self.token().span, msg);
        }
        Ok(Param { name, mode: Mode::Read, ty })
    }

    fn test(&mut self) -> R<Test> {
        self.advance(); // test
        let t = self.token();
        if t.kind != Kind::Str {
            let msg = t!("test_name_expected", found = self.describe());
            self.diags.push(Diag::new(t.span, msg).help(t!("test_name_help")));
            return Err(());
        }
        self.advance();
        self.no_interpolation(t);
        self.skip_newlines();
        let body = self.block()?;
        Ok(Test { name: string_content(&self.text(t.span)), span: t.span, body })
    }

    /// `fn name(params) -> Type`; `in_struct`: the first parameter may be `self`.
    fn function(&mut self, in_struct: bool) -> R<Function> {
        self.advance(); // fn
        let mut name = self.value_name("what_function")?;
        if !in_struct {
            self.alias_taken(&name, &crate::locale::text("what_function", &[]));
            name.text = self.qualified(name.text);
        }
        let generics = self.generics()?;
        let mut params = vec![];
        if !self.is_sym("(") {
            return self.function_rest(name, generics, None, params);
        }
        let open = self.advance().span;
        if self.is_sym(")") {
            let span = open.to(self.token().span);
            self.diags.push(Diag::new(span, t!("empty_parens_fn", name = name.text)));
        }
        let receiver = self.receiver(in_struct)?;
        if receiver.is_some() && !self.is_sym(")") {
            self.expect(",")?;
        }
        while !self.is_sym(")") {
            let name = self.value_name("what_parameter")?;
            self.expect(":")?;
            let mode = self.mode();
            params.push(Param { name, mode, ty: self.type_()? });
            if !self.is_sym(")") {
                self.expect(",")?;
            }
        }
        self.advance(); // )
        self.function_rest(name, generics, receiver, params)
    }

    /// `<T: Comparable + Display, U>` after a function's name (nothing if there is no `<`).
    fn generics(&mut self) -> R<Vec<Generic>> {
        let mut generics = vec![];
        if !self.is_sym("<") {
            return Ok(generics);
        }
        self.advance();
        loop {
            let name = self.plain_type_name()?;
            let mut bounds = vec![];
            if self.is_sym(":") {
                self.advance();
                bounds.push(self.type_name()?);
                while self.is_sym("+") {
                    self.advance();
                    bounds.push(self.type_name()?);
                }
            }
            generics.push(Generic { name, bounds });
            if !self.is_sym(",") {
                break;
            }
            self.advance();
        }
        self.expect(">")?;
        Ok(generics)
    }

    /// `self`, `var self` or `move self` at the start of the parameters.
    fn receiver(&mut self, in_struct: bool) -> R<Option<Mode>> {
        let next_is_self = matches!(self.t[self.pos + 1].kind, Kind::Keyword("self"));
        let (r, n) = match self.kind() {
            Kind::Keyword("self") => (Mode::Read, 1),
            Kind::Keyword("var") if next_is_self => (Mode::Var, 2),
            Kind::Keyword("move") if next_is_self => (Mode::Move, 2),
            _ => return Ok(None),
        };
        let span = self.token().span.to(self.t[self.pos + n - 1].span);
        self.pos += n;
        if !in_struct {
            return self.error(span, t!("self_outside_struct"));
        }
        Ok(Some(r))
    }

    /// `var` or `move` before a parameter's type or an argument; `Read` if neither.
    fn mode(&mut self) -> Mode {
        let mode = match self.kind() {
            Kind::Keyword("var") => Mode::Var,
            Kind::Keyword("move") => Mode::Move,
            _ => return Mode::Read,
        };
        self.advance();
        mode
    }

    /// After the parameters: `-> Type`, the contracts, the body.
    fn function_rest(&mut self, name: Name, generics: Vec<Generic>, receiver: Option<Mode>, params: Vec<Param>) -> R<Function> {
        let ret = if self.is_sym("->") {
            self.advance();
            Some(self.type_()?)
        } else {
            None
        };
        let mut contracts = vec![];
        loop {
            self.skip_newlines();
            let kind = match self.kind() {
                Kind::Keyword("require") => ContractKind::Require,
                Kind::Keyword("ensure") => ContractKind::Ensure,
                _ => break,
            };
            contracts.push(self.contract(kind)?);
        }
        // In an interface, a method without a body: the types following it write theirs.
        if self.in_interface && matches!(self.kind(), Kind::Keyword("fn") | Kind::Sym("}")) {
            self.bodiless = true;
            let at = self.token().span;
            let body = Block { open: at, stmts: vec![], close: at };
            return Ok(Function { name, generics, receiver, params, ret, contracts, body });
        }
        let body = self.block()?;
        Ok(Function { name, generics, receiver, params, ret, contracts, body })
    }

    /// `require cond`, `ensure cond` or `check cond`, with an optional `else "message"`.
    fn contract(&mut self, kind: ContractKind) -> R<Contract> {
        self.advance(); // require, ensure or check
        // `require` and `ensure` come right before the body's `{`.
        let cond = if kind == ContractKind::Check { self.expression()? } else { self.heading_expr()? };
        // `else` may be on the next line.
        let before = self.pos;
        self.skip_newlines();
        if !self.is_keyword("else") {
            self.pos = before;
            return Ok(Contract { kind, cond, message: None });
        }
        self.advance();
        let t = self.token();
        if t.kind != Kind::Str {
            let msg = t!("contract_message_expected", found = self.describe());
            self.diags.push(Diag::new(t.span, msg).help(t!("contract_message_help", keyword = kind.keyword())));
            return Err(());
        }
        self.advance();
        let message = self.text_parts(t);
        Ok(Contract { kind, cond, message: Some(message) })
    }

    // ---- Blocks and statements ----

    fn block(&mut self) -> R<Block> {
        let open = self.expect("{")?;
        let mut stmts = vec![];
        loop {
            self.skip_newlines();
            if self.is_sym("}") {
                break;
            }
            // Forgotten `}` (often: while typing): keep what was read.
            if matches!(self.kind(), Kind::Eof | Kind::Keyword("fn" | "test" | "struct" | "enum")) {
                let span = self.token().span;
                self.diags.push(Diag::new(span, t!("block_unclosed")));
                return Ok(Block { open, stmts, close: span });
            }
            match self.stmt() {
                Ok(s) => {
                    stmts.push(s);
                    if !matches!(self.kind(), Kind::Newline | Kind::Sym("}") | Kind::Eof) {
                        let msg = t!("statement_end_expected", found = self.describe());
                        let _: R<()> = self.error(self.token().span, msg);
                        self.skip_statement();
                    }
                }
                Err(()) => self.skip_statement(),
            }
        }
        let close = self.advance().span;
        Ok(Block { open, stmts, close })
    }

    /// After an error: skips to the end of the statement (line end or the block's `}`).
    /// Only braces count: the lexer emits no `Newline` inside `( )` and `[ ]`.
    fn skip_statement(&mut self) {
        let mut depth = 0;
        loop {
            match self.kind() {
                Kind::Eof => return,
                Kind::Newline | Kind::Sym("}") if depth == 0 => return,
                Kind::Sym("{") => depth += 1,
                Kind::Sym("}") => depth -= 1,
                _ => {}
            }
            self.advance();
        }
    }

    fn stmt(&mut self) -> R<Stmt> {
        let start = self.token().span;
        match self.kind() {
            Kind::Keyword(k @ ("let" | "var")) => {
                self.advance();
                let pattern = self.pattern()?;
                let ty = if self.is_sym(":") {
                    self.advance();
                    Some(self.type_()?)
                } else {
                    None
                };
                self.expect(":=")?;
                Ok(Stmt::Declare { mutable: k == "var", pattern, ty, value: self.expression()? })
            }
            Kind::Keyword("return") => {
                self.advance();
                let value = if matches!(self.kind(), Kind::Newline | Kind::Sym("}") | Kind::Eof) {
                    None
                } else {
                    Some(self.expression()?)
                };
                Ok(Stmt::Return { value, span: start })
            }
            Kind::Keyword("break") => Ok(Stmt::Break(self.advance().span)),
            Kind::Keyword("continue") => Ok(Stmt::Continue(self.advance().span)),
            Kind::Keyword("if") => self.if_stmt(),
            Kind::Keyword("match") => Ok(Stmt::Match(self.match_()?)),
            Kind::Keyword("check") => Ok(Stmt::Check(self.contract(ContractKind::Check)?)),
            Kind::Keyword(k @ ("require" | "ensure")) => {
                self.diags.push(Diag::new(start, t!("contract_place", keyword = k)).help(t!("contract_place_help")));
                Err(())
            }
            Kind::Keyword("while") => {
                self.advance();
                let cond = self.heading_expr()?;
                self.skip_newlines();
                Ok(Stmt::While { cond, body: self.block()? })
            }
            Kind::Keyword("for") => {
                self.advance();
                let counter = self.value_name("what_variable")?;
                if !self.is_keyword("in") {
                    let msg = t!("expected_symbol", symbol = "in", found = self.describe());
                    return self.error(self.token().span, msg);
                }
                self.advance();
                let e = self.heading_expr()?;
                self.skip_newlines();
                let body = self.block()?;
                match e.kind {
                    ExprKind::Binary(op @ (BinOp::Range | BinOp::RangeInclusive), start, end) => {
                        Ok(Stmt::Loop { counter, start: *start, end: *end, inclusive: op == BinOp::RangeInclusive, body })
                    }
                    _ => Ok(Stmt::Each { name: counter, list: e, body }),
                }
            }
            Kind::Keyword("from") => {
                self.advance();
                let counter = self.value_name("what_variable")?;
                self.expect(":=")?;
                let start = self.heading_expr()?;
                if !self.is_keyword("to") {
                    let msg = t!("expected_symbol", symbol = "to", found = self.describe());
                    return self.error(self.token().span, msg);
                }
                self.advance();
                let end = self.heading_expr()?;
                self.skip_newlines();
                Ok(Stmt::Loop { counter, start, end, inclusive: true, body: self.block()? })
            }
            _ => {
                let e = self.expression()?;
                let Some(&(_, op)) = ASSIGNMENTS.iter().find(|(s, _)| self.is_sym(s)) else {
                    return Ok(Stmt::Expr(e));
                };
                let target = self.target(e)?;
                self.advance();
                Ok(Stmt::Assign { target, op, value: self.expression()? })
            }
        }
    }

    fn if_stmt(&mut self) -> R<Stmt> {
        self.advance(); // if
        let cond = self.heading_expr()?;
        self.skip_newlines();
        let then = self.block()?;
        // `else` may be on the next line: look past the line ends.
        let before = self.pos;
        self.skip_newlines();
        if !self.is_keyword("else") {
            self.pos = before;
            return Ok(Stmt::If { cond, then, otherwise: None });
        }
        self.advance();
        self.skip_newlines();
        let otherwise = if self.is_keyword("if") {
            // `else if` = an `else` block containing just an `if`.
            let open = self.token().span;
            let inner = self.if_stmt()?;
            Block { open, stmts: vec![inner], close: self.t[self.pos - 1].span }
        } else {
            self.block()?
        };
        Ok(Stmt::If { cond, then, otherwise: Some(otherwise) })
    }

    /// `if` in an expression: `else` is required (there must be a value in every case).
    fn if_expr(&mut self) -> R<Expr> {
        let start = self.advance().span; // if
        let cond = self.heading_expr()?;
        self.skip_newlines();
        let then = self.block()?;
        self.skip_newlines();
        if !self.is_keyword("else") {
            self.diags.push(
                Diag::new(start, t!("if_needs_else")).help(t!("if_needs_else_help")),
            );
            return Err(());
        }
        self.advance();
        self.skip_newlines();
        let otherwise = if self.is_keyword("if") {
            let open = self.token().span;
            let e = self.if_expr()?;
            Block { open, close: Span::new(e.span.end, e.span.end), stmts: vec![Stmt::Expr(e)] }
        } else {
            self.block()?
        };
        let span = start.to(otherwise.close);
        Ok(self.expr(span, ExprKind::If { cond: Box::new(cond), then, otherwise }))
    }

    /// `match x { Circle(r) => …, _ => … }`: arms separated by `,` or a line break.
    fn match_(&mut self) -> R<Match> {
        self.advance(); // match
        let subject = self.heading_expr()?;
        self.skip_newlines();
        self.expect("{")?;
        let saved = std::mem::replace(&mut self.no_literal, false);
        let mut arms = vec![];
        let close = loop {
            self.skip_newlines();
            if self.is_sym("}") {
                break Ok(self.advance().span);
            }
            if self.kind() == Kind::Eof {
                let span = self.token().span;
                self.diags.push(Diag::new(span, t!("block_unclosed")));
                break Err(());
            }
            match self.arm() {
                Ok(a) => arms.push(a),
                Err(()) => self.skip_statement(),
            }
        };
        self.no_literal = saved;
        Ok(Match { subject, arms, close: close? })
    }

    /// `Circle(r) => 3.14 * r * r` or `Red => { … }`.
    fn arm(&mut self) -> R<Arm> {
        let case = if self.kind() == Kind::Ident && self.text(self.token().span) == "_" {
            Case::Any(self.advance().span)
        } else if self.is_keyword("none") {
            let span = self.advance().span;
            Case::Variant { name: Name { text: "none".into(), span }, binds: vec![] }
        } else if matches!(self.kind(), Kind::Ident) || self.is_sym("(") {
            Case::Bind(self.pattern()?)
        } else {
            let name = match self.kind() {
                Kind::Keyword(word @ ("ok" | "err")) => Name { text: word.into(), span: self.advance().span },
                _ => self.plain_type_name()?,
            };
            let mut binds = vec![];
            if self.is_sym("(") {
                let open = self.advance().span;
                while !self.is_sym(")") {
                    binds.push(self.pattern()?);
                    if !self.is_sym(")") {
                        self.expect(",")?;
                    }
                }
                let close = self.advance().span;
                if binds.is_empty() {
                    self.diags.push(Diag::new(open.to(close), t!("empty_parens_variant", name = name.text)));
                }
            }
            Case::Variant { name, binds }
        };
        let arrow = self.expect("=>")?;
        self.skip_newlines();
        let body = if self.is_sym("{") {
            self.block()?
        } else {
            // From the `=>` to what follows the statement (`,`, line end, `}`): the editor's view of the arm.
            // An unfinished statement (while typing) keeps the arm, so that its names are offered.
            let s = self.stmt();
            if s.is_err() {
                self.skip_statement();
            }
            Block { open: arrow, stmts: s.into_iter().collect(), close: self.token().span }
        };
        self.separator()?;
        Ok(Arm { case, body })
    }

    // ---- Expressions ----

    fn expression(&mut self) -> R<Expr> {
        self.level(0)
    }

    fn level(&mut self, n: usize) -> R<Expr> {
        let Some(&(ops, chainable)) = LEVELS.get(n) else {
            return self.unary();
        };
        let mut left = self.level(n + 1)?;
        let mut seen = false;
        while let Some(&(s, op)) = ops.iter().find(|(s, _)| self.is_sym(s)) {
            if seen && !chainable {
                let span = self.token().span;
                self.diags.push(
                    Diag::new(span, t!("not_chainable", op = s)).help(t!("not_chainable_help")),
                );
                return Err(());
            }
            self.advance();
            let right = self.level(n + 1)?;
            let span = left.span.to(right.span);
            left = self.expr(span, ExprKind::Binary(op, Box::new(left), Box::new(right)));
            seen = true;
        }
        Ok(left)
    }

    fn unary(&mut self) -> R<Expr> {
        let op = match self.kind() {
            Kind::Sym("-") => UnOp::Neg,
            Kind::Sym("!") => UnOp::Not,
            _ => return self.primary(),
        };
        let start = self.advance().span;
        let e = self.unary()?;
        Ok(self.expr(start.to(e.span), ExprKind::Unary(op, Box::new(e))))
    }

    /// `f()`: empty parentheses are not written at a call.
    fn empty_parens(&mut self, args: &[Expr], open: Span, close: Span, name: &str) {
        if args.is_empty() {
            self.diags.push(Diag::new(open.to(close), t!("empty_parens", name = name)));
        }
    }

    /// An atom followed by its suffixes: `x.to_float`.
    fn primary(&mut self) -> R<Expr> {
        let mut e = self.atom()?;
        loop {
            // `scores[i]`: the `[` on the same line (otherwise it starts a list on the next one).
            if self.is_sym("[") && !self.src[e.span.end..self.token().span.start].contains('\n') {
                self.advance();
                let index = self.inner_expr()?;
                let end = self.expect("]")?;
                e = self.expr(e.span.to(end), ExprKind::Index(Box::new(e), Box::new(index)));
                continue;
            }
            // `x?`: on the same line (a `?` never starts a line anyway).
            if self.is_sym("?") {
                let end = self.advance().span;
                e = self.expr(e.span.to(end), ExprKind::Try(Box::new(e)));
                continue;
            }
            if !self.is_sym(".") {
                break;
            }
            self.advance();
            if let Kind::Int(i) = self.kind() {
                let end = self.advance().span;
                e = self.expr(e.span.to(end), ExprKind::Field(Box::new(e), i as usize));
                continue;
            }
            let name = self.value_name("what_method")?;
            let (mut args, mut end) = (vec![], name.span);
            if self.is_sym("(") {
                let open = self.advance().span;
                args = self.arguments()?;
                end = self.advance().span;
                self.empty_parens(&args, open, end, &name.text);
            }
            e = self.expr(e.span.to(end), ExprKind::Method { receiver: Box::new(e), name, args });
        }
        Ok(e)
    }

    /// The arguments of a call, after the `(`; stops at the `)` without consuming it.
    fn arguments(&mut self) -> R<Vec<Expr>> {
        let mut args = vec![];
        while !self.is_sym(")") {
            let start = self.token().span;
            let arg = match self.mode() {
                Mode::Read => self.inner_expr()?,
                mode => {
                    let value = self.inner_expr()?;
                    self.expr(start.to(value.span), ExprKind::Mark(mode, Box::new(value)))
                }
            };
            args.push(arg);
            if !self.is_sym(")") {
                self.expect(",")?;
            }
        }
        Ok(args)
    }

    /// `mode = ` or `mode != `: the test of the mode (`kova run` or `kova build`).
    /// ponytail: contextual, so `mode` alone is still an ordinary name.
    fn mode_test(&self) -> bool {
        matches!(self.t[self.pos + 1].kind, Kind::Sym("=" | "!="))
    }

    /// `mode = build`, `mode != run`.
    fn in_mode(&mut self) -> R<Expr> {
        let start = self.advance().span; // mode
        let negated = self.advance().kind == Kind::Sym("!=");
        let t = self.token();
        let build = match (t.kind, self.text(t.span).as_str()) {
            (Kind::Ident, "build") => true,
            (Kind::Ident, "run") => false,
            _ => {
                let msg = t!("setting_bad_value", key = "mode", allowed = format!("`run` {} `build`", t!("word_or")), value = self.text(t.span));
                return self.error(t.span, msg);
            }
        };
        self.advance();
        Ok(self.expr(start.to(t.span), ExprKind::InMode { build, negated }))
    }

    /// After a function's name: its arguments, if any (`seven` alone calls `fn seven`).
    fn call_rest(&mut self, name: Name) -> R<Expr> {
        if !self.is_sym("(") {
            return Ok(self.expr(name.span, ExprKind::Call { name, args: vec![] }));
        }
        let open = self.advance().span;
        let args = self.arguments()?;
        let end = self.advance().span;
        self.empty_parens(&args, open, end, &name.text);
        Ok(self.expr(name.span.to(end), ExprKind::Call { name, args }))
    }

    fn atom(&mut self) -> R<Expr> {
        let t = self.token();
        let kind = match t.kind {
            Kind::Int(v) => ExprKind::Int(v),
            Kind::Float(v) => ExprKind::Float(v),
            Kind::Char(c) => ExprKind::Char(c),
            Kind::Keyword("true") => ExprKind::Bool(true),
            Kind::Keyword("none") => ExprKind::None,
            Kind::Keyword("false") => ExprKind::Bool(false),
            Kind::Keyword("result") => ExprKind::Result,
            Kind::Keyword("self") => ExprKind::Var("self".into()),
            Kind::Str => {
                let mut parts = self.text_parts(t);
                match parts.as_mut_slice() {
                    [Part::Text(s)] => ExprKind::Str(std::mem::take(s)),
                    _ => ExprKind::Interp(parts),
                }
            }
            Kind::Ident if self.text(t.span) == "mode" && self.mode_test() => return self.in_mode(),
            // `geo.Point { … }`, `geo.Point.origin`: a type of an imported file.
            Kind::Ident if self.import_prefix().is_some() && self.t[self.pos + 2].kind == Kind::Type => return self.type_atom(),
            // `geo.distance(a, b)`, `geo.origin`: a function of an imported file.
            Kind::Ident if self.import_prefix().is_some() => {
                let module = self.import_prefix().unwrap();
                self.advance();
                self.advance(); // .
                let mut name = self.value_name("what_function")?;
                name = Name { text: format!("{module}.{}", name.text), span: t.span.to(name.span) };
                return self.call_rest(name);
            }
            Kind::Ident if self.imports.contains_key(&self.text(t.span)) => {
                let name = self.text(t.span);
                self.diags.push(Diag::new(t.span, t!("module_as_value", name = name)).help(t!("module_as_value_help", name = name)));
                return Err(());
            }
            Kind::Ident => {
                let mut name = self.value_name("what_variable")?;
                if !self.is_sym("(") && !self.no_params.contains(&name.text) {
                    return Ok(self.expr(name.span, ExprKind::Var(name.text.clone())));
                }
                name.text = self.qualified(name.text);
                return self.call_rest(name);
            }
            Kind::Keyword("if") => return self.if_expr(),
            Kind::Keyword(word @ ("ok" | "err")) => {
                self.advance();
                self.expect("(")?;
                let value = self.inner_expr()?;
                let end = self.expect(")")?;
                return Ok(self.expr(t.span.to(end), ExprKind::Outcome { ok: word == "ok", value: Box::new(value) }));
            }
            Kind::Keyword("match") => {
                let m = self.match_()?;
                return Ok(self.expr(t.span.to(m.close), ExprKind::Match(Box::new(m))));
            }
            Kind::Type => return self.type_atom(),
            Kind::Sym("(") => {
                self.advance();
                let e = self.inner_expr()?;
                if !self.is_sym(",") {
                    let end = self.expect(")")?;
                    return Ok(Expr { span: t.span.to(end), ..e });
                }
                // `(a, b)`: a tuple.
                let mut elems = vec![e];
                while self.is_sym(",") {
                    self.advance();
                    elems.push(self.inner_expr()?);
                }
                let end = self.expect(")")?;
                return Ok(self.expr(t.span.to(end), ExprKind::Tuple(elems)));
            }
            Kind::Sym("[") => {
                self.advance();
                let mut elems = vec![];
                while !self.is_sym("]") {
                    elems.push(self.inner_expr()?);
                    if !self.is_sym("]") {
                        self.expect(",")?;
                    }
                }
                let end = self.advance().span;
                return Ok(self.expr(t.span.to(end), ExprKind::List(elems)));
            }
            _ => {
                let msg = t!("expression_expected", found = self.describe());
                return self.error(t.span, msg);
            }
        };
        self.advance();
        Ok(self.expr(t.span, kind))
    }

    /// An expression starting with a type name: `Dog { name := "Rex" }`.
    fn type_atom(&mut self) -> R<Expr> {
        let ty = self.type_name()?;
        // `Stack<Int>`: the type arguments, before `{`, `.` or the `()` of `List<Int>()`.
        let mut types = vec![];
        let mut last = ty.span;
        if self.is_sym("<") {
            let TypeAst::Generic(_, ts) = self.type_arguments(ty.clone())? else { unreachable!() };
            types = ts;
            last = self.t[self.pos - 1].span;
        }
        // `List<Int>()`: a generic constructor keeps its `()` (PLAN Part 4).
        if !types.is_empty() && self.is_sym("(") {
            self.advance();
            let end = self.expect(")")?;
            return Ok(self.expr(ty.span.to(end), ExprKind::New(TypeAst::Generic(ty, types))));
        }
        // The `{` of a literal is on the same line as the type name (PLAN 2.3).
        let brace = self.is_sym("{") && !self.src[last.end..self.token().span.start].contains('\n');
        if brace && self.no_literal {
            self.diags.push(
                Diag::new(ty.span, t!("literal_in_heading", name = ty.text)).help(t!("literal_in_heading_help")),
            );
            return Err(());
        }
        // `Dog.create("Rex")`, `Dog.default`; an enum's variant: `Shape.Circle(2.0)`, `Light.Red`.
        if self.is_sym(".") && matches!(self.t[self.pos + 1].kind, Kind::Ident | Kind::Type) {
            self.advance();
            let variant = self.kind() == Kind::Type;
            let name = if variant { self.plain_type_name()? } else { self.value_name("what_function")? };
            let (mut args, mut end) = (vec![], name.span);
            if self.is_sym("(") {
                let open = self.advance().span;
                args = self.arguments()?;
                end = self.advance().span;
                self.empty_parens(&args, open, end, &name.text);
            }
            if variant && !types.is_empty() {
                self.diags.push(Diag::new(ty.span, t!("not_generic", name = ty.text)).help(t!("not_generic_help", name = ty.text)));
            }
            let span = ty.span.to(end);
            let kind = if variant { ExprKind::Variant { ty, name, args } } else { ExprKind::Static { ty, types, name, args } };
            return Ok(self.expr(span, kind));
        }
        if !brace {
            self.diags.push(Diag::new(ty.span, t!("type_not_value", name = ty.text)).help(t!("type_not_value_help", name = ty.text)));
            return Err(());
        }
        self.advance(); // {
        let saved = std::mem::replace(&mut self.no_literal, false);
        let fields = self.literal_fields();
        self.no_literal = saved;
        let (fields, end) = fields?;
        Ok(self.expr(ty.span.to(end), ExprKind::StructLit { ty, types, fields }))
    }

    /// `name := "Rex", age := 3 }`: separated by `,` or a line break. Returns the `}` too.
    fn literal_fields(&mut self) -> R<(Vec<(Name, Expr)>, Span)> {
        let mut fields = vec![];
        loop {
            self.skip_newlines();
            if self.is_sym("}") {
                return Ok((fields, self.advance().span));
            }
            let name = self.value_name("what_field")?;
            self.expect(":=")?;
            fields.push((name, self.expression()?));
            if self.is_sym(",") {
                self.advance();
            } else if !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
                let msg = t!("expected_symbol", symbol = ",", found = self.describe());
                return self.error(self.token().span, msg);
            }
        }
    }
}
