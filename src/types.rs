// Type checking: known names, correct types, `let`/`var` rules.
//
// Kova guesses (almost) nothing: it checks that what is written is consistent.
//
//   let x: Int := true    →  erreur: types différents : `Int` attendu, `Bool` donné
//   a := 2  (a is a let)  →  erreur: `a` est déclarée avec let
//
// The result: the type of each expression (indexed by `Expr::id`).

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diag, Span, line_col};

#[derive(Clone, PartialEq, Debug)]
pub enum Type {
    Int,
    Float,
    Bool,
    Char,
    /// `(Int, Float)`: at least 2 elements.
    Tuple(Vec<Type>),
    /// What a function without `->` returns.
    Unit,
    /// Unknown type because of an error already reported: it is not repeated.
    Error,
}

impl Type {
    /// The type as in the code: `Int`, `(Int, Float)`.
    pub fn text(&self) -> String {
        match self {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::Bool => "Bool".into(),
            Type::Char => "Char".into(),
            Type::Tuple(ts) => format!("({})", ts.iter().map(Type::text).collect::<Vec<_>>().join(", ")),
            Type::Unit => "rien".into(),
            Type::Error => "?".into(),
        }
    }
}

/// The `header` keys and their allowed values; the first one is the default.
pub const SETTINGS: &[(&str, &[&str], &str)] = &[
    ("mode", &["run", "build"], "ce que fait `kova fichier.kova`"),
    ("lock", &["false", "true"], "true : l'autre commande est refusée"),
];

/// The value of a (checked) `header` setting, and its line; `None` if absent.
pub fn setting<'a>(f: &'a File, src: &str, key: &str) -> Option<(&'a str, usize)> {
    let (_, v) = f.header.as_ref()?.settings.iter().find(|(k, _)| k.text == key)?;
    Some((v.text.as_str(), line_col(src, v.span.start).0))
}

/// Built-in functions: `print(x)` ends the line, `write(x)` doesn't.
pub const BUILTINS: &[&str] = &["print", "write"];

/// Built-in methods (J2: conversions): (receiver type, name, result).
pub const METHODS: &[(Type, &str, Type)] = &[
    (Type::Int, "to_float", Type::Float),
    (Type::Float, "to_int", Type::Int),
    (Type::Char, "to_int", Type::Int),
];

pub struct Info {
    /// Type of each expression, by `Expr::id`.
    pub types: Vec<Type>,
}

#[derive(Clone, Copy, PartialEq)]
enum VarKind {
    Let,
    Var,
    Param,
    Counter,
}

struct Variable {
    ty: Type,
    kind: VarKind,
    span: Span,
}

struct Signature {
    params: Vec<Type>,
    ret: Type,
}

struct Checker<'a> {
    src: &'a str,
    functions: HashMap<&'a str, Signature>,
    /// One table per open block, from outside to inside.
    scopes: Vec<HashMap<String, Variable>>,
    ret: Type,
    /// The type of `result`: `Some` only inside an `ensure`.
    result: Option<Type>,
    loops: usize,
    types: Vec<Type>,
    diags: Vec<Diag>,
}

