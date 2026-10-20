// Parser: turns the token list into a tree (AST).
//
// It doesn't stop at the first error: it records it, skips to the end of the
// statement, then continues. So all errors are seen at once.

use std::collections::HashSet;

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
pub fn parse(tokens: Vec<Token>, source: &str) -> (File, usize, Vec<Diag>) {
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
        .filter(|(w, _)| w[2].kind != Kind::Sym("(") || w[3].kind == Kind::Sym(")"))
        .map(|(w, _)| source[w[1].span.start..w[1].span.end].to_string())
        .collect();
    let mut p = Parser { t: tokens, pos: 0, src: source, diags: vec![], ids: 0, no_params, no_literal: false };
    let file = p.file();
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

    /// A variable or function name: lowercase required. `what` = the key of its kind (`what_variable`).
    fn value_name(&mut self, what: &str) -> R<Name> {
        let what = crate::locale::text(what, &[]);
        let t = self.token();
        match t.kind {
            Kind::Ident => {
                self.advance();
                Ok(Name { text: self.text(t.span), span: t.span })
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

    /// A type name: uppercase required.
    fn type_name(&mut self) -> R<Name> {
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

    /// `Int` or `(Int, Float)`.
    fn type_(&mut self) -> R<TypeAst> {
        if !self.is_sym("(") {
            return Ok(TypeAst::Name(self.type_name()?));
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

    /// `x`, `t.0`, `rex.age` or `(a, b)` left of `:=`: re-reads the expression already parsed.
    fn target(&mut self, e: Expr) -> R<Target> {
        let span = e.span;
        let (base, step) = match e.kind {
            ExprKind::Var(name) => return Ok(Target::Place { name: Name { text: name, span }, fields: vec![], span }),
            ExprKind::Field(base, i) => (base, Step::Index(i)),
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
                Kind::Keyword("fn") => self.function(false).map(|f| functions.push(f)),
                Kind::Keyword("struct" | "copy") => self.struct_().map(|s| structs.push(s)),
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
        File { header, functions, structs, tests, langs }
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
        while !matches!(self.kind(), Kind::Keyword("fn" | "test" | "struct" | "copy") | Kind::Eof) && !self.at_lang() {
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
        self.advance(); // struct
        let name = self.type_name()?;
        self.skip_newlines();
        self.expect("{")?;
        let (mut fields, mut methods) = (vec![], vec![]);
        loop {
            self.skip_newlines();
            match self.kind() {
                Kind::Sym("}") => break,
                Kind::Eof | Kind::Keyword("test" | "struct" | "copy") => {
                    let span = self.token().span;
                    self.diags.push(Diag::new(span, t!("struct_unclosed")));
                    return Ok(Struct { copy, name, fields, methods });
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
                _ => match self.field() {
                    Ok(f) => fields.push(f),
                    Err(()) => self.skip_statement(),
                },
            }
        }
        self.advance(); // }
        Ok(Struct { copy, name, fields, methods })
    }

    /// `name: String`, followed by `,`, a line break or the `}`.
    fn field(&mut self) -> R<Param> {
        let name = self.value_name("what_field")?;
        self.expect(":")?;
        let ty = self.type_()?;
        if self.is_sym(",") {
            self.advance();
        } else if !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
            let msg = t!("expected_symbol", symbol = ",", found = self.describe());
            return self.error(self.token().span, msg);
        }
        Ok(Param { name, ty })
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
        let name = self.value_name("what_function")?;
        let mut params = vec![];
        if !self.is_sym("(") {
            return self.function_rest(name, None, params);
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
            params.push(Param { name, ty: self.type_()? });
            if !self.is_sym(")") {
                self.expect(",")?;
            }
        }
        self.advance(); // )
        self.function_rest(name, receiver, params)
    }

    /// `self`, `var self` or `move self` at the start of the parameters.
    fn receiver(&mut self, in_struct: bool) -> R<Option<Receiver>> {
        let next_is_self = matches!(self.t[self.pos + 1].kind, Kind::Keyword("self"));
        let (r, n) = match self.kind() {
            Kind::Keyword("self") => (Receiver::Read, 1),
            Kind::Keyword("var") if next_is_self => (Receiver::Var, 2),
            Kind::Keyword("move") if next_is_self => (Receiver::Move, 2),
            _ => return Ok(None),
        };
        let span = self.token().span.to(self.t[self.pos + n - 1].span);
        self.pos += n;
        if !in_struct {
            return self.error(span, t!("self_outside_struct"));
        }
        Ok(Some(r))
    }

    /// After the parameters: `-> Type`, the contracts, the body.
    fn function_rest(&mut self, name: Name, receiver: Option<Receiver>, params: Vec<Param>) -> R<Function> {
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
        let body = self.block()?;
        Ok(Function { name, receiver, params, ret, contracts, body })
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
            if matches!(self.kind(), Kind::Eof | Kind::Keyword("fn" | "test" | "struct")) {
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
                let ExprKind::Binary(op @ (BinOp::Range | BinOp::RangeInclusive), start, end) = e.kind else {
                    self.diags.push(
                        Diag::new(e.span, t!("for_range_only")).help(t!("for_range_help")),
                    );
                    return Err(());
                };
                self.skip_newlines();
                let body = self.block()?;
                Ok(Stmt::Loop { counter, start: *start, end: *end, inclusive: op == BinOp::RangeInclusive, body })
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
        while self.is_sym(".") {
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
            args.push(self.inner_expr()?);
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

    fn atom(&mut self) -> R<Expr> {
        let t = self.token();
        let kind = match t.kind {
            Kind::Int(v) => ExprKind::Int(v),
            Kind::Float(v) => ExprKind::Float(v),
            Kind::Char(c) => ExprKind::Char(c),
            Kind::Keyword("true") => ExprKind::Bool(true),
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
            Kind::Ident => {
                let name = self.value_name("what_variable")?;
                if !self.is_sym("(") {
                    let kind = if self.no_params.contains(&name.text) {
                        ExprKind::Call { name: name.clone(), args: vec![] }
                    } else {
                        ExprKind::Var(name.text.clone())
                    };
                    return Ok(self.expr(name.span, kind));
                }
                let open = self.advance().span;
                let args = self.arguments()?;
                let end = self.advance().span;
                self.empty_parens(&args, open, end, &name.text);
                return Ok(self.expr(name.span.to(end), ExprKind::Call { name, args }));
            }
            Kind::Keyword("if") => return self.if_expr(),
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
        // The `{` of a literal is on the same line as the type name (PLAN 2.3).
        let brace = self.is_sym("{") && !self.src[ty.span.end..self.token().span.start].contains('\n');
        if brace && self.no_literal {
            self.diags.push(
                Diag::new(ty.span, t!("literal_in_heading", name = ty.text)).help(t!("literal_in_heading_help")),
            );
            return Err(());
        }
        // `Dog.create("Rex")`, `Dog.default`.
        if self.is_sym(".") && self.t[self.pos + 1].kind == Kind::Ident {
            self.advance();
            let name = self.value_name("what_function")?;
            let (mut args, mut end) = (vec![], name.span);
            if self.is_sym("(") {
                let open = self.advance().span;
                args = self.arguments()?;
                end = self.advance().span;
                self.empty_parens(&args, open, end, &name.text);
            }
            return Ok(self.expr(ty.span.to(end), ExprKind::Static { ty, name, args }));
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
        Ok(self.expr(ty.span.to(end), ExprKind::StructLit { ty, fields }))
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
