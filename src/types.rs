// Type checking: known names, correct types, `let`/`var` rules.
//
// Kova guesses (almost) nothing: it checks that what is written is consistent.
//
//   let x: Int := true    →  error: different types: `Int` expected, `Bool` given
//   a := 2  (a is a let)  →  error: `a` is declared with let
//
// The result: the type of each expression (indexed by `Expr::id`).

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::diag::{Diag, Span, line_col};
use crate::locale::{self, t};

#[derive(Clone, PartialEq, Debug)]
pub enum Type {
    Int,
    Float,
    Bool,
    Char,
    /// UTF-8 text, owned: moved when given away, freed at the end of its block (PLAN 2.7).
    String,
    /// `(Int, Float)`: at least 2 elements.
    Tuple(Vec<Type>),
    /// A `struct`: its slots are those of its fields, in order (like a tuple).
    Struct(Rc<StructDef>),
    /// What a function without `->` returns.
    Unit,
    /// Unknown type because of an error already reported: it is not repeated.
    Error,
}

impl Type {
    /// `rex.name` is a field read (and not a method call).
    pub fn is_field(&self, name: &str) -> bool {
        matches!(self, Type::Struct(d) if d.field(name).is_some())
    }

    /// The type as in the code: `Int`, `(Int, Float)`.
    pub fn text(&self) -> String {
        match self {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::Bool => "Bool".into(),
            Type::Char => "Char".into(),
            Type::String => "String".into(),
            Type::Tuple(ts) => format!("({})", ts.iter().map(Type::text).collect::<Vec<_>>().join(", ")),
            Type::Struct(d) => d.name.clone(),
            Type::Unit => t!("type_nothing"),
            Type::Error => "?".into(),
        }
    }
}

#[derive(PartialEq, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub copy: bool,
}

impl StructDef {
    /// The number and type of a field.
    pub fn field(&self, name: &str) -> Option<(usize, &Type)> {
        self.fields.iter().enumerate().find(|(_, (n, _))| n == name).map(|(i, (_, t))| (i, t))
    }
}

/// The structs of the program, by name.
pub type Structs = HashMap<String, Rc<StructDef>>;

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
    (Type::String, "length", Type::Int),
    (Type::String, "clone", Type::String),
];

/// Copied when given away (like an `Int`), instead of moved (like a `String`).
pub fn is_copy(t: &Type) -> bool {
    match t {
        Type::String => false,
        Type::Tuple(ts) => ts.iter().all(is_copy),
        Type::Struct(d) => d.copy,
        _ => true,
    }
}

pub struct Info {
    /// Type of each expression, by `Expr::id`.
    pub types: Vec<Type>,
    /// By `Expr::id`: true if this variable is moved here (filled by `ownership.rs`).
    pub moves: Vec<bool>,
    pub structs: Structs,
}

#[derive(Clone, Copy, PartialEq)]
enum VarKind {
    Let,
    Var,
    Param,
    Counter,
    /// `self` in a method.
    Receiver(Receiver),
}

struct Variable {
    ty: Type,
    kind: VarKind,
    span: Span,
}

#[derive(Clone)]
struct Signature {
    receiver: Option<Receiver>,
    params: Vec<Type>,
    ret: Type,
}

/// The name of a struct's method or static function among the functions: `Dog.sound`.
pub fn method_name(ty: &str, name: &str) -> String {
    format!("{ty}.{name}")
}

/// The variable at the root of a place: `rex` in `rex.dog.name` or in `t.0`;
/// `None` for a computed value (`make_dog.name`).
pub fn place_root<'e>(e: &'e Expr, types: &[Type]) -> Option<(&'e str, Span)> {
    match &e.kind {
        ExprKind::Var(n) => Some((n, e.span)),
        ExprKind::Field(x, _) => place_root(x, types),
        ExprKind::Method { receiver, name, .. } if types[receiver.id].is_field(&name.text) => place_root(receiver, types),
        _ => None,
    }
}

