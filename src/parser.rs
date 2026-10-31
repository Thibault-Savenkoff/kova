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
    (&[("|", BinOp::BitOr)], true),
    (&[("^", BinOp::BitXor)], true),
    (&[("&", BinOp::BitAnd)], true),
    (&[("<<", BinOp::ShiftLeft), (">>", BinOp::ShiftRight)], true),
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
        constants: HashSet::new(),
        no_literal: false,
        no_else: false,
        in_interface: false,
        bodiless: false,
        in_extern: false,
        symbol: None,
        copies: vec![],
        ui_runs: vec![],
        aliases: HashMap::new(),
        public: false,
        private: vec![],
        module: None,
        own: HashSet::new(),
        imports: HashMap::new(),
        package: None,
    };
    let mut interfaces = p.file().interfaces;
    debug_assert!(p.diags.is_empty() && p.ids == 0, "the prelude has no expressions");
    interfaces.iter_mut().for_each(|i| i.builtin = true);
    interfaces
}

pub fn parse(tokens: Vec<Token>, source: &str) -> (File, usize, Vec<Diag>) {
    parse_in(tokens, source, 0, None, None)
}

/// Reads one file of a program. `first_id`: the number of its first expression (the numbers
/// go on from file to file). `module`: the file's name when it is imported (`geometry`); its
/// own names are then written with it (`geometry.Point`), so that two files never share one.
/// `package`: the package the file belongs to (`kova_json`), whose files its own imports name.
pub fn parse_in(tokens: Vec<Token>, source: &str, first_id: usize, module: Option<&str>, package: Option<&str>) -> (File, usize, Vec<Diag>) {
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
        .chain(["read_line".to_string()]);
    // A constant is a function without parameters (`Parser::constant`).
    let constants: HashSet<String> = tokens
        .windows(2)
        .enumerate()
        .filter(|(i, w)| (*i == 0 || outside[*i - 1]) && w[0].kind == Kind::Keyword("let") && matches!(w[1].kind, Kind::Ident | Kind::Type))
        .map(|(_, w)| source[w[1].span.start..w[1].span.end].to_string())
        .collect();
    let no_params = no_params.chain(constants.iter().cloned()).collect();
    let text = |t: &Token| source[t.span.start..t.span.end].to_string();
    let at_top = |i: usize| i == 0 || outside[i - 1];
    // `import geometry as geo`: `geo.Point` means `geometry.Point` in this file.
    // `import shapes.circle`: `circle.Circle` means `shapes.circle.Circle`.
    let mut imports = HashMap::new();
    let kind = |i: usize| tokens.get(i).map(|t| t.kind);
    for i in 0..tokens.len() {
        if !(at_top(i) && tokens[i].kind == Kind::Keyword("import") && kind(i + 1) == Some(Kind::Ident)) {
            continue;
        }
        let (mut module, mut last) = (text(&tokens[i + 1]), i + 1);
        while kind(last + 1) == Some(Kind::Sym(".")) && kind(last + 2) == Some(Kind::Ident) {
            last += 2;
            module = format!("{module}.{}", text(&tokens[last]));
        }
        // `import json from "github.com/ada/kova-json" version "…"`: the package's own name.
        if kind(last + 1) == Some(Kind::Keyword("from")) && kind(last + 2) == Some(Kind::Str) {
            imports.insert(module, crate::ast::package_name(&crate::lexer::string_content(&text(&tokens[last + 2]))));
            continue;
        }
        // Inside a package, its other files: `import util` is `kova_json.util`.
        let module = package.map_or(module.clone(), |p| format!("{p}.{module}"));
        let renamed = kind(last + 1) == Some(Kind::Ident) && text(&tokens[last + 1]) == "as" && kind(last + 2) == Some(Kind::Ident);
        imports.insert(text(&tokens[if renamed { last + 2 } else { last }]), module);
    }
    // `extern sqlite from "sqlite3"`: `sqlite.open` is a function of this block.
    let word = |i: usize, w: &str| kind(i) == Some(Kind::Ident) && text(&tokens[i]) == w;
    for i in 0..tokens.len() {
        if at_top(i) && word(i, "extern") && kind(i + 1) == Some(Kind::Ident) && kind(i + 2) == Some(Kind::Keyword("from")) {
            let name = text(&tokens[i + 1]);
            imports.insert(name.clone(), module.map_or(name.clone(), |m| format!("{m}.{name}")));
        }
    }
    // The names this file declares at its top: `fn distance`, `struct Point`, `copy struct`, `enum`, `interface`.
    let own = (0..tokens.len().saturating_sub(1))
        .filter(|&i| at_top(i))
        .filter(|&i| match (tokens[i].kind, tokens[i + 1].kind) {
            (Kind::Keyword("fn"), Kind::Ident) | (Kind::Keyword("let"), Kind::Ident | Kind::Type) => true,
            (Kind::Keyword("struct" | "enum" | "interface"), Kind::Type) => true,
            _ => false,
        })
        .map(|i| text(&tokens[i + 1]))
        .collect();
    let module = module.map(str::to_string);
    let package = package.map(str::to_string);
    let mut p = Parser {
        t: tokens,
        pos: 0,
        src: source,
        diags: vec![],
        ids: first_id,
        no_params,
        constants,
        no_literal: false,
        no_else: false,
        in_interface: false,
        bodiless: false,
        in_extern: false,
        symbol: None,
        copies: vec![],
        ui_runs: vec![],
        aliases: HashMap::new(),
        public: false,
        private: vec![],
        own: if module.is_some() { own } else { HashSet::new() },
        module,
        package,
        imports,
    };
    let mut file = p.file();
    if p.module.is_none() {
        file.interfaces.splice(0..0, prelude());
    }
    (file, p.ids, p.diags)
}

struct Parser<'a> {
    /// `type Grid := List<List<Int>>`: each use is read as the type itself.
    // ponytail: known in its own file only; a `public` alias when a module needs to share one.
    aliases: HashMap<String, TypeAst>,
    t: Vec<Token>,
    pos: usize,
    src: &'a str,
    diags: Vec<Diag>,
    ids: usize,
    no_params: HashSet<String>,
    /// See `Parser::constant`.
    constants: HashSet<String>,
    /// In a contract's condition: its `else` starts the message, it gives no value.
    no_else: bool,
    /// In a heading (`if`, `while`…): `Dog {` is not a struct literal, the `{` opens the body (PLAN 2.3).
    no_literal: bool,
    /// The name of this file when it is imported (`geometry`), and the names it declares:
    /// inside it, `Point` is `geometry.Point`.
    module: Option<String>,
    own: HashSet<String>,
    /// The package this file belongs to: see `parse_in`.
    package: Option<String>,
    /// `import geometry as geo`: `geo` → `geometry`.
    imports: HashMap<String, String>,
    /// In an `interface`: a method may have no body (`function_rest` then sets `bodiless`).
    in_interface: bool,
    bodiless: bool,
    /// Reading an `extern` block: functions without a body, maybe `from "c_name"`.
    in_extern: bool,
    /// That `from "c_name"`, read by `extern_`.
    symbol: Option<CName>,
    /// Its parameters (and result, after them) given by value: `copy Rect`.
    copies: Vec<usize>,
    /// See `File::ui_runs`.
    ui_runs: Vec<UiRun>,
    /// Under `public:` at file level (otherwise private, the default); see `File::private`.
    public: bool,
    private: Vec<(String, Span)>,
}

