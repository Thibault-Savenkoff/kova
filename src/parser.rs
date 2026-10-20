// Parser: turns the token list into a tree (AST).
//
// It doesn't stop at the first error: it records it, skips to the end of the
// statement, then continues. So all errors are seen at once.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{Kind, Token, string_content};

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
    let mut p = Parser { t: tokens, pos: 0, src: source, diags: vec![], ids: 0 };
    let file = p.file();
    (file, p.ids, p.diags)
}

struct Parser<'a> {
    t: Vec<Token>,
    pos: usize,
    src: &'a str,
    diags: Vec<Diag>,
    ids: usize,
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
            Kind::Newline => "une fin de ligne".into(),
            Kind::Eof => "la fin du fichier".into(),
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
        let msg = format!("`{s}` attendu, trouvé {}", self.describe());
        self.error(self.token().span, msg)
    }
    fn expr(&mut self, span: Span, kind: ExprKind) -> Expr {
        self.ids += 1;
        Expr { id: self.ids - 1, span, kind }
    }

    /// A variable or function name: lowercase required.
    fn value_name(&mut self, what: &str) -> R<Name> {
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
                    Diag::new(t.span, format!("un nom de {what} commence par une minuscule"))
                        .help(format!("écris `{lowercase}` (la majuscule est réservée aux types)")),
                );
                Err(())
            }
            Kind::Keyword(k) => {
                let msg = format!("`{k}` est un mot réservé de Kova : il ne peut pas servir de nom");
                self.error(t.span, msg)
            }
            _ => {
                let msg = format!("nom de {what} attendu, trouvé {}", self.describe());
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
            Kind::Ident => self.error(t.span, "un nom de type commence par une majuscule"),
            _ => {
                let msg = format!("type attendu, trouvé {}", self.describe());
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
            return self.error(open.to(close), "un tuple a au moins 2 éléments");
        }
        Ok(TypeAst::Tuple(types))
    }

    /// `x`, `_` or `(a, b)`.
    fn pattern(&mut self) -> R<Pattern> {
        if self.kind() == Kind::Ident && self.text(self.token().span) == "_" {
            return Ok(Pattern::Ignore(self.advance().span));
        }
        if !self.is_sym("(") {
            return Ok(Pattern::Name(self.value_name("variable")?));
        }
        let open = self.advance().span;
        let mut patterns = vec![self.pattern()?];
        while self.is_sym(",") {
            self.advance();
            patterns.push(self.pattern()?);
        }
        let close = self.expect(")")?;
        if patterns.len() < 2 {
            return self.error(open.to(close), "un tuple a au moins 2 éléments");
        }
        Ok(Pattern::Tuple(patterns, open.to(close)))
    }

    /// `x`, `t.0` or `(a, b)` left of `:=`: re-reads the expression already parsed.
    fn target(&mut self, e: Expr) -> R<Target> {
        let span = e.span;
        match e.kind {
            ExprKind::Var(name) => Ok(Target::Place { name: Name { text: name, span }, fields: vec![], span }),
            ExprKind::Field(base, i) => match self.target(*base)? {
                Target::Place { name, mut fields, .. } => {
                    fields.push(i);
                    Ok(Target::Place { name, fields, span })
                }
                Target::Tuple(..) => self.error(span, "on ne peut donner une valeur qu'à une variable"),
            },
            ExprKind::Tuple(elems) => {
                let targets = elems.into_iter().map(|e| self.target(e)).collect::<R<Vec<_>>>()?;
                Ok(Target::Tuple(targets, span))
            }
            _ => self.error(span, "on ne peut donner une valeur qu'à une variable"),
        }
    }

    // ---- File and functions ----

    fn file(&mut self) -> File {
        let (mut header, mut functions, mut tests) = (None, vec![], vec![]);
        loop {
            self.skip_newlines();
            let read = match self.kind() {
                Kind::Eof => break,
                Kind::Keyword("header") => {
                    let first = header.is_none() && functions.is_empty() && tests.is_empty();
                    if !first {
                        let span = self.token().span;
                        self.diags.push(Diag::new(span, "le `header` doit être la première déclaration du fichier"));
                    }
                    self.header().map(|h| {
                        if first {
                            header = Some(h);
                        }
                    })
                }
                Kind::Keyword("fn") => self.function().map(|f| functions.push(f)),
                Kind::Keyword("test") => self.test().map(|t| tests.push(t)),
                _ => {
                    let msg = format!("`fn` ou `test` attendu, trouvé {}", self.describe());
                    self.error(self.token().span, msg)
                }
            };
            if read.is_err() {
                self.skip_to_next_fn();
            }
        }
        File { header, functions, tests }
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
                self.diags.push(Diag::new(span, "`}` manquante à la fin du `header`"));
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
        let key = self.value_name("réglage")?;
        self.expect(":=")?;
        let t = self.token();
        if !matches!(t.kind, Kind::Ident | Kind::Keyword(_)) {
            let msg = format!("valeur du réglage attendue (un mot comme `build` ou `true`), trouvé {}", self.describe());
            return self.error(t.span, msg);
        }
        self.advance();
        if !matches!(self.kind(), Kind::Newline | Kind::Sym("}")) {
            let msg = format!("un réglage par ligne : fin de ligne attendue, trouvé {}", self.describe());
            return self.error(self.token().span, msg);
        }
        Ok((key, Name { text: self.text(t.span), span: t.span }))
    }

    fn skip_to_next_fn(&mut self) {
        self.advance();
        while !matches!(self.kind(), Kind::Keyword("fn" | "test") | Kind::Eof) {
            self.advance();
        }
    }

    fn test(&mut self) -> R<Test> {
        self.advance(); // test
        let t = self.token();
        if t.kind != Kind::Str {
            let msg = format!("nom du test entre guillemets attendu, trouvé {}", self.describe());
            self.diags.push(Diag::new(t.span, msg).help("exemple : `test \"carré de 3\" { check carre(3) = 9 }`"));
            return Err(());
        }
        self.advance();
        self.skip_newlines();
        let body = self.block()?;
        Ok(Test { name: string_content(&self.text(t.span)), span: t.span, body })
    }

    fn function(&mut self) -> R<Function> {
        self.advance(); // fn
        let name = self.value_name("fonction")?;
        self.expect("(")?;
        let mut params = vec![];
        while !self.is_sym(")") {
            let name = self.value_name("paramètre")?;
            self.expect(":")?;
            params.push(Param { name, ty: self.type_()? });
            if !self.is_sym(")") {
                self.expect(",")?;
            }
        }
        self.advance(); // )
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
        Ok(Function { name, params, ret, contracts, body })
    }

    /// `require cond`, `ensure cond` or `check cond`, with an optional `else "message"`.
    fn contract(&mut self, kind: ContractKind) -> R<Contract> {
        self.advance(); // require, ensure or check
        let cond = self.expression()?;
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
            let msg = format!("message entre guillemets attendu après `else`, trouvé {}", self.describe());
            let help = format!("exemple : `{} b != 0 else \"b ne doit pas valoir 0\"`", kind.keyword());
            self.diags.push(Diag::new(t.span, msg).help(help));
            return Err(());
        }
        self.advance();
        Ok(Contract { kind, cond, message: Some(string_content(&self.text(t.span))) })
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
            if matches!(self.kind(), Kind::Eof | Kind::Keyword("fn" | "test")) {
                let span = self.token().span;
                self.diags.push(Diag::new(span, "`}` manquante à la fin du bloc"));
                return Ok(Block { open, stmts, close: span });
            }
            match self.stmt() {
                Ok(s) => {
                    stmts.push(s);
                    if !matches!(self.kind(), Kind::Newline | Kind::Sym("}") | Kind::Eof) {
                        let msg = format!("fin d'instruction attendue, trouvé {}", self.describe());
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
                let msg = format!("`{k}` s'écrit entre la signature de la fonction et son `{{`");
                self.diags.push(Diag::new(start, msg).help("à l'intérieur d'une fonction, utilise `check`"));
                Err(())
            }
            Kind::Keyword("while") => {
                self.advance();
                let cond = self.expression()?;
                self.skip_newlines();
                Ok(Stmt::While { cond, body: self.block()? })
            }
            Kind::Keyword("for") => {
                self.advance();
                let counter = self.value_name("variable")?;
                if !self.is_keyword("in") {
                    let msg = format!("`in` attendu, trouvé {}", self.describe());
                    return self.error(self.token().span, msg);
                }
                self.advance();
                let e = self.expression()?;
                let ExprKind::Binary(op @ (BinOp::Range | BinOp::RangeInclusive), start, end) = e.kind else {
                    self.diags.push(
                        Diag::new(e.span, "en J1, `for` ne parcourt que des intervalles")
                            .help("exemple : `for i in 0..10` (sans 10) ou `for i in 0..=10` (avec 10)"),
                    );
                    return Err(());
                };
                self.skip_newlines();
                let body = self.block()?;
                Ok(Stmt::Loop { counter, start: *start, end: *end, inclusive: op == BinOp::RangeInclusive, body })
            }
            Kind::Keyword("from") => {
                self.advance();
                let counter = self.value_name("variable")?;
                self.expect(":=")?;
                let start = self.expression()?;
                if !self.is_keyword("to") {
                    let msg = format!("`to` attendu, trouvé {}", self.describe());
                    return self.error(self.token().span, msg);
                }
                self.advance();
                let end = self.expression()?;
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
        let cond = self.expression()?;
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
        let cond = self.expression()?;
        self.skip_newlines();
        let then = self.block()?;
        self.skip_newlines();
        if !self.is_keyword("else") {
            self.diags.push(
                Diag::new(start, "un `if` qui donne une valeur a besoin d'un `else`")
                    .help("ajoute `else { ... }` : la valeur doit exister dans tous les cas"),
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
                    Diag::new(span, format!("`{s}` ne peut pas s'enchaîner"))
                        .help("pour `a < b < c`, écris `a < b && b < c`"),
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

    /// An atom followed by its suffixes: `x.to_float()`.
    fn primary(&mut self) -> R<Expr> {
        let mut e = self.atom()?;
        while self.is_sym(".") {
            self.advance();
            if let Kind::Int(i) = self.kind() {
                let end = self.advance().span;
                e = self.expr(e.span.to(end), ExprKind::Field(Box::new(e), i as usize));
                continue;
            }
            let name = self.value_name("méthode")?;
            self.expect("(")?;
            let args = self.arguments()?;
            let end = self.advance().span;
            e = self.expr(e.span.to(end), ExprKind::Method { receiver: Box::new(e), name, args });
        }
        Ok(e)
    }

    /// The arguments of a call, after the `(`; stops at the `)` without consuming it.
    fn arguments(&mut self) -> R<Vec<Expr>> {
        let mut args = vec![];
        while !self.is_sym(")") {
            args.push(self.expression()?);
            if !self.is_sym(")") {
                self.expect(",")?;
            }
        }
        Ok(args)
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
            Kind::Str => {
                let msg = "les textes arrivent au jalon J3 (pour l'instant : seulement après `else` dans un contrat)";
                return self.error(t.span, msg);
            }
            Kind::Ident => {
                let name = self.value_name("variable")?;
                if !self.is_sym("(") {
                    return Ok(self.expr(name.span, ExprKind::Var(name.text)));
                }
                self.advance();
                let args = self.arguments()?;
                let end = self.advance().span;
                return Ok(self.expr(name.span.to(end), ExprKind::Call { name, args }));
            }
            Kind::Keyword("if") => return self.if_expr(),
            Kind::Sym("(") => {
                self.advance();
                let e = self.expression()?;
                if !self.is_sym(",") {
                    let end = self.expect(")")?;
                    return Ok(Expr { span: t.span.to(end), ..e });
                }
                // `(a, b)`: a tuple.
                let mut elems = vec![e];
                while self.is_sym(",") {
                    self.advance();
                    elems.push(self.expression()?);
                }
                let end = self.expect(")")?;
                return Ok(self.expr(t.span.to(end), ExprKind::Tuple(elems)));
            }
            _ => {
                let msg = format!("expression attendue, trouvé {}", self.describe());
                return self.error(t.span, msg);
            }
        };
        self.advance();
        Ok(self.expr(t.span, kind))
    }
}