struct Checker<'a> {
    src: &'a str,
    /// Functions, methods and static functions (`Dog.sound`).
    functions: HashMap<String, Signature>,
    structs: Structs,
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
        structs: Structs::new(),
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

    // 1. The structs, then the signatures: what is written further down can be used.
    let mut decls: HashMap<&str, usize> = HashMap::new();
    for (i, s) in f.structs.iter().enumerate() {
        let name = s.name.text.as_str();
        if primitive(name).is_some() {
            c.error(s.name.span, t!("type_builtin", name = name));
        } else if let Some(&j) = decls.get(name) {
            let (line, _) = line_col(src, f.structs[j].name.span.start);
            c.error(s.name.span, t!("type_exists", name = name, line = line));
        } else {
            decls.insert(name, i);
        }
    }
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) == Some(&i) {
            c.define(f, &decls, i, &mut vec![]);
        }
    }
    for func in &f.functions {
        for t in func.params.iter().map(|p| &p.ty).chain(&func.ret) {
            c.report_unknown(t);
        }
        let params = func.params.iter().map(|p| type_of(&p.ty, &c.structs)).collect();
        let ret = func.ret.as_ref().map_or(Type::Unit, |t| type_of(t, &c.structs));
        let name = func.name.text.as_str();
        if BUILTINS.contains(&name) {
            c.error(func.name.span, t!("builtin_function", name = name));
        } else if c.functions.contains_key(name) {
            c.error(func.name.span, t!("function_exists", name = name));
        } else {
            c.functions.insert(name.to_string(), Signature { receiver: None, params, ret });
        }
    }
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) != Some(&i) {
            continue;
        }
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for m in &s.methods {
            for t in m.params.iter().map(|p| &p.ty).chain(&m.ret) {
                c.report_unknown(t);
            }
            let params = m.params.iter().map(|p| type_of(&p.ty, &c.structs)).collect();
            let ret = m.ret.as_ref().map_or(Type::Unit, |t| type_of(t, &c.structs));
            let name = m.name.text.as_str();
            // Uniform access: `rex.name` must mean a single thing.
            if s.fields.iter().any(|p| p.name.text == name) {
                c.diags.push(
                    Diag::new(m.name.span, t!("field_and_method", name = name, type = s.name.text))
                        .help(t!("field_and_method_help", name = name)),
                );
            } else if let Some(first) = seen.insert(name, m.name.span) {
                let (line, _) = line_col(src, first.start);
                c.error(m.name.span, t!("method_exists", name = name, type = s.name.text, line = line));
            } else {
                c.functions.insert(method_name(&s.name.text, name), Signature { receiver: m.receiver, params, ret });
            }
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
        let others = f.header.iter().map(|h| h.open).chain(f.functions.iter().map(|f| f.name.span));
        let others = others.chain(f.structs.iter().map(|s| s.name.span)).chain(f.tests.iter().map(|t| t.span));
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
        c.body(func, None);
    }
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) == Some(&i) {
            let ty = Type::Struct(c.structs[&s.name.text].clone());
            for m in &s.methods {
                c.body(m, Some(&ty));
            }
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

    let moves = vec![false; c.types.len()];
    (Info { types: c.types, moves, structs: c.structs }, c.diags)
}

fn primitive(name: &str) -> Option<Type> {
    Some(match name {
        "Int" => Type::Int,
        "Float" => Type::Float,
        "Bool" => Type::Bool,
        "Char" => Type::Char,
        "String" => Type::String,
        _ => return None,
    })
}