impl Parser<'_> {
    // ---- Tools ----

    fn token(&self) -> Token {
        self.t[self.pos]
    }
    fn kind(&self) -> Kind {
        self.token().kind
    }
    /// The kind of the token `n` places further.
    fn kind_at(&self, n: usize) -> Kind {
        self.t[(self.pos + n).min(self.t.len() - 1)].kind
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
        let d = Diag::new(span, msg);
        let habit = if span == self.token().span { self.habit() } else { None };
        self.diags.push(match habit {
            Some(h) => d.help(h),
            None => d,
        });
        Err(())
    }
    /// What to write instead, when the error comes from a habit of another language.
    fn habit(&self) -> Option<String> {
        let word = |back: usize| self.pos.checked_sub(back).map_or(String::new(), |p| self.text(self.t[p].span));
        // Two signs with no space between them, like `==`.
        let joined = self.pos > 0 && self.t[self.pos - 1].span.end == self.token().span.start;
        let (now, before) = (word(0), word(1));
        Some(match (before.as_str(), now.as_str()) {
            ("=", "=") if joined => t!("habit_equal"),
            ("+", "+") | ("-", "-") if joined => t!("habit_increment", sign = before),
            ("*", "*") if joined => t!("habit_power"),
            ("not", _) => t!("habit_not"),
            ("?", _) if word(2) == "?" => t!("habit_default"),
            (_, "=") => t!("habit_assign"),
            (_, "in") => t!("habit_in", name = before),
            (_, "=>") if self.pos > 0 && self.t[self.pos - 1].kind == Kind::Ident => t!("habit_block_type", name = before),
            (_, "and") => t!("habit_and"),
            (_, "or") => t!("habit_or"),
            _ => return None,
        })
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

    /// Outside `public:`, a name is hidden from the other files (only an imported file has any).
    fn hide(&mut self, public: bool, name: &Name) {
        if !public {
            self.private.push((name.text.clone(), name.span));
        }
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

    /// A name after a dot: a reserved word is one too there (`r.header(…)`, `files.copy(…)`),
    /// since nothing else can follow a dot.
    fn member_name(&mut self, key: &str) -> R<Name> {
        match self.kind() {
            Kind::Keyword(k) => {
                let span = self.advance().span;
                Ok(Name { text: k.into(), span })
            }
            _ => self.value_name(key),
        }
    }

    /// A part of a keyword message.
    fn part_name(&mut self) -> R<Name> {
        self.value_name("what_function")
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
        // A block type anywhere else than a parameter: the block would outlive its call. In an
        // `extern` block, a struct's field may be one: a function C calls (its address).
        if self.is_sym("{") && self.in_extern {
            return self.block_type();
        }
        if self.is_sym("{") {
            let span = self.token().span;
            return self.error(span, t!("block_type_place"));
        }
        // `any Animal`: a value of any type following the interface `Animal`.
        if self.is_keyword("any") {
            self.advance();
            return Ok(TypeAst::Any(self.type_name()?));
        }
        if !self.is_sym("(") {
            if let Some(ty) = self.aliases.get(&self.text(self.token().span)).filter(|_| self.kind() == Kind::Type).cloned() {
                self.advance();
                return Ok(ty);
            }
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

    /// `{Int -> Bool}`, `{Int}`, `{-> Bool}`, `{}`: a parameter's block type.
    fn block_type(&mut self) -> R<TypeAst> {
        self.advance(); // {
        let mut params = vec![];
        while !self.is_sym("}") && !self.is_sym("->") {
            // `{ {Int} -> Int }`: a block taking a block, still a parameter's type.
            params.push(if self.is_sym("{") { self.block_type()? } else { self.type_()? });
            if !self.is_sym("}") && !self.is_sym("->") {
                self.expect(",")?;
            }
        }
        let result = if self.is_sym("->") {
            self.advance();
            Some(Box::new(self.type_()?))
        } else {
            None
        };
        self.expect("}")?;
        Ok(TypeAst::Block(params, result))
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

    /// The file's `type` lines, read first: types may be used before they are declared.
    fn read_aliases(&mut self) {
        let (mut depth, reported) = (0, self.diags.len());
        for i in 0..self.t.len().saturating_sub(2) {
            match self.t[i].kind {
                Kind::Sym("{") => depth += 1,
                Kind::Sym("}") => depth -= 1,
                Kind::Ident if depth == 0 && self.text(self.t[i].span) == "type" && self.t[i + 1].kind == Kind::Type && self.t[i + 2].kind == Kind::Sym(":=") => {
                    self.pos = i + 3;
                    if let Ok(ty) = self.type_() {
                        self.aliases.insert(self.text(self.t[i + 1].span), ty);
                    }
                }
                _ => {}
            }
        }
        self.diags.truncate(reported);
        self.pos = 0;
    }

    fn file(&mut self) -> File {
        self.read_aliases();
        let (mut header, mut functions, mut structs, mut tests, mut langs) = (None, vec![], vec![], vec![], vec![]);
        let (mut interfaces, mut imports, mut externs) = (vec![], vec![], vec![]);
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
                Kind::Keyword(word @ ("public" | "private")) if self.t[self.pos + 1].kind == Kind::Sym(":") => {
                    self.public = word == "public";
                    self.advance();
                    self.advance();
                    Ok(())
                }
                Kind::Keyword("fn") => self.function(false).map(|f| {
                    self.hide(self.public, &f.name);
                    functions.push(f)
                }),
                Kind::Keyword("let") => self.constant().map(|f| {
                    self.hide(self.public, &f.name);
                    functions.push(f)
                }),
                Kind::Keyword("struct" | "copy" | "enum") => self.struct_().map(|s| {
                    self.hide(self.public, &s.name);
                    structs.push(s)
                }),
                Kind::Keyword("interface") => self.interface().map(|i| {
                    self.hide(self.public, &i.name);
                    interfaces.push(i)
                }),
                Kind::Keyword("test") => self.test().map(|t| tests.push(t)),
                // `lang` is not a reserved word: it only means something here.
                Kind::Ident if self.text(self.token().span) == "lang" => self.lang().map(|l| langs.push(l)),
                // Read by `read_aliases`, and read again for its errors.
                Kind::Ident if self.text(self.token().span) == "type" && self.kind_at(1) == Kind::Type => {
                    self.advance();
                    self.advance();
                    self.expect(":=").and_then(|_| self.type_()).map(|_| ())
                }
                // Neither is `extern`.
                Kind::Ident if self.at_extern() => self.extern_().map(|e| {
                    structs.extend(e.structs.iter().map(Struct::kova_view));
                    externs.push(e)
                }),
                Kind::Ident if self.text(self.token().span) == "const" => {
                    self.diags.push(Diag::new(self.token().span, t!("const_word")).help(t!("const_word_help")));
                    Err(())
                }
                _ => {
                    let msg = t!("declaration_expected", found = self.describe());
                    self.error(self.token().span, msg)
                }
            };
            if read.is_err() {
                self.skip_to_next_fn();
            }
        }
        let private = std::mem::take(&mut self.private);
        let ui_runs = std::mem::take(&mut self.ui_runs);
        File { header, imports, functions, structs, interfaces, tests, langs, private, modules: vec![], externs, ui_runs, ui_from: None }
    }

    /// `import geometry` or `import geometry as geo`.
    fn import(&mut self) -> R<Import> {
        self.advance(); // import
        let mut module = self.value_name("what_module")?;
        // `import shapes.circle`: the file `shapes/circle.kova`, called `circle` here.
        let mut last = module.clone();
        while self.is_sym(".") {
            self.advance();
            last = self.value_name("what_module")?;
            module = Name { text: format!("{}.{}", module.text, last.text), span: module.span.to(last.span) };
        }
        // `import json from "github.com/ada/kova-json" version "1.2.0"`.
        if self.is_keyword("from") {
            if module.text.contains('.') {
                return self.error(module.span, t!("package_dotted_name"));
            }
            self.advance();
            let address = self.text_value(true)?;
            if !(self.kind() == Kind::Ident && self.text(self.token().span) == "version") {
                let msg = t!("package_version_expected", found = self.describe());
                return self.error(self.token().span, msg);
            }
            self.advance();
            let version = self.text_value(false)?;
            let own = Name { text: crate::ast::package_name(&address.text), span: address.span };
            return Ok(Import { module: own, alias: module, package: Some((address, version)) });
        }
        if let Some(p) = &self.package {
            module.text = format!("{p}.{}", module.text);
        }
        let alias = if self.kind() == Kind::Ident && self.text(self.token().span) == "as" {
            self.advance();
            self.value_name("what_module")?
        } else {
            last
        };
        Ok(Import { module, alias, package: None })
    }

    /// A text written as is (`"…"`, no `{…}` inside): its content, without the quotes.
    /// `address`: the package's address is expected, else its version.
    fn text_value(&mut self, address: bool) -> R<Name> {
        let t = self.token();
        let raw = self.text(t.span);
        if t.kind != Kind::Str || raw.contains('{') {
            let found = self.describe();
            let msg = if address { t!("package_address_expected", found = found) } else { t!("package_version_expected", found = found) };
            return self.error(t.span, msg);
        }
        self.advance();
        Ok(Name { text: crate::lexer::string_content(&raw), span: t.span })
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

    /// `mode := build`: a key, `:=`, a single word, or a text (`name := "Counter"`, kept with its
    /// quotes: the checker says which keys take one).
    fn setting(&mut self) -> R<(Name, Name)> {
        let key = self.value_name("what_setting")?;
        self.expect(":=")?;
        let t = self.token();
        if !matches!(t.kind, Kind::Ident | Kind::Type | Kind::Keyword(_) | Kind::Str) {
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

    /// `let limit: Int := 10` at the top of a file: a function without parameters, so it has what
    /// a function has (`public:`, imports). Made of written values, it is the same at each use.
    fn constant(&mut self) -> R<Function> {
        let open = self.advance().span;
        let t = self.advance();
        if !matches!(t.kind, Kind::Ident | Kind::Type) {
            let msg = t!("name_expected", what = crate::locale::text("what_variable", &[]), found = format!("`{}`", self.text(t.span)));
            return self.error(t.span, msg);
        }
        let name = Name { text: self.qualified(self.text(t.span)), span: t.span };
        self.expect(":")?;
        let ty = self.type_()?;
        self.expect(":=")?;
        let value = self.expression()?;
        if !self.is_constant(&value) {
            self.diags.push(Diag::new(value.span, t!("constant_value")).help(t!("constant_value_help")));
        }
        let span = value.span;
        let body = Block { open, stmts: vec![Stmt::Return { value: Some(value), span }], close: span };
        Ok(Function { name, generics: vec![], receiver: None, params: vec![], breaks: vec![], ret: Some(ty), contracts: vec![], only: None, body })
    }

    /// Whether `e` is made only of written values and other constants.
    fn is_constant(&self, e: &Expr) -> bool {
        let all = |l: &[Expr]| l.iter().all(|x| self.is_constant(x));
        match &e.kind {
            ExprKind::Int(_) | ExprKind::BigInt(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::None => true,
            ExprKind::Unary(_, a) => self.is_constant(a),
            ExprKind::Binary(_, a, b) => self.is_constant(a) && self.is_constant(b),
            ExprKind::Tuple(l) | ExprKind::List(l) | ExprKind::SetLit(l) | ExprKind::Variant { args: l, .. } => all(l),
            ExprKind::MapLit(l) => l.iter().all(|(k, v)| self.is_constant(k) && self.is_constant(v)),
            ExprKind::StructLit { fields, .. } => fields.iter().all(|(_, v)| self.is_constant(v)),
            ExprKind::Call { name, args } => args.is_empty() && self.constants.iter().any(|c| name.text == *c || name.text.ends_with(&format!(".{c}"))),
            _ => false,
        }
    }

    fn skip_to_next_fn(&mut self) {
        self.advance();
        while !matches!(self.kind(), Kind::Keyword("fn" | "let" | "test" | "struct" | "copy" | "enum" | "interface") | Kind::Eof)
            && !self.at_lang()
            && !self.at_extern()
        {
            self.advance();
        }
    }

    fn at_lang(&self) -> bool {
        self.kind() == Kind::Ident && self.text(self.token().span) == "lang"
    }

    fn at_extern(&self) -> bool {
        self.kind() == Kind::Ident && self.text(self.token().span) == "extern" && self.kind_at(1) == Kind::Ident
    }

    /// `extern sqlite from "sqlite3" { fn … }` (PLAN Part 4 "`extern`, step 1").
    fn extern_(&mut self) -> R<Extern> {
        self.advance(); // extern
        let name = self.value_name("what_module")?;
        if !self.is_keyword("from") {
            let msg = t!("extern_from", found = self.describe());
            return self.error(self.token().span, msg);
        }
        self.advance();
        let library = self.c_name()?;
        let module = self.imports.get(&name.text).cloned().unwrap_or(name.text.clone());
        self.skip_newlines();
        self.expect("{")?;
        let (mut functions, mut structs) = (vec![], vec![]);
        loop {
            self.skip_newlines();
            match self.kind() {
                Kind::Sym("}") => break,
                // Its fields are public: C reads them all.
                Kind::Keyword("struct") => {
                    self.in_extern = true;
                    let s = self.struct_();
                    self.in_extern = false;
                    match s {
                        Ok(s) => structs.push(s),
                        Err(()) => self.skip_statement(),
                    }
                }
                Kind::Keyword("fn") => {
                    self.in_extern = true;
                    let f = self.function(true);
                    self.in_extern = false;
                    let (symbol, copies) = (self.symbol.take(), std::mem::take(&mut self.copies));
                    match f {
                        Ok(mut f) => {
                            f.name.text = format!("{module}.{}", f.name.text);
                            self.hide(self.public, &f.name);
                            functions.push((f, symbol, copies));
                        }
                        Err(()) => self.skip_statement(),
                    }
                }
                Kind::Eof | Kind::Keyword("test" | "copy" | "enum" | "interface") => {
                    let span = self.token().span;
                    self.diags.push(Diag::new(span, t!("struct_unclosed", name = name.text)));
                    break;
                }
                _ => {
                    let msg = t!("extern_item", found = self.describe());
                    self.diags.push(Diag::new(self.token().span, msg));
                    self.skip_statement();
                }
            }
        }
        if self.is_sym("}") {
            self.advance();
        }
        // `Point` inside the block is `win.Point`, like the functions.
        let own: Vec<String> = structs.iter().map(|s: &Struct| s.name.text.clone()).collect();
        fn requalify(t: &mut TypeAst, own: &[String], module: &str) {
            match t {
                TypeAst::Name(n) if own.contains(&n.text) => n.text = format!("{module}.{}", n.text),
                TypeAst::Optional(t) => requalify(t, own, module),
                TypeAst::Block(ps, r) => {
                    ps.iter_mut().for_each(|t| requalify(t, own, module));
                    r.iter_mut().for_each(|t| requalify(t, own, module));
                }
                _ => {}
            }
        }
        for s in &mut structs {
            s.name.text = format!("{module}.{}", s.name.text);
            self.hide(self.public, &s.name);
            s.fields.iter_mut().for_each(|p| requalify(&mut p.ty, &own, &module));
        }
        for (f, ..) in &mut functions {
            f.params.iter_mut().for_each(|p| requalify(&mut p.ty, &own, &module));
            f.ret.iter_mut().for_each(|t| requalify(t, &own, &module));
        }
        Ok(Extern { name, library, functions, structs })
    }

    /// After `from`: a name, then the systems that name it otherwise (`windows "winsqlite3"`).
    fn c_name(&mut self) -> R<CName> {
        let text = self.plain_text()?;
        let mut on = vec![];
        while self.kind() == Kind::Ident {
            let t = self.advance();
            let system = Name { text: self.text(t.span), span: t.span };
            if !crate::codegen::SYSTEMS.contains(&system.text.as_str()) {
                self.diags.push(Diag::new(t.span, t!("extern_system", found = system.text)));
            }
            on.push((system, self.plain_text()?));
        }
        Ok(CName { text, on })
    }

    /// A `"…"` without `{code}`: a library's or a C function's name.
    fn plain_text(&mut self) -> R<(String, Span)> {
        let t = self.token();
        if t.kind != Kind::Str {
            let msg = t!("extern_name_text", found = self.describe());
            return self.error(t.span, msg);
        }
        self.advance();
        match self.text_parts(t).as_slice() {
            [Part::Text(s)] if !s.is_empty() => Ok((s.clone(), t.span)),
            _ => {
                let msg = t!("extern_name_text", found = self.text(t.span));
                self.error(t.span, msg)
            }
        }
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
        // `"""…"""`: as written, its errors already reported by the lexer.
        if self.src[t.span.start..t.span.end].starts_with("\"\"\"") {
            return vec![Part::Text(crate::lexer::raw_content(&self.src[t.span.start..t.span.end]).unwrap_or_default())];
        }
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
        // Fields and methods are private by default too; an enum's variants go with the enum.
        let mut public = self.in_extern;
        let member = |name: &Name, m: &Name| Name { text: format!("{}.{}", name.text, m.text), span: m.span };
        loop {
            self.skip_newlines();
            match self.kind() {
                Kind::Sym("}") => break,
                Kind::Eof | Kind::Keyword("test" | "struct" | "copy" | "enum" | "interface") => {
                    let span = self.token().span;
                    self.diags.push(Diag::new(span, t!("struct_unclosed", name = name.text)));
                    return Ok(Struct { copy, name, generics, interfaces, fields, variants, methods });
                }
                Kind::Keyword(word @ ("public" | "private")) if self.t[self.pos + 1].kind == Kind::Sym(":") => {
                    public = word == "public";
                    self.advance();
                    self.advance();
                }
                Kind::Keyword("fn") => match self.function(true) {
                    Ok(f) => {
                        self.hide(public, &member(&name, &f.name));
                        methods.push(f)
                    }
                    Err(()) => self.skip_statement(),
                },
                _ if variants.is_some() => match self.variant() {
                    Ok(v) => variants.as_mut().unwrap().push(v),
                    Err(()) => self.skip_statement(),
                },
                _ => match self.field() {
                    Ok(f) => {
                        self.hide(public, &member(&name, &f.name));
                        fields.push(f)
                    }
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
                fields.push(Param { name, mode: Mode::Read, ty: self.type_()?, default: None });
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
        // `age: Int := 0`: the value of a literal that leaves it out.
        let default = if self.is_sym(":=") {
            self.advance();
            let e = self.expression()?;
            let text = self.src[e.span.start..e.span.end].to_string();
            Some((e, text))
        } else {
            None
        };
        // `tricks: List<String>` at the end of a line: the lexer took `>` for an operator
        // that continues the line, so the line break is only in the source.
        let line_break = self.src[self.t[self.pos - 1].span.end..self.token().span.start].contains('\n');
        if self.is_sym(",") {
            self.advance();
        } else if !line_break && !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
            let msg = t!("expected_symbol", symbol = ",", found = self.describe());
            return self.error(self.token().span, msg);
        }
        Ok(Param { name, mode: Mode::Read, ty, default })
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
            return self.function_rest(name, generics, None, params, vec![]);
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
        self.params(&mut params)?;
        // `fn put(value: Int) at(index: Int)`: a keyword message, named `put(…) at(…)`.
        let mut breaks = vec![];
        while self.next_part(self.t[self.pos - 1].span) && self.kind_at(1) == Kind::Sym("(") {
            breaks.push(params.len());
            let part = self.part_name()?;
            let open = self.advance().span;
            let before = params.len();
            self.params(&mut params)?;
            if params.len() == before {
                self.diags.push(Diag::new(open.to(self.t[self.pos - 1].span), t!("empty_part", name = part.text)));
            }
            add_part(&mut name, &part.text);
        }
        // `fn walk(self) far(km: Int)`: called `rex.walk far(5)`, so named `walk far(…)` as the call
        // is (like `l.map workers(8)`).
        if breaks.first() == Some(&0) {
            name.text = name.text.replacen("(…) ", " ", 1);
        }
        self.function_rest(name, generics, receiver, params, breaks)
    }

    /// The parameters up to the `)`, which is consumed.
    fn params(&mut self, params: &mut Vec<Param>) -> R<()> {
        while !self.is_sym(")") {
            let name = self.value_name("what_parameter")?;
            self.expect(":")?;
            let mode = self.mode();
            self.copy_mark(params.len());
            let ty = if self.is_sym("{") { self.block_type()? } else { self.type_()? };
            let default = if self.is_sym(":=") {
                self.advance();
                let e = self.expression()?;
                let text = self.src[e.span.start..e.span.end].to_string();
                Some((e, text))
            } else {
                None
            };
            params.push(Param { name, mode, ty, default });
            if !self.is_sym(")") {
                self.expect(",")?;
            }
        }
        self.advance(); // )
        Ok(())
    }

    /// `at(` (or `then {` at a call) on the same line as the `)` ending at `end`: the next part
    /// of a keyword message.
    /// A reserved word there (`between(1) and(3)`) is one too, refused by `part_name`;
    /// but not `else`, which may follow an expression.
    fn next_part(&self, end: Span) -> bool {
        matches!(self.kind(), Kind::Ident | Kind::Keyword(_))
            && !matches!(self.kind(), Kind::Keyword("else"))
            && (self.kind_at(1) == Kind::Sym("(") || self.kind_at(1) == Kind::Sym("{") && !self.no_literal)
            && !self.src[end.end..self.token().span.start].contains('\n')
    }

    /// `at(5)` after `grid.put(3)`: the other parts of a keyword message, whose arguments
    /// follow the first ones, and the message's name `put(…) at(…)`.
    fn more_parts(&mut self, name: &mut Name, args: &mut Vec<Expr>, end: &mut Span) -> R<()> {
        while self.next_part(*end) {
            // `Dog.named("Rex") then { … }`: a block alone needs no `()`, as in `l.each { … }`.
            if self.kind_at(1) == Kind::Sym("{") {
                let part = self.part_name()?;
                *end = self.trailing_block(args)?.expect("a `{` outside a heading");
                add_part(name, &part.text);
                break;
            }
            let part = self.part_name()?;
            let open = self.advance().span;
            let more = self.arguments()?;
            *end = self.advance().span;
            if more.is_empty() {
                self.diags.push(Diag::new(open.to(*end), t!("empty_part", name = part.text)));
            }
            args.extend(more);
            add_part(name, &part.text);
        }
        Ok(())
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

    /// In an `extern` block, `copy` before a struct: C takes or gives the struct itself, not its
    /// address (PLAN Part 4 "`extern`, structs by value").
    fn copy_mark(&mut self, number: usize) {
        if self.in_extern && self.is_keyword("copy") {
            self.advance();
            self.copies.push(number);
        }
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
    fn function_rest(
        &mut self,
        name: Name,
        generics: Vec<Generic>,
        receiver: Option<Mode>,
        params: Vec<Param>,
        breaks: Vec<usize>,
    ) -> R<Function> {
        let ret = if self.is_sym("->") {
            self.advance();
            self.copy_mark(params.len());
            Some(self.type_()?)
        } else {
            None
        };
        // In an `extern` block: maybe `from "c_name"`, never a body.
        if self.in_extern {
            if self.is_keyword("from") {
                self.advance();
                self.symbol = Some(self.c_name()?);
            }
            if !matches!(self.kind(), Kind::Newline | Kind::Sym("}") | Kind::Eof) {
                let msg = t!("extern_no_body", found = self.describe());
                return self.error(self.token().span, msg);
            }
            let at = self.token().span;
            let body = Block { open: at, stmts: vec![], close: at };
            return Ok(Function { name, generics, receiver, params, breaks, ret, contracts: vec![], only: None, body });
        }
        let (mut contracts, mut only) = (vec![], None);
        loop {
            self.skip_newlines();
            let kind = match self.kind() {
                Kind::Keyword("require") => ContractKind::Require,
                Kind::Keyword("ensure") => ContractKind::Ensure,
                // `mode build` / `mode run`: `mode` is not a keyword (a variable may be called so).
                Kind::Ident if self.text(self.token().span) == "mode" && self.kind_at(1) == Kind::Ident => {
                    let span = self.token().span;
                    let clause = self.only_in()?;
                    if only.is_some() {
                        self.diags.push(Diag::new(span.to(clause.span), t!("mode_twice")));
                    }
                    only = Some(clause);
                    continue;
                }
                _ => break,
            };
            contracts.push(self.contract(kind)?);
        }
        // In an interface, a method without a body: the types following it write theirs.
        if self.in_interface && matches!(self.kind(), Kind::Keyword("fn") | Kind::Sym("}")) {
            self.bodiless = true;
            let at = self.token().span;
            let body = Block { open: at, stmts: vec![], close: at };
            return Ok(Function { name, generics, receiver, params, breaks, ret, contracts, only, body });
        }
        // `fn area(self) -> Int := self.w * self.h`: one expression, the function's whole body.
        if self.is_sym(":=") {
            let open = self.advance().span;
            let value = self.expression()?;
            let span = value.span;
            let stmt = if ret.is_some() { Stmt::Return { value: Some(value), span } } else { Stmt::Expr(value) };
            let body = Block { open, stmts: vec![stmt], close: span };
            return Ok(Function { name, generics, receiver, params, breaks, ret, contracts, only, body });
        }
        let body = self.block()?;
        Ok(Function { name, generics, receiver, params, breaks, ret, contracts, only, body })
    }

    /// `mode build`, then `else "message"`, `else skip` or `else return value` (PLAN 2.14).
    fn only_in(&mut self) -> R<OnlyIn> {
        let start = self.advance().span; // mode
        let t = self.token();
        let build = match self.text(t.span).as_str() {
            "build" => true,
            "run" => false,
            _ => {
                let msg = t!("setting_bad_value", key = "mode", allowed = format!("`run` {} `build`", t!("word_or")), value = self.text(t.span));
                return self.error(t.span, msg);
            }
        };
        self.advance();
        let span = start.to(t.span);
        // `else` may be on the next line.
        let before = self.pos;
        self.skip_newlines();
        if !self.is_keyword("else") {
            self.pos = before;
            return Ok(OnlyIn { build, span, otherwise: Otherwise::Discreet });
        }
        let at = self.advance().span; // else
        let t = self.token();
        let otherwise = match t.kind {
            Kind::Str => {
                self.advance();
                Otherwise::Stop(self.text_parts(t))
            }
            Kind::Ident if self.text(t.span) == "skip" => {
                self.advance();
                Otherwise::Skip
            }
            Kind::Keyword("return") => {
                self.advance();
                if matches!(self.kind(), Kind::Newline | Kind::Sym("{") | Kind::Eof) {
                    self.diags.push(Diag::new(at.to(t.span), t!("mode_bare_return")).help(t!("mode_bare_return_help")));
                    return Err(());
                }
                Otherwise::Return(self.heading_expr()?)
            }
            _ => {
                let msg = t!("mode_else_expected", found = self.describe());
                self.diags.push(Diag::new(t.span, msg).help(t!("mode_else_help")));
                return Err(());
            }
        };
        Ok(OnlyIn { build, span, otherwise })
    }

    /// `require cond`, `ensure cond` or `check cond`, with an optional `else "message"`.
    fn contract(&mut self, kind: ContractKind) -> R<Contract> {
        self.advance(); // require, ensure or check
        // `require` and `ensure` come right before the body's `{`.
        let saved = std::mem::replace(&mut self.no_else, true);
        let cond = if kind == ContractKind::Check { self.expression() } else { self.heading_expr() };
        self.no_else = saved;
        let cond = cond?;
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
        self.block_rest(open)
    }

    /// A block's statements, after its `{`.
    fn block_rest(&mut self, open: Span) -> R<Block> {
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
                // `for var x in l`: the mark goes on what is walked over.
                if self.is_keyword("var") || self.is_keyword("move") {
                    let word = self.text(self.token().span);
                    self.diags.push(Diag::new(self.token().span, t!("for_mark_place", word = word)).help(t!("for_mark_place_help", word = word)));
                    return Err(());
                }
                let counter = self.value_name("what_variable")?;
                // `for name, age in ages`: a Map's key and value.
                let value = if self.is_sym(",") {
                    self.advance();
                    Some(self.value_name("what_variable")?)
                } else {
                    None
                };
                if !self.is_keyword("in") {
                    let msg = t!("expected_symbol", symbol = "in", found = self.describe());
                    return self.error(self.token().span, msg);
                }
                self.advance();
                let mode = if self.is_keyword("var") {
                    self.advance();
                    Mode::Var
                } else if self.is_keyword("move") {
                    self.advance();
                    Mode::Move
                } else {
                    Mode::Read
                };
                let e = self.heading_expr()?;
                let step = self.step()?;
                self.skip_newlines();
                let body = self.block()?;
                match e.kind {
                    ExprKind::Binary(op @ (BinOp::Range | BinOp::RangeInclusive), start, end) => {
                        if let Some(v) = &value {
                            self.diags.push(Diag::new(v.span, t!("for_two_names", type = "Int")));
                        }
                        if mode == Mode::Var {
                            self.diags.push(Diag::new(start.span.to(end.span), t!("for_var_list", type = "range")));
                        }
                        if mode == Mode::Move {
                            self.diags.push(Diag::new(start.span.to(end.span), t!("for_move_range")).help(t!("for_move_range_help")));
                        }
                        Ok(Stmt::Loop { counter, start: *start, end: *end, inclusive: op == BinOp::RangeInclusive, step: step.unwrap_or(1), body })
                    }
                    _ => {
                        if step.is_some() {
                            self.diags.push(Diag::new(e.span, t!("step_needs_range")));
                        }
                        Ok(Stmt::Each { name: counter, value, mode, list: e, body })
                    }
                }
            }
            // Another language's loop, or Kova's before it was removed.
            Kind::Keyword("from") => {
                let span = self.token().span;
                self.diags.push(Diag::new(span, t!("from_loop")).help(t!("from_loop_help")));
                Err(())
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
        let mode = if self.is_keyword("var") {
            self.advance();
            Mode::Var
        } else {
            Mode::Read
        };
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
        Ok(Match { mode, subject, arms, close: close? })
    }

    /// `Circle(r) => 3.14 * r * r` or `Red => { … }`.
    /// The variant of a case: `.Red`, written as in a value; `ok`, `err` and `none` have no dot there either.
    fn case_name(&mut self) -> R<Name> {
        if let Kind::Keyword(word @ ("ok" | "err" | "none")) = self.kind() {
            return Ok(Name { text: word.into(), span: self.advance().span });
        }
        if self.is_sym(".") {
            self.advance();
            return self.plain_type_name();
        }
        let name = self.plain_type_name()?;
        self.diags.push(Diag::new(name.span, t!("case_dot", name = name.text)));
        Ok(name)
    }

    fn arm(&mut self) -> R<Arm> {
        let case = if self.kind() == Kind::Ident && self.text(self.token().span) == "_" {
            Case::Any(self.advance().span)
        } else if self.is_keyword("none") {
            let span = self.advance().span;
            Case::Variant { name: Name { text: "none".into(), span }, binds: vec![] }
        } else if matches!(self.kind(), Kind::Int(_) | Kind::Big | Kind::Char(_) | Kind::Str) || self.is_sym("-") {
            // From the level of `..`: `1..=5` is a case too.
            let range = LEVELS.iter().position(|(ops, _)| ops[0].0 == "..").unwrap();
            let mut values = vec![self.level(range)?];
            while self.is_sym(",") {
                self.advance();
                values.push(self.level(range)?);
            }
            Case::Values(values)
        } else if matches!(self.kind(), Kind::Ident) || self.is_sym("(") {
            Case::Bind(self.pattern()?)
        } else {
            let name = self.case_name()?;
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
        let mut also = vec![];
        if matches!(case, Case::Variant { .. }) {
            while self.is_sym(",") {
                self.advance();
                let name = self.case_name()?;
                if self.is_sym("(") {
                    let open = self.advance().span;
                    while !self.is_sym(")") && self.kind() != Kind::Eof {
                        self.advance();
                    }
                    let close = self.expect(")")?;
                    self.diags.push(Diag::new(open.to(close), t!("grouped_fields")).help(t!("grouped_fields_help")));
                }
                also.push(name);
            }
            if let Case::Variant { name, binds } = &case
                && !also.is_empty()
                && !binds.is_empty()
            {
                self.diags.push(Diag::new(name.span, t!("grouped_fields")).help(t!("grouped_fields_help")));
            }
        }
        let arrow = self.expect("=>")?;
        self.skip_newlines();
        let body = if self.is_sym("{") {
            self.block()?
        } else {
            // From the `=>` to what follows the statement (`,`, line end, `}`): the editor's view of the arm.
            // An unfinished statement (while typing) keeps the arm, so that its names are offered.
            let at = self.pos;
            let s = self.stmt();
            // `.A =>` with nothing after it: the next line is the next case, not its body.
            if s.is_ok() && self.is_sym("=>") {
                self.pos = at;
                self.diags.push(Diag::new(arrow, t!("case_empty")).help(t!("case_empty_help")));
                return Ok(Arm { case, also, body: Block { open: arrow, stmts: vec![], close: self.token().span } });
            }
            if s.is_err() {
                self.skip_statement();
            }
            Block { open: arrow, stmts: s.into_iter().collect(), close: self.token().span }
        };
        self.separator()?;
        Ok(Arm { case, also, body })
    }

    // ---- Expressions ----

    fn expression(&mut self) -> R<Expr> {
        let e = self.level(0)?;
        if self.no_else || !self.is_keyword("else") {
            return Ok(e);
        }
        self.advance();
        let fallback = self.expression()?;
        Ok(self.expr(e.span.to(fallback.span), ExprKind::Else(Box::new(e), Box::new(fallback))))
    }

    fn level(&mut self, n: usize) -> R<Expr> {
        let Some(&(ops, chainable)) = LEVELS.get(n) else {
            return self.unary();
        };
        let mut left = self.level(n + 1)?;
        // `a is Dog`, as high as `=`: `a is Dog && a.age > 3`. `is` is contextual: `is(…)` stays a
        // part of a keyword message (`.header("Host") is("x")`).
        if ops[0].0 == "=" && self.kind() == Kind::Ident && self.text(self.token().span) == "is" && self.kind_at(1) == Kind::Type {
            self.advance();
            let ty = self.type_()?;
            let span = left.span.to(self.t[self.pos - 1].span);
            return Ok(self.expr(span, ExprKind::Is(Box::new(left), ty)));
        }
        let mut seen = false;
        loop {
            // `>>` is two `>`, stuck together: the end of `List<List<Int>>` is too.
            let shift = ops[0].0 == "<<"
                && self.is_sym(">")
                && self.kind_at(1) == Kind::Sym(">")
                && self.token().span.end == self.t[self.pos + 1].span.start;
            if shift {
                self.advance();
            }
            let Some(&(s, op)) = ops.iter().find(|(s, _)| if shift { *s == ">>" } else { self.is_sym(s) }) else { break };
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
            let mut name = self.member_name("what_method")?;
            let (mut args, mut end) = (vec![], name.span);
            if self.is_sym("(") {
                let open = self.advance().span;
                args = self.arguments()?;
                end = self.advance().span;
                self.empty_parens(&args, open, end, &name.text);
                self.more_parts(&mut name, &mut args, &mut end)?;
            } else if self.kind() == Kind::Ident && self.kind_at(1) == Kind::Sym("(") && !self.src[end.end..self.token().span.start].contains('\n') {
                // `l.map workers(8) { … }`: a part after a method without arguments, named
                // `map workers(…)`.
                self.more_parts(&mut name, &mut args, &mut end)?;
                name.text = name.text.replacen("(…) ", " ", 1);
            }
            end = self.trailing_block(&mut args)?.unwrap_or(end);
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
                Mode::Read if self.is_sym("{") => self.block_literal()?,
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

    /// `mode = build` or `mode != run`, which Kova no longer has (PLAN Part 4 "Three things
    /// undone"): `mode` followed by anything else is an ordinary name.
    fn mode_test(&self) -> bool {
        let then = self.t[(self.pos + 2).min(self.t.len() - 1)];
        matches!(self.t[self.pos + 1].kind, Kind::Sym("=" | "!=")) && then.kind == Kind::Ident && matches!(self.text(then.span).as_str(), "run" | "build")
    }

    fn in_mode(&mut self) -> R<Expr> {
        let start = self.advance().span; // mode
        self.advance();
        let end = self.advance().span;
        self.diags.push(Diag::new(start.to(end), t!("mode_test_gone")).help(t!("mode_test_gone_help")));
        Err(())
    }

    /// After a function's name: its arguments, if any (`seven` alone calls `fn seven`).
    fn call_rest(&mut self, mut name: Name) -> R<Expr> {
        let (mut args, mut end) = (vec![], name.span);
        if self.is_sym("(") {
            let open = self.advance().span;
            args = self.arguments()?;
            end = self.advance().span;
            self.empty_parens(&args, open, end, &name.text);
            self.more_parts(&mut name, &mut args, &mut end)?;
        }
        end = self.trailing_block(&mut args)?.unwrap_or(end);
        if name.text == "ui.run" || name.text.starts_with("ui.run(…) ") {
            self.ui_run(name.span.to(end), &mut args);
        }
        if name.text == "ui.column(…) align(…)" || name.text == "ui.row(…) align(…)" {
            self.ui_align(&name.text, &mut args);
        }
        Ok(self.expr(name.span.to(end), ExprKind::Call { name, args }))
    }

    /// `ui.run(title, state, view, update)`, perhaps followed by `width(…) height(…)`: the two
    /// functions, named, go into `File::ui_runs` and leave the call.
    fn ui_run(&mut self, call: Span, args: &mut Vec<Expr>) {
        let named = |e: &Expr| match &e.kind {
            ExprKind::Var(n) => Some(Name { text: n.clone(), span: e.span }),
            _ => None,
        };
        match args.get(2..4).map(|a| a.iter().map(named).collect::<Vec<_>>()).as_deref() {
            Some([Some(view), Some(update)]) => {
                self.ui_runs.push(UiRun { call, view: view.clone(), update: update.clone() });
                args.drain(2..4);
            }
            _ => self.diags.push(Diag::new(call, t!("ui_run_shape"))),
        }
    }

    /// `ui.column(…) align(center)`: the word stands for `ui.Align.Center`. A column's parts go
    /// from left to right, a row's from top to bottom.
    fn ui_align(&mut self, call: &str, args: &mut [Expr]) {
        let words = if call.starts_with("ui.column") { ["left", "center", "right"] } else { ["top", "center", "bottom"] };
        let Some(Expr { kind: ExprKind::Var(word), span, .. }) = args.last() else { return };
        let span = *span;
        if !words.contains(&word.as_str()) {
            self.diags.push(Diag::new(span, t!("ui_align_word", word = word, words = words.join("`, `"))));
            return;
        }
        let mut variant = word.clone();
        variant[..1].make_ascii_uppercase();
        args.last_mut().unwrap().kind = ExprKind::Variant {
            ty: Name { text: "ui.Align".into(), span },
            types: vec![],
            name: Name { text: variant, span },
            args: vec![],
        };
    }

    /// `l.each { x: Int => … }`: a block given last, after the `)` and on the same line
    /// (a line break before it would have made a `Newline`). Not in a heading (PLAN 2.3).
    fn trailing_block(&mut self, args: &mut Vec<Expr>) -> R<Option<Span>> {
        if !self.is_sym("{") {
            return Ok(None);
        }
        // `if l.any { x: Int => … } {`: no statement starts with `x:`, so this `{` opens a block
        // and not the `if`'s body. Read as such, with one clear message.
        let params = self.kind_at(1) == Kind::Ident && self.kind_at(2) == Kind::Sym(":");
        if self.no_literal && params {
            let b = self.block_literal()?;
            self.diags.push(Diag::new(b.span, t!("block_in_heading")).help(t!("block_in_heading_help")));
            let end = b.span;
            args.push(b);
            return Ok(Some(end));
        }
        if self.no_literal {
            return Ok(None);
        }
        let b = self.block_literal()?;
        let end = b.span;
        args.push(b);
        Ok(Some(end))
    }

    /// `{ x: Int, y: Int => … }` or `{ … }` without parameters.
    /// `step 2` or `step -1` after a range: a written number, which says which way the loop goes.
    fn step(&mut self) -> R<Option<i64>> {
        if self.kind() != Kind::Ident || self.text(self.token().span) != "step" {
            return Ok(None);
        }
        self.advance();
        let start = self.token().span;
        let minus = self.is_sym("-");
        if minus {
            self.advance();
        }
        let Kind::Int(v) = self.kind() else {
            return self.error(start, t!("step_number"));
        };
        let v = v as i64;
        let span = self.advance().span;
        if v == 0 {
            self.diags.push(Diag::new(start.to(span), t!("step_number")));
            return Ok(Some(1));
        }
        Ok(Some(if minus { -v } else { v }))
    }

    fn block_literal(&mut self) -> R<Expr> {
        let open = self.advance().span; // {
        let mut params = vec![];
        if self.kind() == Kind::Ident && matches!(self.t[self.pos + 1].kind, Kind::Sym(":")) {
            loop {
                let name = self.value_name("what_parameter")?;
                self.expect(":")?;
                let ty = if self.is_sym("{") { self.block_type()? } else { self.type_()? };
                params.push(Param { name, mode: Mode::Read, ty, default: None });
                if !self.is_sym(",") {
                    break;
                }
                self.advance();
            }
            self.expect("=>")?;
        }
        let saved = std::mem::replace(&mut self.no_literal, false);
        let body = self.block_rest(open);
        self.no_literal = saved;
        let body = body?;
        Ok(self.expr(open.to(body.close), ExprKind::Block { params, body }))
    }

    fn atom(&mut self) -> R<Expr> {
        let t = self.token();
        let kind = match t.kind {
            Kind::Int(v) => ExprKind::Int(v),
            Kind::Big => {
                let digits = self.text(t.span).replace('_', "");
                ExprKind::BigInt(digits.trim_start_matches('0').to_string())
            }
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
                let mut name = self.member_name("what_function")?;
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
                let block = self.is_sym("{") && !self.no_literal;
                if !self.is_sym("(") && !block && !self.no_params.contains(&name.text) {
                    return Ok(self.expr(name.span, ExprKind::Var(name.text.clone())));
                }
                name.text = self.qualified(name.text);
                return self.call_rest(name);
            }
            // `MAX`: a constant whose name is in capitals.
            Kind::Type if self.constants.contains(&self.text(t.span)) && self.kind_at(1) != Kind::Sym(".") => {
                let mut name = Name { text: self.text(t.span), span: t.span };
                self.advance();
                name.text = self.qualified(name.text);
                return self.call_rest(name);
            }
            Kind::Keyword("if") => return self.if_expr(),
            // `ok` alone: the success of a `Nothing or E`.
            Kind::Keyword("ok") if self.kind_at(1) != Kind::Sym("(") => {
                let span = self.advance().span;
                let nothing = self.expr(span, ExprKind::Tuple(vec![]));
                return Ok(self.expr(span, ExprKind::Outcome { ok: true, value: Box::new(nothing) }));
            }
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
            // `.Red`: the case of the enum expected here, whose name is written elsewhere.
            Kind::Sym(".") if self.kind_at(1) == Kind::Type => {
                self.advance();
                let name = self.plain_type_name()?;
                let (mut args, mut end) = (vec![], name.span);
                if self.is_sym("(") {
                    let open = self.advance().span;
                    args = self.arguments()?;
                    end = self.advance().span;
                    self.empty_parens(&args, open, end, &name.text);
                }
                let ty = Name { text: String::new(), span: t.span };
                return Ok(self.expr(t.span.to(end), ExprKind::Variant { ty, types: vec![], name, args }));
            }
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
                let (mut elems, mut pairs) = (vec![], vec![]);
                if self.is_sym(":") && self.kind_at(1) == Kind::Sym("]") {
                    return self.error(self.token().span, t!("empty_map_colon"));
                }
                while !self.is_sym("]") {
                    let x = self.inner_expr()?;
                    // `["Rex" := 3]`: a Map, as soon as the first element has its `:=`.
                    if self.is_sym(":=") || !pairs.is_empty() {
                        self.expect(":=")?;
                        pairs.push((x, self.inner_expr()?));
                    } else {
                        elems.push(x);
                    }
                    if !self.is_sym("]") {
                        self.expect(",")?;
                    }
                }
                let end = self.advance().span;
                let kind = if pairs.is_empty() { ExprKind::List(elems) } else { ExprKind::MapLit(pairs) };
                return Ok(self.expr(t.span.to(end), kind));
            }
            // A block is only given to a function: written anywhere else, it would outlive its call.
            Kind::Sym("{") => return self.error(t.span, t!("block_not_argument")),
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
        // `Set["a", "b"]`: written apart from a List (PLAN Part 4 "Sets written `Set[…]`").
        if self.text(self.token().span) == "Set" && self.kind_at(1) == Kind::Sym("[") {
            let start = self.advance().span;
            self.advance(); // [
            let mut elems = vec![];
            while !self.is_sym("]") {
                elems.push(self.inner_expr()?);
                if !self.is_sym("]") {
                    self.expect(",")?;
                }
            }
            let end = self.advance().span;
            return Ok(self.expr(start.to(end), ExprKind::SetLit(elems)));
        }
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
            let mut name = if variant { self.plain_type_name()? } else { self.value_name("what_function")? };
            let (mut args, mut end) = (vec![], name.span);
            if self.is_sym("(") {
                let open = self.advance().span;
                args = self.arguments()?;
                end = self.advance().span;
                self.empty_parens(&args, open, end, &name.text);
                if !variant {
                    self.more_parts(&mut name, &mut args, &mut end)?;
                }
            }
            // `Dog.make(3) { … }`
            if !variant {
                end = self.trailing_block(&mut args)?.unwrap_or(end);
            }
            let span = ty.span.to(end);
            let kind = if variant { ExprKind::Variant { ty, types, name, args } } else { ExprKind::Static { ty, types, name, args } };
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
        let (fields, base, end) = fields?;
        Ok(self.expr(ty.span.to(end), ExprKind::StructLit { ty, types, fields, base }))
    }

    /// `name := "Rex", age := 3 }`: separated by `,` or a line break. Returns the `}` too.
    fn literal_fields(&mut self) -> R<(Vec<(Name, Expr)>, Option<Box<Expr>>, Span)> {
        let (mut fields, mut base) = (vec![], None);
        loop {
            self.skip_newlines();
            if self.is_sym("}") {
                return Ok((fields, base, self.advance().span));
            }
            if self.is_sym("...") {
                self.advance();
                base = Some(Box::new(self.expression()?));
            } else {
                let name = self.value_name("what_field")?;
                // `Dog { name }`: the variable of that name.
                let value = if matches!(self.kind(), Kind::Sym("," | "}") | Kind::Newline) {
                    self.expr(name.span, ExprKind::Var(name.text.clone()))
                } else {
                    self.expect(":=")?;
                    self.expression()?
                };
                fields.push((name, value));
            }
            if self.is_sym(",") {
                self.advance();
            } else if !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
                let msg = t!("expected_symbol", symbol = ",", found = self.describe());
                return self.error(self.token().span, msg);
            }
        }
    }
}

/// `put` then `at` → `put(…) at(…)`: a keyword message's name, which no identifier can take.
fn add_part(name: &mut Name, part: &str) {
    if !name.text.ends_with(')') {
        name.text += "(…)";
    }
    name.text += &format!(" {part}(…)");
}