pub fn check(f: &File, nb_exprs: usize, src: &str) -> (Info, Vec<Diag>) {
    let mut c = Checker {
        src,
        functions: HashMap::new(),
        scopes: vec![],
        ret: Type::Unit,
        result: None,
        loops: 0,
        types: vec![Type::Error; nb_exprs],
        diags: vec![],
    };

    // 0. The header.
    let mut seen: HashMap<&str, Span> = HashMap::new();
    for (key, value) in f.header.iter().flat_map(|h| &h.settings) {
        let Some((_, allowed, _)) = SETTINGS.iter().find(|(k, ..)| *k == key.text) else {
            let keys: Vec<String> = SETTINGS.iter().map(|(k, ..)| format!("`{k}`")).collect();
            c.diags.push(
                Diag::new(key.span, format!("réglage inconnu `{}` dans le header", key.text))
                    .help(format!("réglages possibles : {}", keys.join(", "))),
            );
            continue;
        };
        if let Some(first) = seen.insert(&key.text, key.span) {
            let (line, _) = line_col(src, first.start);
            c.error(key.span, format!("`{}` est déjà réglé (ligne {line})", key.text));
        }
        if !allowed.contains(&value.text.as_str()) {
            let allowed: Vec<String> = allowed.iter().map(|a| format!("`{a}`")).collect();
            c.error(value.span, format!("`{}` vaut {}, pas `{}`", key.text, allowed.join(" ou "), value.text));
        }
    }

    // 1. Signatures first: a function written further down can be called.
    for func in &f.functions {
        for t in func.params.iter().map(|p| &p.ty).chain(&func.ret) {
            c.report_unknown(t);
        }
        let params = func.params.iter().map(|p| type_of(&p.ty)).collect();
        let ret = func.ret.as_ref().map_or(Type::Unit, type_of);
        let name = func.name.text.as_str();
        if BUILTINS.contains(&name) {
            c.error(func.name.span, format!("`{name}` est une fonction prédéfinie"));
        } else if c.functions.contains_key(name) {
            c.error(func.name.span, format!("la fonction `{name}` existe déjà"));
        } else {
            c.functions.insert(name, Signature { params, ret });
        }
    }

    // 2. `fn main()`: required, without parameters or result.
    match f.functions.iter().find(|f| f.name.text == "main") {
        None => c.error(Span::new(0, 0), "il manque la fonction `main`"),
        Some(m) if !m.params.is_empty() || m.ret.is_some() => {
            c.diags.push(Diag::new(m.name.span, "`main` ne prend pas de paramètre et ne renvoie rien"));
        }
        _ => {}
    }

    // 3. The bodies.
    for func in &f.functions {
        c.ret = func.ret.as_ref().map_or(Type::Unit, type_of);
        let mut params = HashMap::new();
        for p in &func.params {
            if params.contains_key(&p.name.text) {
                c.error(p.name.span, format!("le paramètre `{}` existe déjà", p.name.text));
            }
            params.insert(p.name.text.clone(), Variable { ty: type_of(&p.ty), kind: VarKind::Param, span: p.name.span });
        }
        c.scopes = vec![params];
        // Contracts only see the parameters (and `result` in an `ensure`).
        for k in &func.contracts {
            c.result = (k.kind == ContractKind::Ensure).then(|| c.ret.clone());
            c.expect(&k.cond, &Type::Bool);
        }
        c.result = None;
        c.block(&func.body, vec![]);
        if c.ret != Type::Unit && !always_returns(&func.body) {
            c.diags.push(
                Diag::new(func.body.close, format!("`{}` peut arriver ici sans `return`", func.name.text))
                    .help(format!("la fonction doit renvoyer un `{}` dans tous les cas", c.ret.text())),
            );
        }
    }

    // 4. Tests: checked even by `kova run`, so that errors show up right away.
    let mut names: HashMap<&str, Span> = HashMap::new();
    for t in &f.tests {
        if let Some(first) = names.insert(&t.name, t.span) {
            let (line, _) = line_col(src, first.start);
            c.error(t.span, format!("le test « {} » existe déjà (ligne {line})", t.name));
        }
        c.ret = Type::Unit;
        c.scopes = vec![];
        c.block(&t.body, vec![]);
    }

    (Info { types: c.types }, c.diags)
}

/// The written type; `Error` if it contains an unknown name.
pub fn type_of(t: &TypeAst) -> Type {
    match t {
        TypeAst::Name(n) => match n.text.as_str() {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "Bool" => Type::Bool,
            "Char" => Type::Char,
            _ => Type::Error,
        },
        TypeAst::Tuple(ts) => {
            let ts: Vec<Type> = ts.iter().map(type_of).collect();
            if ts.contains(&Type::Error) { Type::Error } else { Type::Tuple(ts) }
        }
    }
}

/// Does the block always end with a `return`?
fn always_returns(b: &Block) -> bool {
    b.stmts.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If { then, otherwise: Some(otherwise), .. } => always_returns(then) && always_returns(otherwise),
        _ => false,
    })
}