/// The written type; `Error` if it contains an unknown name.
pub fn type_of(t: &TypeAst, structs: &Structs) -> Type {
    match t {
        TypeAst::Name(n) => primitive(&n.text)
            .or_else(|| structs.get(&n.text).map(|d| Type::Struct(d.clone())))
            .unwrap_or(Type::Error),
        TypeAst::Tuple(ts) => {
            let ts: Vec<Type> = ts.iter().map(|t| type_of(t, structs)).collect();
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

    /// The body of a function; `owner` = the struct of a method.
    fn body(&mut self, func: &Function, owner: Option<&Type>) {
        self.ret = func.ret.as_ref().map_or(Type::Unit, |t| type_of(t, &self.structs));
        let mut params = HashMap::new();
        if let (Some(ty), Some(r)) = (owner, func.receiver) {
            params.insert("self".to_string(), Variable { ty: ty.clone(), kind: VarKind::Receiver(r), span: func.name.span });
        }
        for p in &func.params {
            if params.contains_key(&p.name.text) {
                self.error(p.name.span, t!("param_exists", name = p.name.text));
            }
            self.function_name(&p.name);
            params.insert(p.name.text.clone(), Variable { ty: type_of(&p.ty, &self.structs), kind: VarKind::Param, span: p.name.span });
        }
        self.scopes = vec![params];
        // Contracts only see the parameters (and `result` in an `ensure`).
        for k in &func.contracts {
            self.result = (k.kind == ContractKind::Ensure).then(|| self.ret.clone());
            self.contract(k);
        }
        self.result = None;
        self.block(&func.body, vec![]);
        if self.ret != Type::Unit && !always_returns(&func.body) {
            self.diags.push(
                Diag::new(func.body.close, t!("missing_return", name = func.name.text))
                    .help(t!("missing_return_help", type = self.ret.text())),
            );
        }
    }

    /// Builds the `StructDef` of `f.structs[i]` after those of its fields' structs.
    /// `open`: the structs being built; meeting one again = it would contain itself.
    fn define(&mut self, f: &File, decls: &HashMap<&str, usize>, i: usize, open: &mut Vec<usize>) -> Option<Rc<StructDef>> {
        let s = &f.structs[i];
        if let Some(d) = self.structs.get(&s.name.text) {
            return Some(d.clone());
        }
        if open.contains(&i) {
            return None;
        }
        open.push(i);
        let mut fields: Vec<(String, Type)> = vec![];
        for p in &s.fields {
            let ty = self.field_type(f, decls, &p.ty, open);
            if let Some(k) = s.fields.iter().position(|q| q.name.text == p.name.text)
                && s.fields[k].name.span != p.name.span
            {
                let (line, _) = line_col(self.src, s.fields[k].name.span.start);
                self.error(p.name.span, t!("field_exists", name = p.name.text, line = line));
                continue;
            }
            if s.copy && !is_copy(&ty) && ty != Type::Error {
                let msg = t!("copy_struct_field", type = s.name.text, name = p.name.text, field_type = ty.text());
                self.diags.push(Diag::new(p.name.span, msg).help(t!("copy_struct_field_help", type = s.name.text)));
            }
            fields.push((p.name.text.clone(), ty));
        }
        open.pop();
        let d = Rc::new(StructDef { name: s.name.text.clone(), fields, copy: s.copy });
        self.structs.insert(s.name.text.clone(), d.clone());
        Some(d)
    }

    /// The type of a field, building the structs it uses first (see `define`).
    fn field_type(&mut self, f: &File, decls: &HashMap<&str, usize>, t: &TypeAst, open: &mut Vec<usize>) -> Type {
        match t {
            TypeAst::Tuple(ts) => {
                let ts: Vec<Type> = ts.iter().map(|t| self.field_type(f, decls, t, open)).collect();
                if ts.contains(&Type::Error) { Type::Error } else { Type::Tuple(ts) }
            }
            TypeAst::Name(n) => {
                if let Some(t) = primitive(&n.text) {
                    return t;
                }
                let Some(&j) = decls.get(n.text.as_str()) else {
                    self.error(n.span, t!("type_unknown", name = n.text));
                    return Type::Error;
                };
                match self.define(f, decls, j, open) {
                    Some(d) => Type::Struct(d),
                    None => {
                        self.diags.push(
                            Diag::new(n.span, t!("struct_contains_itself", name = n.text))
                                .help(t!("struct_contains_itself_help", name = n.text)),
                        );
                        Type::Error
                    }
                }
            }
        }
    }

    fn report_unknown(&mut self, t: &TypeAst) {
        match t {
            TypeAst::Name(n) if type_of(t, &self.structs) == Type::Error => self.error(n.span, t!("type_unknown", name = n.text)),
            TypeAst::Name(_) => {}
            TypeAst::Tuple(ts) => ts.iter().for_each(|t| self.report_unknown(t)),
        }
    }

    fn written_type(&mut self, t: &TypeAst) -> Type {
        self.report_unknown(t);
        type_of(t, &self.structs)
    }

    fn lookup(&self, name: &str) -> Option<&Variable> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn contract(&mut self, k: &Contract) {
        self.expect(&k.cond, &Type::Bool);
        if let Some(m) = &k.message {
            self.parts(m);
        }
    }

    /// The `{values}` of a text: anything `print` can show.
    fn parts(&mut self, parts: &[Part]) {
        for p in parts {
            let Part::Code(e) = p else { continue };
            match self.expr(e) {
                Type::Unit => self.error(e.span, t!("interp_no_value")),
                Type::Tuple(_) => self.diags.push(Diag::new(e.span, t!("interp_tuple")).help(t!("interp_tuple_help"))),
                _ => {}
            }
        }
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
    fn target(&mut self, t: &Target, seen: &mut Vec<(String, Vec<String>)>) -> Type {
        match t {
            Target::Tuple(ts, _) => {
                let types: Vec<Type> = ts.iter().map(|t| self.target(t, seen)).collect();
                if types.contains(&Type::Error) { Type::Error } else { Type::Tuple(types) }
            }
            Target::Place { name, fields, span } => {
                let n = &name.text;
                // `t` and `t.0` in the same target: the same slot would receive two values.
                let path: Vec<String> = fields.iter().map(Step::text).collect();
                let overlaps = seen.iter().any(|(m, f)| m == n && (f.starts_with(&path) || path.starts_with(f)));
                if overlaps {
                    self.error(*span, t!("two_values", target = &self.src[span.start..span.end]));
                }
                seen.push((n.clone(), path));
                let Some(var) = self.lookup(n) else {
                    self.error(name.span, t!("variable_unknown", name = n));
                    return Type::Error;
                };
                let (mut ty, kind) = (var.ty.clone(), var.kind);
                self.may_change(n, name.span, kind);
                for step in fields {
                    ty = match step {
                        Step::Index(i) => self.field(ty, *i, *span),
                        Step::Name(f) => self.named_field(ty, f),
                    };
                }
                ty
            }
        }
    }

    /// `n` (a variable of this kind) is assigned or given to a `var self` method.
    fn may_change(&mut self, n: &str, span: Span, kind: VarKind) {
        match kind {
            VarKind::Var | VarKind::Receiver(Receiver::Var) => {}
            VarKind::Let => self.diags.push(
                Diag::new(span, t!("declared_with_let", name = n)).help(t!("declared_with_let_help", name = n)),
            ),
            VarKind::Param => self.diags.push(
                Diag::new(span, t!("param_immutable", name = n)).help(t!("param_immutable_help", name = n)),
            ),
            VarKind::Counter => self.error(span, t!("counter_immutable", name = n)),
            VarKind::Receiver(r) => {
                self.diags.push(Diag::new(span, t!("self_immutable", receiver = r.text())).help(t!("self_immutable_help")))
            }
        }
    }

    /// `rex.rename(...)` with `var self`: `rex` must be a variable that may change.
    fn changes(&mut self, receiver: &Expr, method: &Name) {
        let Some((root, span)) = place_root(receiver, &self.types) else {
            self.error(receiver.span, t!("var_method_temporary", name = method.text));
            return;
        };
        if let Some(kind) = self.lookup(root).map(|v| v.kind) {
            self.may_change(root, span, kind);
        }
    }

    /// The arguments of a call, checked against the parameters' types.
    fn arguments(&mut self, name: &Name, args: &[Expr], params: &[Type]) {
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
    }

    /// `rex.sound`: a method of `rex`'s struct.
    fn method_call(&mut self, receiver: &Expr, d: &StructDef, name: &Name, args: &[Expr], sig: Signature) -> Type {
        match sig.receiver {
            None => self.diags.push(
                Diag::new(name.span, t!("static_on_value", name = name.text, type = d.name))
                    .help(t!("static_on_value_help", name = name.text, type = d.name)),
            ),
            Some(Receiver::Var) => self.changes(receiver, name),
            Some(_) => {}
        }
        self.arguments(name, args, &sig.params);
        sig.ret
    }

    /// `Dog.create("Rex")`: a struct's function without `self`.
    fn static_call(&mut self, ty: &Name, name: &Name, args: &[Expr]) -> Type {
        let found = match self.structs.get(&ty.text).cloned() {
            None => {
                self.error(ty.span, t!("type_unknown", name = ty.text));
                None
            }
            Some(d) => match self.functions.get(&method_name(&d.name, &name.text)).cloned() {
                None => {
                    self.no_field(&d, name);
                    None
                }
                Some(sig) => Some(sig),
            },
        };
        let Some(sig) = found else {
            args.iter().for_each(|a| {
                self.expr(a);
            });
            return Type::Error;
        };
        if sig.receiver.is_some() {
            self.diags.push(
                Diag::new(name.span, t!("method_needs_value", name = name.text, type = ty.text))
                    .help(t!("method_needs_value_help", name = name.text)),
            );
        }
        self.arguments(name, args, &sig.params);
        sig.ret
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

    /// The type of `x.name` when `x` has type `ty` and `name` is not a method.
    fn named_field(&mut self, ty: Type, name: &Name) -> Type {
        match &ty {
            Type::Struct(d) => match d.field(&name.text) {
                Some((_, t)) => t.clone(),
                None => {
                    self.no_field(d, name);
                    Type::Error
                }
            },
            Type::Error => Type::Error,
            other => {
                self.error(name.span, t!("no_method", type = other.text(), name = name.text));
                Type::Error
            }
        }
    }

    /// `rex.color`: no such field or method in `Dog`.
    fn no_field(&mut self, d: &StructDef, name: &Name) {
        let prefix = method_name(&d.name, "");
        let mut methods: Vec<&str> = self.functions.keys().filter_map(|k| k.strip_prefix(&prefix)).collect();
        methods.sort();
        let fields: Vec<String> =
            d.fields.iter().map(|(n, _)| n.as_str()).chain(methods).map(|n| format!("`{n}`")).collect();
        let mut diag = Diag::new(name.span, t!("no_member", type = d.name, name = name.text));
        if !fields.is_empty() {
            diag = diag.help(t!("fields_of", type = d.name, fields = fields.join(", ")));
        }
        self.diags.push(diag);
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            // `let rex := Dog { ... }`: the type is written on the right (PLAN 2.5).
            Stmt::Declare { mutable, pattern, ty: None, value } if matches!(value.kind, ExprKind::StructLit { .. }) => {
                let ty = self.expr(value);
                self.bind(pattern, &ty, if *mutable { VarKind::Var } else { VarKind::Let });
            }
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
                let concat = *op == Some(BinOp::Plus) && ty == Type::String;
                if op.is_some() && !concat && !matches!(ty, Type::Int | Type::Float | Type::Error) {
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
            Stmt::Check(k) => self.contract(k),
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
                let call = match &e.kind {
                    ExprKind::Call { .. } | ExprKind::Static { .. } => true,
                    ExprKind::Method { receiver, name, .. } => matches!(&self.types[receiver.id],
                        Type::Struct(d) if self.functions.contains_key(&method_name(&d.name, &name.text))),
                    _ => false,
                };
                if !call {
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
                None if name == "self" => {
                    self.error(e.span, t!("self_outside_method"));
                    Type::Error
                }
                None => {
                    self.error(e.span, t!("variable_unknown", name = name));
                    Type::Error
                }
            },
            ExprKind::InMode { .. } => Type::Bool,
            ExprKind::Str(_) => Type::String,
            ExprKind::Interp(parts) => {
                self.parts(parts);
                Type::String
            }
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
            ExprKind::StructLit { ty, fields } => self.struct_literal(ty, fields),
            ExprKind::Call { name, args } => self.call(name, args),
            ExprKind::Static { ty, name, args } => self.static_call(ty, name, args),
            ExprKind::Method { receiver, name, args } => {
                let t = self.expr(receiver);
                if let Type::Struct(d) = &t
                    && let Some(sig) = self.functions.get(&method_name(&d.name, &name.text)).cloned()
                {
                    return self.method_call(receiver, d, name, args, sig);
                }
                for a in args {
                    self.expr(a);
                }
                if t == Type::Error {
                    return Type::Error;
                }
                // `.clone` on any value made of several slots: a deep copy (its Strings are copied too).
                if name.text == "clone" && matches!(t, Type::Struct(_) | Type::Tuple(_)) && !t.is_field("clone") {
                    if !args.is_empty() {
                        self.error(name.span, t!("no_arguments", name = name.text));
                    }
                    return t;
                }
                if let Type::Struct(d) = &t {
                    let Some((_, ft)) = d.field(&name.text) else {
                        self.no_field(d, name);
                        return Type::Error;
                    };
                    if !args.is_empty() {
                        self.error(name.span, t!("no_arguments", name = name.text));
                    }
                    return ft.clone();
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
                            Type::Struct(_) => self.diags.push(
                                Diag::new(e.span, t!("struct_compare", op = op.text(), type = ta.text()))
                                    .help(t!("struct_compare_help")),
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
                    Plus => self.same_type(a, b, &[Type::Int, Type::Float, Type::String], "`+`"),
                    Minus | Times | Div => {
                        self.same_type(a, b, &[Type::Int, Type::Float], &format!("`{}`", op.text()))
                    }
                }
            }
        }
    }

    /// `Dog { name := "Rex", age := 3 }`: every field exactly once, in any order.
    fn struct_literal(&mut self, ty: &Name, fields: &[(Name, Expr)]) -> Type {
        let Some(d) = self.structs.get(&ty.text).cloned() else {
            self.error(ty.span, t!("type_unknown", name = ty.text));
            fields.iter().for_each(|(_, v)| {
                self.expr(v);
            });
            return Type::Error;
        };
        let mut given: Vec<&str> = vec![];
        for (name, value) in fields {
            let Some((_, t)) = d.field(&name.text) else {
                self.no_field(&d, name);
                self.expr(value);
                continue;
            };
            if given.contains(&name.text.as_str()) {
                self.error(name.span, t!("field_twice", name = name.text));
            }
            given.push(&name.text);
            self.expect(value, t);
        }
        let missing: Vec<String> =
            d.fields.iter().filter(|(n, _)| !given.contains(&n.as_str())).map(|(n, _)| format!("`{n}`")).collect();
        if !missing.is_empty() {
            self.error(ty.span, t!("fields_missing", type = d.name, fields = missing.join(", ")));
        }
        Type::Struct(d)
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
        self.arguments(name, args, &params);
        ret
    }
}
