// Type checking: known names, correct types, `let`/`var` rules.
//
// Kova guesses (almost) nothing: it checks that what is written is consistent.
//
//   let x: Int := true    →  error: different types: `Int` expected, `Bool` given
//   a := 2  (a is a let)  →  error: `a` is declared with let
//
// The result: the type of each expression (indexed by `Expr::id`).

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diag, Span, line_col};
use crate::locale::{self, t};

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
            Type::Unit => t!("type_nothing"),
            Type::Error => "?".into(),
        }
    }
}

/// The `header` keys, their allowed values (the first one is the default) and the key of their help.
pub const SETTINGS: &[(&str, &[&str], &str)] = &[
    ("mode", &["run", "build"], "setting_mode"),
    ("lock", &["false", "true"], "setting_lock"),
    // `auto`, then the codes of `locale::LANGUAGES` (checked by a test).
    ("lang", &["auto", "en", "fr"], "setting_lang"),
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
                Diag::new(key.span, t!("setting_unknown", key = key.text))
                    .help(t!("setting_unknown_help", keys = keys.join(", "))),
            );
            continue;
        };
        if let Some(first) = seen.insert(&key.text, key.span) {
            let (line, _) = line_col(src, first.start);
            c.error(key.span, t!("setting_twice", key = key.text, line = line));
        }
        if !allowed.contains(&value.text.as_str()) {
            let allowed: Vec<String> = allowed.iter().map(|a| format!("`{a}`")).collect();
            let allowed = allowed.join(&format!(" {} ", t!("word_or")));
            c.error(value.span, t!("setting_bad_value", key = key.text, allowed = allowed, value = value.text));
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
            c.error(func.name.span, t!("builtin_function", name = name));
        } else if c.functions.contains_key(name) {
            c.error(func.name.span, t!("function_exists", name = name));
        } else {
            c.functions.insert(name, Signature { params, ret });
        }
    }

    // 2. `fn main()`: required, without parameters or result; not in a translation file.
    match f.functions.iter().find(|f| f.name.text == "main") {
        None if !f.langs.is_empty() => {}
        None => c.error(Span::new(0, 0), t!("main_missing")),
        Some(m) if !m.params.is_empty() || m.ret.is_some() => c.error(m.name.span, t!("main_signature")),
        _ => {}
    }

    // A translation file contains only `lang` blocks.
    if !f.langs.is_empty() {
        let others = f.header.iter().map(|h| h.open).chain(f.functions.iter().map(|f| f.name.span)).chain(f.tests.iter().map(|t| t.span));
        for span in others {
            c.error(span, t!("lang_file_only"));
        }
    }
    for l in &f.langs {
        let mut keys: HashMap<&str, Span> = HashMap::new();
        for (key, text) in &l.entries {
            if let Some(first) = keys.insert(&key.text, key.span) {
                let (line, _) = line_col(src, first.start);
                c.error(key.span, t!("key_twice", key = key.text, line = line));
            }
            if let Err((k, msg)) = locale::segments(&src[text.start..text.end]) {
                c.error(Span::new(text.start + k, text.start + k + 1), msg);
            }
        }
    }

    // 3. The bodies.
    for func in &f.functions {
        c.ret = func.ret.as_ref().map_or(Type::Unit, type_of);
        let mut params = HashMap::new();
        for p in &func.params {
            if params.contains_key(&p.name.text) {
                c.error(p.name.span, t!("param_exists", name = p.name.text));
            }
            c.function_name(&p.name);
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
                Diag::new(func.body.close, t!("missing_return", name = func.name.text))
                    .help(t!("missing_return_help", type = c.ret.text())),
            );
        }
    }

    // 4. Tests: checked even by `kova run`, so that errors show up right away.
    let mut names: HashMap<&str, Span> = HashMap::new();
    for t in &f.tests {
        if let Some(first) = names.insert(&t.name, t.span) {
            let (line, _) = line_col(src, first.start);
            c.error(t.span, t!("test_exists", name = t.name, line = line));
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
            TypeAst::Name(n) if type_of(t) == Type::Error => self.error(n.span, t!("type_unknown", name = n.text)),
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

    /// A bare `main` calls `fn main`: no variable may take the name of a function without parameters.
    fn function_name(&mut self, name: &Name) -> bool {
        let taken = self.functions.get(name.text.as_str()).is_some_and(|f| f.params.is_empty());
        if taken {
            self.diags.push(
                Diag::new(name.span, t!("function_name_taken", name = name.text)).help(t!("function_name_taken_help", name = name.text)),
            );
        }
        taken
    }

    /// Declares a variable. Refused if the name is still visible (same block or enclosing block).
    fn declare(&mut self, name: &Name, ty: Type, kind: VarKind) {
        if self.function_name(name) {
            return;
        }
        if let Some(old) = self.lookup(&name.text) {
            let (line, _) = line_col(self.src, old.span.start);
            self.diags.push(
                Diag::new(name.span, t!("already_exists", name = name.text, line = line)).help(t!("already_exists_help")),
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
                    let msg = t!("pattern_size", count = ps.len(), type = ty.text());
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
        if *expected == Type::Float && t == Type::Int && self.as_float(e) {
            return;
        }
        self.mismatch(e, t, expected);
    }

    /// A number written directly, like `3` or `-2`, becomes a `Float` if it fits exactly
    /// (PLAN 2.5: `let f: Float := 3`, `1.5 + 2`). A variable still needs `.to_float`.
    fn as_float(&mut self, e: &Expr) -> bool {
        let (v, inner) = match &e.kind {
            ExprKind::Int(v) => (*v, None),
            ExprKind::Unary(UnOp::Neg, x) => match x.kind {
                ExprKind::Int(v) => (v, Some(x.id)),
                _ => return false,
            },
            _ => return false,
        };
        if v as f64 as u64 != v || v as f64 >= 18446744073709551615.0 {
            return false;
        }
        self.types[e.id] = Type::Float;
        if let Some(id) = inner {
            self.types[id] = Type::Float;
        }
        true
    }

    /// `a op b`, both of the same type; `2 * x` with a `Float` `x` makes `2` a `Float`.
    fn same_type(&mut self, a: &Expr, b: &Expr, allowed: &[Type], what: &str) -> Type {
        let t = self.one_of(a, allowed, what);
        self.like(a, t, b)
    }

    /// Checks `b` against the type `t` of `a` (see `same_type`).
    fn like(&mut self, a: &Expr, t: Type, b: &Expr) -> Type {
        if t != Type::Int || !matches!(a.kind, ExprKind::Int(_) | ExprKind::Unary(UnOp::Neg, _)) {
            self.expect(b, &t);
            return t;
        }
        let tb = self.expr(b);
        if tb == Type::Float && self.as_float(a) {
            return Type::Float;
        }
        self.mismatch(b, tb, &t);
        t
    }

    fn mismatch(&mut self, e: &Expr, t: Type, expected: &Type) {
        if t != *expected && t != Type::Error && *expected != Type::Error {
            let msg = t!("types_differ", expected = expected.text(), given = t.text());
            let mut d = Diag::new(e.span, msg);
            if let (Type::Float, ExprKind::Int(v)) = (expected, &e.kind) {
                d = d.help(t!("float_inexact", value = v, nearest = crate::kir::float_text(*v as f64)));
            } else if matches!((expected, &t), (Type::Int, Type::Float) | (Type::Float, Type::Int)) {
                d = d.help(t!("no_conversion"));
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
        let names = names.join(&format!(" {} ", t!("word_or")));
        self.error(e.span, t!("applies_to", what = what, allowed = names, given = t.text()));
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
                    self.error(*span, t!("two_values", target = &self.src[span.start..span.end]));
                }
                seen.push((n.clone(), fields.clone()));
                let Some(var) = self.lookup(n) else {
                    self.error(name.span, t!("variable_unknown", name = n));
                    return Type::Error;
                };
                let (mut ty, kind) = (var.ty.clone(), var.kind);
                match kind {
                    VarKind::Var => {}
                    VarKind::Let => self.diags.push(
                        Diag::new(name.span, t!("declared_with_let", name = n)).help(t!("declared_with_let_help", name = n)),
                    ),
                    VarKind::Param => self.diags.push(
                        Diag::new(name.span, t!("param_immutable", name = n)).help(t!("param_immutable_help", name = n)),
                    ),
                    VarKind::Counter => {
                        self.error(name.span, t!("counter_immutable", name = n))
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
                let msg = t!("tuple_field_range", count = ts.len(), last = ts.len() - 1);
                self.error(span, msg);
                Type::Error
            }
            Type::Error => Type::Error,
            other => {
                self.error(span, t!("field_on_non_tuple", index = i, type = other.text()));
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
                            Diag::new(span, t!("type_required", name = name)).help(t!("type_required_help", name = name)),
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
                    self.error(target.span(), t!("not_numeric", target = text));
                }
                self.expect(value, &ty);
            }
            Stmt::Return { value, span } => match (value, self.ret.clone()) {
                (None, Type::Unit | Type::Error) => {}
                (None, t) => self.error(*span, t!("must_return", type = t.text())),
                (Some(e), Type::Unit) => {
                    self.expr(e);
                    self.diags.push(
                        Diag::new(e.span, t!("returns_nothing")).help(t!("returns_nothing_help")),
                    );
                }
                (Some(e), t) => self.expect(e, &t),
            },
            Stmt::Check(k) => self.expect(&k.cond, &Type::Bool),
            Stmt::Break(span) | Stmt::Continue(span) => {
                if self.loops == 0 {
                    let word = if matches!(s, Stmt::Break(_)) { "break" } else { "continue" };
                    self.error(*span, t!("outside_loop", word = word));
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
                    let mut d = Diag::new(e.span, t!("does_nothing"));
                    if let ExprKind::Binary(BinOp::Eq, ..) = e.kind {
                        d = d.help(t!("eq_vs_assign"));
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
                    self.error(e.span, t!("int_too_big"));
                }
                Type::Int
            }
            ExprKind::Float(_) => Type::Float,
            ExprKind::Char(_) => Type::Char,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Result => match &self.result {
                Some(Type::Unit) => {
                    self.error(e.span, t!("result_in_unit"));
                    Type::Error
                }
                Some(t) => t.clone(),
                None => {
                    self.diags.push(
                        Diag::new(e.span, t!("result_outside_ensure")).help(t!("result_help")),
                    );
                    Type::Error
                }
            },
            ExprKind::Var(name) => match self.lookup(name) {
                Some(v) => v.ty.clone(),
                None => {
                    self.error(e.span, t!("variable_unknown", name = name));
                    Type::Error
                }
            },
            ExprKind::InMode { .. } => Type::Bool,
            ExprKind::Tuple(elems) => {
                let mut ts = vec![];
                for x in elems {
                    let t = self.expr(x);
                    if t == Type::Unit {
                        self.error(x.span, t!("tuple_no_value"));
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
                    let mut d = Diag::new(name.span, t!("no_method", type = t.text(), name = name.text));
                    if !known.is_empty() {
                        d = d.help(t!("methods_of", type = t.text(), methods = known.join(", ")));
                    }
                    self.diags.push(d);
                    return Type::Error;
                };
                if !args.is_empty() {
                    self.error(name.span, t!("no_arguments", name = name.text));
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
                            Type::Unit => self.error(a.span, t!("expression_nothing")),
                            Type::Tuple(_) => self.diags.push(
                                Diag::new(e.span, t!("tuple_compare", op = op.text())).help(t!("tuple_compare_help")),
                            ),
                            _ => {}
                        }
                        self.like(a, ta, b);
                        Type::Bool
                    }
                    Lt | Le | Gt | Ge => {
                        self.same_type(a, b, &[Type::Int, Type::Float, Type::Char], &format!("`{}`", op.text()));
                        Type::Bool
                    }
                    Range | RangeInclusive => {
                        self.expr(a);
                        self.expr(b);
                        self.error(e.span, t!("range_only_in_for", op = op.text()));
                        Type::Error
                    }
                    Rem => {
                        self.expect(a, &Type::Int);
                        self.expect(b, &Type::Int);
                        Type::Int
                    }
                    Plus | Minus | Times | Div => {
                        self.same_type(a, b, &[Type::Int, Type::Float], &format!("`{}`", op.text()))
                    }
                }
            }
        }
    }

    fn if_value(&mut self, cond: &Expr, then: &Block, otherwise: &Block) -> Type {
        self.expect(cond, &Type::Bool);
        let (a, b) = (self.block_value(then), self.block_value(otherwise));
        if a != b && a != Type::Error && b != Type::Error {
            let msg = t!("branches_differ", a = a.text(), b = b.text());
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
                    Diag::new(b.close, t!("branch_needs_value")).help(t!("branch_value_help")),
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
                self.error(name.span, t!("builtin_one_arg", name = name.text));
            }
            for a in args {
                match self.expr(a) {
                    Type::Unit => {
                        self.error(a.span, t!("print_no_value", name = name.text))
                    }
                    Type::Tuple(_) => self.diags.push(
                        Diag::new(a.span, t!("print_tuple", name = name.text)).help(t!("print_tuple_help")),
                    ),
                    _ => {}
                }
            }
            return Type::Unit;
        }
        let Some(sig) = self.functions.get(name.text.as_str()) else {
            self.error(name.span, t!("function_unknown", name = name.text));
            args.iter().for_each(|a| {
                self.expr(a);
            });
            return Type::Error;
        };
        let (params, ret) = (sig.params.clone(), sig.ret.clone());
        if params.len() != args.len() {
            let msg = t!("argument_count", name = name.text, expected = params.len(), given = args.len());
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