impl Checker<'_> {
    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    fn report_unknown(&mut self, t: &TypeAst) {
        match t {
            TypeAst::Name(n) if type_of(t) == Type::Error => self.error(n.span, format!("type inconnu `{}`", n.text)),
            TypeAst::Name(_) => {}
            TypeAst::Tuple(ts) => ts.iter().for_each(|t| self.report_unknown(t)),
        }
    }

    fn written_type(&mut self, t: &TypeAst) -> Type {
        self.report_unknown(t);
        type_of(t)
    }

    fn lookup(&self, name: &str) -> Option<&Variable> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// Declares a variable. Refused if the name is still visible (same block or enclosing block).
    fn declare(&mut self, name: &Name, ty: Type, kind: VarKind) {
        if let Some(old) = self.lookup(&name.text) {
            let (line, _) = line_col(self.src, old.span.start);
            self.diags.push(
                Diag::new(name.span, format!("`{}` existe déjà (ligne {line})", name.text))
                    .help("choisis un autre nom : un nom encore visible ne peut pas être redéclaré"),
            );
            return;
        }
        let v = Variable { ty, kind, span: name.span };
        self.scopes.last_mut().unwrap().insert(name.text.clone(), v);
    }

    /// Declares the names of a pattern: `(a, b)` receiving an `(Int, Float)` → `a: Int`, `b: Float`.
    fn bind(&mut self, p: &Pattern, ty: &Type, kind: VarKind) {
        match (p, ty) {
            (Pattern::Name(n), _) => self.declare(n, ty.clone(), kind),
            (Pattern::Ignore(_), _) => {}
            (Pattern::Tuple(ps, _), Type::Tuple(ts)) if ps.len() == ts.len() => {
                for (p, t) in ps.iter().zip(ts) {
                    self.bind(p, t, kind);
                }
            }
            (Pattern::Tuple(ps, span), _) => {
                if *ty != Type::Error {
                    let msg = format!("ce motif a {} éléments, mais la valeur est un `{}`", ps.len(), ty.text());
                    self.error(*span, msg);
                }
                for p in ps {
                    self.bind(p, &Type::Error, kind);
                }
            }
        }
    }

    /// Checks a block; `before` = variables to declare at its start (a loop counter).
    fn block(&mut self, b: &Block, before: Vec<(&Name, Type, VarKind)>) {
        self.scopes.push(HashMap::new());
        for (name, ty, kind) in before {
            self.declare(name, ty, kind);
        }
        for s in &b.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    /// Checks that `e` has type `expected`.
    fn expect(&mut self, e: &Expr, expected: &Type) {
        let t = self.expr(e);
        if t != *expected && t != Type::Error && *expected != Type::Error {
            let msg = format!("types différents : `{}` attendu, `{}` donné", expected.text(), t.text());
            let mut d = Diag::new(e.span, msg);
            if let (Type::Float, ExprKind::Int(v)) = (expected, &e.kind) {
                d = d.help(format!("pour un `Float`, écris `{v}.0`"));
            } else if matches!((expected, &t), (Type::Int, Type::Float) | (Type::Float, Type::Int)) {
                d = d.help("Kova ne convertit jamais tout seul : utilise `.to_float()` ou `.to_int()`");
            }
            self.diags.push(d);
        }
    }

    /// Checks that `e` has one of the `allowed` types; returns its type.
    fn one_of(&mut self, e: &Expr, allowed: &[Type], what: &str) -> Type {
        let t = self.expr(e);
        if t == Type::Error || allowed.contains(&t) {
            return t;
        }
        let names: Vec<String> = allowed.iter().map(|t| format!("`{}`", t.text())).collect();
        self.error(e.span, format!("{what} s'applique à {}, pas à `{}`", names.join(" ou "), t.text()));
        Type::Error
    }

    /// The type of a `:=` target, checking that it may change.
    /// `seen`: the places already met in the same target (`(a, a) := …` is refused).
    fn target(&mut self, t: &Target, seen: &mut Vec<(String, Vec<usize>)>) -> Type {
        match t {
            Target::Tuple(ts, _) => {
                let types: Vec<Type> = ts.iter().map(|t| self.target(t, seen)).collect();
                if types.contains(&Type::Error) { Type::Error } else { Type::Tuple(types) }
            }
            Target::Place { name, fields, span } => {
                let n = &name.text;
                // `t` and `t.0` in the same target: the same slot would receive two values.
                let overlaps = seen.iter().any(|(m, f)| m == n && (f.starts_with(fields) || fields.starts_with(f)));
                if overlaps {
                    self.error(*span, format!("`{}` reçoit deux valeurs dans la même affectation", &self.src[span.start..span.end]));
                }
                seen.push((n.clone(), fields.clone()));
                let Some(var) = self.lookup(n) else {
                    self.error(name.span, format!("variable inconnue `{n}`"));
                    return Type::Error;
                };
                let (mut ty, kind) = (var.ty.clone(), var.kind);
                match kind {
                    VarKind::Var => {}
                    VarKind::Let => self.diags.push(
                        Diag::new(name.span, format!("`{n}` est déclarée avec let"))
                            .help(format!("déclare-la avec `var {n}` si elle doit changer")),
                    ),
                    VarKind::Param => self.diags.push(
                        Diag::new(name.span, format!("`{n}` est un paramètre : il ne peut pas changer"))
                            .help(format!("copie-le d'abord : `var {n}2: ... := {n}`")),
                    ),
                    VarKind::Counter => {
                        self.error(name.span, format!("`{n}` est un compteur de boucle : il ne peut pas changer"))
                    }
                }
                for &i in fields {
                    ty = self.field(ty, i, *span);
                }
                ty
            }
        }
    }

    /// The type of `t.i` when `t` has type `ty`.
    fn field(&mut self, ty: Type, i: usize, span: Span) -> Type {
        match ty {
            Type::Tuple(ts) if i < ts.len() => ts[i].clone(),
            Type::Tuple(ts) => {
                let msg = format!("ce tuple a {} éléments : de `.0` à `.{}`", ts.len(), ts.len() - 1);
                self.error(span, msg);
                Type::Error
            }
            Type::Error => Type::Error,
            other => {
                self.error(span, format!("`.{i}` s'applique à un tuple, pas à `{}`", other.text()));
                Type::Error
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Declare { mutable, pattern, ty, value } => {
                let ty = match ty {
                    Some(t) => self.written_type(t),
                    None => {
                        let (span, name) = match pattern {
                            Pattern::Name(n) => (n.span, n.text.clone()),
                            Pattern::Tuple(_, s) | Pattern::Ignore(s) => (*s, self.src[s.start..s.end].to_string()),
                        };
                        self.diags.push(
                            Diag::new(span, format!("il faut écrire le type de `{name}`"))
                                .help(format!("par exemple `{name}: Int`")),
                        );
                        Type::Error
                    }
                };
                self.expect(value, &ty);
                self.bind(pattern, &ty, if *mutable { VarKind::Var } else { VarKind::Let });
            }
            Stmt::Assign { target, op, value } => {
                let ty = self.target(target, &mut vec![]);
                if op.is_some() && !matches!(ty, Type::Int | Type::Float | Type::Error) {
                    let text = &self.src[target.span().start..target.span().end];
                    self.error(target.span(), format!("`{text}` n'est ni un `Int` ni un `Float`"));
                }
                self.expect(value, &ty);
            }
            Stmt::Return { value, span } => match (value, self.ret.clone()) {
                (None, Type::Unit | Type::Error) => {}
                (None, t) => self.error(*span, format!("cette fonction doit renvoyer un `{}`", t.text())),
                (Some(e), Type::Unit) => {
                    self.expr(e);
                    self.diags.push(
                        Diag::new(e.span, "cette fonction ne renvoie rien")
                            .help("ajoute `-> Type` après les paramètres pour renvoyer une valeur"),
                    );
                }
                (Some(e), t) => self.expect(e, &t),
            },
            Stmt::Check(k) => self.expect(&k.cond, &Type::Bool),
            Stmt::Break(span) | Stmt::Continue(span) => {
                if self.loops == 0 {
                    let word = if matches!(s, Stmt::Break(_)) { "break" } else { "continue" };
                    self.error(*span, format!("`{word}` en dehors d'une boucle"));
                }
            }
            Stmt::If { cond, then, otherwise } => {
                self.expect(cond, &Type::Bool);
                self.block(then, vec![]);
                if let Some(o) = otherwise {
                    self.block(o, vec![]);
                }
            }
            Stmt::While { cond, body } => {
                self.expect(cond, &Type::Bool);
                self.loops += 1;
                self.block(body, vec![]);
                self.loops -= 1;
            }
            Stmt::Loop { counter, start, end, body, .. } => {
                self.expect(start, &Type::Int);
                self.expect(end, &Type::Int);
                self.loops += 1;
                self.block(body, vec![(counter, Type::Int, VarKind::Counter)]);
                self.loops -= 1;
            }
            Stmt::Expr(e) => {
                self.expr(e);
                if !matches!(e.kind, ExprKind::Call { .. }) {
                    let mut d = Diag::new(e.span, "cette expression ne fait rien");
                    if let ExprKind::Binary(BinOp::Eq, ..) = e.kind {
                        d = d.help("`=` compare ; pour donner une valeur, écris `:=`");
                    }
                    self.diags.push(d);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> Type {
        let t = self.expr_kind(e);
        self.types[e.id] = t.clone();
        t
    }

    fn expr_kind(&mut self, e: &Expr) -> Type {
        match &e.kind {
            ExprKind::Int(v) => {
                if *v > i64::MAX as u64 {
                    self.error(e.span, "nombre trop grand pour un `Int` (maximum 9223372036854775807)");
                }
                Type::Int
            }
            ExprKind::Float(_) => Type::Float,
            ExprKind::Char(_) => Type::Char,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Result => match &self.result {
                Some(Type::Unit) => {
                    self.error(e.span, "`result` n'existe pas : cette fonction ne renvoie rien");
                    Type::Error
                }
                Some(t) => t.clone(),
                None => {
                    self.diags.push(
                        Diag::new(e.span, "`result` ne s'utilise que dans un `ensure`")
                            .help("`ensure result >= 0` vérifie la valeur renvoyée par la fonction"),
                    );
                    Type::Error
                }
            },
            ExprKind::Var(name) => match self.lookup(name) {
                Some(v) => v.ty.clone(),
                None => {
                    self.error(e.span, format!("variable inconnue `{name}`"));
                    Type::Error
                }
            },
            ExprKind::Tuple(elems) => {
                let mut ts = vec![];
                for x in elems {
                    let t = self.expr(x);
                    if t == Type::Unit {
                        self.error(x.span, "cette expression ne donne pas de valeur à mettre dans un tuple");
                    }
                    ts.push(t);
                }
                if ts.iter().any(|t| matches!(t, Type::Error | Type::Unit)) { Type::Error } else { Type::Tuple(ts) }
            }
            ExprKind::Field(x, i) => {
                let t = self.expr(x);
                self.field(t, *i, e.span)
            }
            ExprKind::Call { name, args } => self.call(name, args),
            ExprKind::Method { receiver, name, args } => {
                let t = self.expr(receiver);
                for a in args {
                    self.expr(a);
                }
                if t == Type::Error {
                    return Type::Error;
                }
                let Some((_, _, result)) = METHODS.iter().find(|(r, n, _)| *r == t && *n == name.text) else {
                    let known: Vec<String> =
                        METHODS.iter().filter(|(r, ..)| *r == t).map(|(_, n, _)| format!("`{n}`")).collect();
                    let mut d = Diag::new(name.span, format!("`{}` n'a pas de méthode `{}`", t.text(), name.text));
                    if !known.is_empty() {
                        d = d.help(format!("méthodes de `{}` : {}", t.text(), known.join(", ")));
                    }
                    self.diags.push(d);
                    return Type::Error;
                };
                if !args.is_empty() {
                    self.error(name.span, format!("`{}` ne prend pas d'argument", name.text));
                }
                result.clone()
            }
            ExprKind::Unary(UnOp::Neg, x) => {
                // `-9223372036854775808`: the only case where the number alone would be too big.
                if let ExprKind::Int(v) = x.kind
                    && v == 1 << 63
                {
                    self.types[x.id] = Type::Int;
                    return Type::Int;
                }
                self.one_of(x, &[Type::Int, Type::Float], "`-`")
            }
            ExprKind::Unary(UnOp::Not, x) => {
                self.expect(x, &Type::Bool);
                Type::Bool
            }
            ExprKind::If { cond, then, otherwise } => self.if_value(cond, then, otherwise),
            ExprKind::Binary(op, a, b) => {
                use BinOp::*;
                match op {
                    Or | And => {
                        self.expect(a, &Type::Bool);
                        self.expect(b, &Type::Bool);
                        Type::Bool
                    }
                    Eq | Ne => {
                        let ta = self.expr(a);
                        match ta {
                            Type::Unit => self.error(a.span, "cette expression ne renvoie rien"),
                            Type::Tuple(_) => self.diags.push(
                                Diag::new(e.span, format!("`{}` ne compare pas encore des tuples", op.text()))
                                    .help("compare les éléments un par un : `t.0 = u.0 && t.1 = u.1`"),
                            ),
                            _ => {}
                        }
                        self.expect(b, &ta);
                        Type::Bool
                    }
                    Lt | Le | Gt | Ge => {
                        let t = self.one_of(a, &[Type::Int, Type::Float, Type::Char], &format!("`{}`", op.text()));
                        self.expect(b, &t);
                        Type::Bool
                    }
                    Range | RangeInclusive => {
                        self.expr(a);
                        self.expr(b);
                        self.error(e.span, format!("`{}` ne s'utilise que dans `for`", op.text()));
                        Type::Error
                    }
                    Rem => {
                        self.expect(a, &Type::Int);
                        self.expect(b, &Type::Int);
                        Type::Int
                    }
                    Plus | Minus | Times | Div => {
                        let t = self.one_of(a, &[Type::Int, Type::Float], &format!("`{}`", op.text()));
                        self.expect(b, &t);
                        t
                    }
                }
            }
        }
    }

    fn if_value(&mut self, cond: &Expr, then: &Block, otherwise: &Block) -> Type {
        self.expect(cond, &Type::Bool);
        let (a, b) = (self.block_value(then), self.block_value(otherwise));
        if a != b && a != Type::Error && b != Type::Error {
            let msg = format!("les deux branches doivent donner le même type : `{}` et `{}`", a.text(), b.text());
            self.error(otherwise.open, msg);
            return Type::Error;
        }
        a
    }

    /// The type of a block giving a value: that of its last line.
    fn block_value(&mut self, b: &Block) -> Type {
        self.scopes.push(HashMap::new());
        let (last, before) = match b.stmts.split_last() {
            Some((l, b)) => (Some(l), b),
            None => (None, &[][..]),
        };
        for s in before {
            self.stmt(s);
        }
        let ty = match last {
            Some(Stmt::Expr(e)) => self.expr(e),
            Some(Stmt::If { cond, then, otherwise: Some(otherwise) }) => self.if_value(cond, then, otherwise),
            other => {
                if let Some(s) = other {
                    self.stmt(s);
                }
                self.diags.push(
                    Diag::new(b.close, "cette branche doit finir par une valeur")
                        .help("la dernière ligne d'une branche est la valeur du `if`"),
                );
                Type::Error
            }
        };
        self.scopes.pop();
        ty
    }

    fn call(&mut self, name: &Name, args: &[Expr]) -> Type {
        if BUILTINS.contains(&name.text.as_str()) {
            if args.len() != 1 {
                self.error(name.span, format!("`{}` prend exactement 1 argument", name.text));
            }
            for a in args {
                match self.expr(a) {
                    Type::Unit => {
                        self.error(a.span, format!("`{}` : cette expression ne donne pas de valeur à afficher", name.text))
                    }
                    Type::Tuple(_) => self.diags.push(
                        Diag::new(a.span, format!("`{}` n'affiche pas encore les tuples", name.text))
                            .help("affiche les éléments un par un : `print(t.0)`"),
                    ),
                    _ => {}
                }
            }
            return Type::Unit;
        }
        let Some(sig) = self.functions.get(name.text.as_str()) else {
            self.error(name.span, format!("fonction inconnue `{}`", name.text));
            args.iter().for_each(|a| {
                self.expr(a);
            });
            return Type::Error;
        };
        let (params, ret) = (sig.params.clone(), sig.ret.clone());
        if params.len() != args.len() {
            let msg = format!("`{}` prend {} argument(s), {} donné(s)", name.text, params.len(), args.len());
            self.error(name.span, msg);
        }
        for (i, a) in args.iter().enumerate() {
            match params.get(i) {
                Some(t) => self.expect(a, t),
                None => {
                    self.expr(a);
                }
            }
        }
        ret
    }
}
