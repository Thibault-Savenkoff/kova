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
    /// An `enum`: one slot for the variant's number, then the fields of every variant.
    Enum(Rc<EnumDef>),
    /// `Int?`: an `Int` or `none`. Laid out like an enum with 2 variants, `none` and `some`.
    Optional(Box<Type>),
    /// `String or IoError`: like an enum with 2 variants, `err(error)` and `ok(value)`.
    Result(Box<Type>, Box<Type>),
    /// `List<Int>`: one slot, the address of a block that can grow; owned like a `String`.
    List(Box<Type>),
    /// A type known only by what it promises: `Self` in an interface. Replaced by the real type
    /// (`Dog`) in the KIR, one copy of the function per real type (`substitute`).
    Param(Rc<ParamDef>),
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
            Type::Enum(d) => d.name.clone(),
            Type::Optional(t) => format!("{}?", t.text()),
            Type::Result(t, e) => format!("{} or {}", t.text(), e.text()),
            Type::List(t) => format!("List<{}>", t.text()),
            Type::Param(p) => p.name.clone(),
            Type::Unit => t!("type_nothing"),
            Type::Error => "?".into(),
        }
    }
}

/// `Self` in `interface Animal`: its name and the interfaces it follows (`Animal`).
#[derive(PartialEq, Debug)]
pub struct ParamDef {
    pub name: String,
    pub bounds: Vec<String>,
}

/// `t` with each `Type::Param` named in `s` replaced by its type: `List<Self>` → `List<Dog>`.
pub fn substitute(t: &Type, s: &[(String, Type)]) -> Type {
    let sub = |t: &Type| Box::new(substitute(t, s));
    match t {
        Type::Param(p) => s.iter().find(|(n, _)| *n == p.name).map_or_else(|| t.clone(), |(_, t)| t.clone()),
        Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| substitute(t, s)).collect()),
        Type::Optional(t) => Type::Optional(sub(t)),
        Type::Result(t, e) => Type::Result(sub(t), sub(e)),
        Type::List(t) => Type::List(sub(t)),
        _ => t.clone(),
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

#[derive(PartialEq, Debug)]
pub struct EnumDef {
    pub name: String,
    /// Each variant: its name and its fields.
    pub variants: Vec<(String, Vec<(String, Type)>)>,
}

impl EnumDef {
    /// The number and fields of a variant.
    pub fn variant(&self, name: &str) -> Option<(usize, &[(String, Type)])> {
        self.variants.iter().enumerate().find(|(_, (n, _))| n == name).map(|(i, (_, f))| (i, &f[..]))
    }

    /// `Circle`, `Square`: for messages.
    fn names(&self) -> String {
        self.variants.iter().map(|(n, _)| format!("`{n}`")).collect::<Vec<_>>().join(", ")
    }
}

/// An enum or an optional seen as variants (an optional = `none`, then `some(value)`):
/// what `match`, the layout and the display share.
pub fn enum_view(t: &Type) -> Option<Rc<EnumDef>> {
    match t {
        Type::Enum(d) => Some(d.clone()),
        Type::Optional(inner) => Some(Rc::new(EnumDef {
            name: t.text(),
            variants: vec![("none".into(), vec![]), ("some".into(), vec![("value".into(), (**inner).clone())])],
        })),
        // `err` first: as for an optional, slot 0 is 1 when there is a value.
        Type::Result(ok, err) => Some(Rc::new(EnumDef {
            name: t.text(),
            variants: vec![("err".into(), vec![("error".into(), (**err).clone())]), ("ok".into(), vec![("value".into(), (**ok).clone())])],
        })),
        _ => None,
    }
}

/// The variables an `if` condition proves present: `x != none`, on each side of `&&`.
/// `then` false: for the `else` branch, `x = none`.
pub fn narrowed(cond: &Expr, then: bool) -> Vec<&str> {
    match &cond.kind {
        ExprKind::Binary(op @ (BinOp::Ne | BinOp::Eq), a, b) if (*op == BinOp::Ne) == then => match (&a.kind, &b.kind) {
            (ExprKind::Var(n), ExprKind::None) | (ExprKind::None, ExprKind::Var(n)) => vec![n.as_str()],
            _ => vec![],
        },
        ExprKind::Binary(BinOp::And, a, b) if then => [narrowed(a, true), narrowed(b, true)].concat(),
        _ => vec![],
    }
}

/// The structs and enums of the program, by name: a `Type::Struct` or a `Type::Enum`.
pub type Structs = HashMap<String, Type>;

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

/// The library's functions (PLAN 2.10), with an ordinary signature: (name, its text for the editor).
pub const LIBRARY: &[(&str, &str)] =
    &[("read_file", "fn read_file(path: String) -> String or IoError"), ("read_line", "fn read_line -> String?")];

/// `IoError`, the failures of `read_file`. Its variants are numbered like the failures of
/// `kova_read_file` (runtime/runtime.c) and of the interpreter: number = variant + 1.
pub fn io_error() -> Type {
    let path = || vec![("path".to_string(), Type::String)];
    let names = ["NotFound", "Denied", "IsDirectory", "NotText", "Other"];
    Type::Enum(Rc::new(EnumDef { name: "IoError".into(), variants: names.iter().map(|n| (n.to_string(), path())).collect() }))
}

/// Built-in methods: (receiver type, name, result, true if the result is optional: `Int?`).
pub const METHODS: &[(Type, &str, Type, bool)] = &[
    (Type::Int, "to_float", Type::Float, false),
    (Type::Int, "to_char", Type::Char, true),
    (Type::Float, "to_int", Type::Int, false),
    (Type::Char, "to_int", Type::Int, false),
    (Type::String, "length", Type::Int, false),
    (Type::String, "clone", Type::String, false),
    (Type::String, "to_int", Type::Int, true),
];

/// Copied when given away (like an `Int`), instead of moved (like a `String`).
pub fn is_copy(t: &Type) -> bool {
    match t {
        Type::String | Type::List(_) => false,
        Type::Tuple(ts) => ts.iter().all(is_copy),
        Type::Struct(d) => d.copy,
        // An enum is a value like an `Int` when nothing in it is owned: `Light.Red`, `Shape.Circle(2.0)`.
        Type::Enum(d) => d.variants.iter().all(|(_, fs)| fs.iter().all(|(_, t)| is_copy(t))),
        Type::Optional(t) => is_copy(t),
        Type::Result(t, e) => is_copy(t) && is_copy(e),
        // Checked once for every real type: as if it were owned.
        Type::Param(_) => false,
        Type::Int | Type::Float | Type::Bool | Type::Char | Type::Unit | Type::Error => true,
    }
}

pub struct Info {
    /// Type of each expression, by `Expr::id`.
    pub types: Vec<Type>,
    /// By `Expr::id`: true if this variable is moved here (filled by `ownership.rs`).
    pub moves: Vec<bool>,
    /// By `Expr::id`: true if this `T` value goes where a `T?` is expected (`let x: Int? := 5`).
    pub wraps: Vec<bool>,
    pub structs: Structs,
    /// The default methods a type takes from its interfaces: (type, interface number in
    /// `File::interfaces`, method number), for `Dog.introduce`.
    pub defaults: Vec<(String, usize, usize)>,
}

impl Info {
    /// The type of the value `e` gives where it goes: a `T?` if its `T` is wrapped.
    pub fn given(&self, e: &Expr) -> Type {
        let t = self.types[e.id].clone();
        if self.wraps[e.id] { Type::Optional(Box::new(t)) } else { t }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum VarKind {
    Let,
    Var,
    Param,
    Counter,
    /// `x` in `for x in l`: borrows an element.
    Element,
    /// `r` in `Circle(r) =>`: a field of the value matched.
    Matched,
    /// `x` inside `if x != none { … }`: the value of the optional `x`, borrowed.
    Narrowed,
    /// `self` in a method.
    Receiver(Mode),
}

struct Variable {
    ty: Type,
    kind: VarKind,
    span: Span,
}

#[derive(Clone, PartialEq)]
struct Signature {
    receiver: Option<Mode>,
    params: Vec<(Mode, Type)>,
    ret: Type,
}

impl Signature {
    fn substitute(&self, s: &[(String, Type)]) -> Signature {
        let params = self.params.iter().map(|(m, t)| (*m, substitute(t, s))).collect();
        Signature { receiver: self.receiver, params, ret: substitute(&self.ret, s) }
    }
}

/// A function's heading as written: `fn less(self, other: Self) -> Bool`.
pub fn heading_text(f: &Function) -> String {
    let mode = |m: Mode| match m {
        Mode::Read => "",
        Mode::Var => "var ",
        Mode::Move => "move ",
    };
    let mut params: Vec<String> = f.receiver.iter().map(|&m| format!("{}self", mode(m))).collect();
    params.extend(f.params.iter().map(|p| format!("{}: {}{}", p.name.text, mode(p.mode), p.ty.text())));
    let params = if params.is_empty() { String::new() } else { format!("({})", params.join(", ")) };
    let ret = f.ret.as_ref().map_or(String::new(), |t| format!(" -> {}", t.text()));
    format!("fn {}{params}{ret}", f.name.text)
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
        ExprKind::Field(x, _) | ExprKind::Index(x, _) => place_root(x, types),
        ExprKind::Method { receiver, name, .. } if types[receiver.id].is_field(&name.text) => place_root(receiver, types),
        _ => None,
    }
}

struct Checker<'a> {
    src: &'a str,
    /// Functions, methods and static functions (`Dog.sound`).
    functions: HashMap<String, Signature>,
    structs: Structs,
    /// Interface name → its number in `File::interfaces`; its methods are in `functions`
    /// (`Animal.sound`), with `Self` as a `Type::Param`.
    interfaces: HashMap<String, usize>,
    defaults: Vec<(String, usize, usize)>,
    /// One table per open block, from outside to inside.
    scopes: Vec<HashMap<String, Variable>>,
    ret: Type,
    /// The type of `result`: `Some` only inside an `ensure`.
    result: Option<Type>,
    loops: usize,
    /// In a contract: `x?` cannot leave the function from there.
    in_contract: bool,
    types: Vec<Type>,
    wraps: Vec<bool>,
    diags: Vec<Diag>,
}

pub fn check(f: &File, nb_exprs: usize, src: &str) -> (Info, Vec<Diag>) {
    let mut c = Checker {
        src,
        functions: HashMap::new(),
        structs: Structs::new(),
        interfaces: HashMap::new(),
        defaults: vec![],
        scopes: vec![],
        ret: Type::Unit,
        result: None,
        loops: 0,
        in_contract: false,
        types: vec![Type::Error; nb_exprs],
        wraps: vec![false; nb_exprs],
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
    c.structs.insert("IoError".into(), io_error());
    let string = || Type::String;
    let read_file = Signature { receiver: None, params: vec![(Mode::Read, string())], ret: Type::Result(Box::new(string()), Box::new(io_error())) };
    c.functions.insert("read_file".into(), read_file);
    c.functions.insert("read_line".into(), Signature { receiver: None, params: vec![], ret: Type::Optional(Box::new(string())) });
    let mut decls: HashMap<&str, usize> = HashMap::new();
    for (i, s) in f.structs.iter().enumerate() {
        let name = s.name.text.as_str();
        if primitive(name).is_some() || c.structs.contains_key(name) || name == "Self" {
            c.error(s.name.span, t!("type_builtin", name = name));
        } else if let Some(&j) = decls.get(name) {
            let (line, _) = line_col(src, f.structs[j].name.span.start);
            c.error(s.name.span, t!("type_exists", name = name, line = line));
        } else {
            decls.insert(name, i);
        }
    }
    for (i, it) in f.interfaces.iter().enumerate() {
        let name = it.name.text.as_str();
        if primitive(name).is_some() || c.structs.contains_key(name) || name == "Self" {
            c.error(it.name.span, t!("type_builtin", name = name));
        } else if let Some(span) = decls.get(name).map(|&j| f.structs[j].name.span).or(c.interfaces.get(name).map(|&j| f.interfaces[j].name.span)) {
            let (line, _) = line_col(src, span.start);
            c.error(it.name.span, t!("type_exists", name = name, line = line));
        } else {
            c.interfaces.insert(name.to_string(), i);
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
        let params = func.params.iter().map(|p| (p.mode, type_of(&p.ty, &c.structs))).collect();
        let ret = func.ret.as_ref().map_or(Type::Unit, |t| type_of(t, &c.structs));
        let name = func.name.text.as_str();
        if BUILTINS.contains(&name) || LIBRARY.iter().any(|(n, _)| *n == name) {
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
        let me = c.structs[&s.name.text].clone();
        c.structs.insert("Self".into(), me);
        for m in &s.methods {
            for t in m.params.iter().map(|p| &p.ty).chain(&m.ret) {
                c.report_unknown(t);
            }
            let params = m.params.iter().map(|p| (p.mode, type_of(&p.ty, &c.structs))).collect();
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
        c.structs.remove("Self");
    }
    // The interfaces' methods, `Animal.sound`, with `Self` standing for the type that follows it.
    for (i, it) in f.interfaces.iter().enumerate() {
        if c.interfaces.get(it.name.text.as_str()) != Some(&i) {
            continue;
        }
        c.structs.insert("Self".into(), self_param(&it.name.text));
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for (m, _) in &it.methods {
            for t in m.params.iter().map(|p| &p.ty).chain(&m.ret) {
                c.report_unknown(t);
            }
            let name = m.name.text.as_str();
            if m.receiver.is_none() {
                c.diags.push(Diag::new(m.name.span, t!("interface_static", name = name)).help(t!("interface_static_help", name = name)));
            }
            if !m.contracts.is_empty() {
                c.error(m.name.span, t!("interface_contract", name = name));
            }
            if let Some(first) = seen.insert(name, m.name.span) {
                let (line, _) = line_col(src, first.start);
                c.error(m.name.span, t!("method_exists", name = name, type = it.name.text, line = line));
                continue;
            }
            let params = m.params.iter().map(|p| (p.mode, type_of(&p.ty, &c.structs))).collect();
            let ret = m.ret.as_ref().map_or(Type::Unit, |t| type_of(t, &c.structs));
            c.functions.insert(method_name(&it.name.text, name), Signature { receiver: m.receiver, params, ret });
        }
        c.structs.remove("Self");
    }
    // `struct Dog : Animal`: Dog writes each method Animal asks for, or takes its default one.
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) == Some(&i) {
            c.conforms(f, s);
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
            let ty = c.structs[&s.name.text].clone();
            c.structs.insert("Self".into(), ty.clone());
            for m in &s.methods {
                c.body(m, Some(&ty));
            }
            c.structs.remove("Self");
        }
    }
    // A default method is checked once, for every type that will take it: `self` is a `Self`.
    for (i, it) in f.interfaces.iter().enumerate() {
        if c.interfaces.get(it.name.text.as_str()) == Some(&i) {
            let me = self_param(&it.name.text);
            c.structs.insert("Self".into(), me.clone());
            for (m, _) in it.methods.iter().filter(|(_, body)| *body) {
                c.body(m, Some(&me));
            }
            c.structs.remove("Self");
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
    (Info { types: c.types, moves, wraps: c.wraps, structs: c.structs, defaults: c.defaults }, c.diags)
}

/// `Self` inside `interface Animal`.
fn self_param(interface: &str) -> Type {
    Type::Param(Rc::new(ParamDef { name: "Self".into(), bounds: vec![interface.to_string()] }))
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
            .or_else(|| structs.get(&n.text).cloned())
            .unwrap_or(Type::Error),
        TypeAst::Tuple(ts) => {
            let ts: Vec<Type> = ts.iter().map(|t| type_of(t, structs)).collect();
            if ts.contains(&Type::Error) { Type::Error } else { Type::Tuple(ts) }
        }
        TypeAst::Generic(n, ts) if n.text == "List" && ts.len() == 1 => match type_of(&ts[0], structs) {
            Type::Error => Type::Error,
            t => Type::List(Box::new(t)),
        },
        TypeAst::Generic(..) => Type::Error,
        TypeAst::Optional(t) => match type_of(t, structs) {
            Type::Error => Type::Error,
            t => Type::Optional(Box::new(t)),
        },
        TypeAst::Result(t, e) => match (type_of(t, structs), type_of(e, structs)) {
            (Type::Error, _) | (_, Type::Error) => Type::Error,
            (t, e) => Type::Result(Box::new(t), Box::new(e)),
        },
    }
}

/// Does the block always end with a `return`?
fn always_returns(b: &Block) -> bool {
    b.stmts.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If { then, otherwise: Some(otherwise), .. } => always_returns(then) && always_returns(otherwise),
        Stmt::Match(m) => !m.arms.is_empty() && m.arms.iter().all(|a| always_returns(&a.body)),
        _ => false,
    })
}

impl Checker<'_> {
    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    /// The body of a function; `owner` = the struct of a method.
    /// Checks that `s` has every method of the interfaces it follows, as they are written
    /// there (with `Self` = `s`), and gives it their default methods.
    fn conforms(&mut self, f: &File, s: &Struct) {
        let me = self.structs[&s.name.text].clone();
        let sub = [("Self".to_string(), me)];
        let mut followed: HashMap<&str, Span> = HashMap::new();
        // Default method name → the interface that gives it.
        let mut given: HashMap<&str, &str> = HashMap::new();
        for n in &s.interfaces {
            let Some(&k) = self.interfaces.get(n.text.as_str()) else {
                let msg = if self.structs.contains_key(&n.text) || primitive(&n.text).is_some() {
                    t!("not_interface", name = n.text)
                } else {
                    t!("interface_unknown", name = n.text)
                };
                self.error(n.span, msg);
                continue;
            };
            if let Some(first) = followed.insert(&n.text, n.span) {
                let (line, _) = line_col(self.src, first.start);
                self.error(n.span, t!("interface_twice", name = n.text, line = line));
                continue;
            }
            let it = &f.interfaces[k];
            let mut asked: Vec<&str> = vec![];
            for (j, (m, body)) in it.methods.iter().enumerate() {
                // Already reported in the interface: a method without `self`, written twice.
                if m.receiver.is_none() || asked.contains(&m.name.text.as_str()) {
                    continue;
                }
                asked.push(&m.name.text);
                let Some(want) = self.functions.get(&method_name(&it.name.text, &m.name.text)) else { continue };
                let want = want.substitute(&sub);
                let mine = method_name(&s.name.text, &m.name.text);
                let heading = heading_text(m);
                if let Some(w) = s.methods.iter().find(|w| w.name.text == m.name.text) {
                    if self.functions.get(&mine).is_some_and(|g| *g != want) {
                        self.diags.push(
                            Diag::new(w.name.span, t!("interface_signature", name = m.name.text, interface = it.name.text))
                                .help(t!("interface_signature_help", interface = it.name.text, heading = heading)),
                        );
                    }
                } else if !body {
                    self.diags.push(
                        Diag::new(n.span, t!("interface_missing", type = s.name.text, name = m.name.text, interface = it.name.text))
                            .help(t!("interface_missing_help", heading = heading)),
                    );
                } else if let Some(other) = given.insert(&m.name.text, &it.name.text) {
                    self.diags.push(
                        Diag::new(n.span, t!("interface_conflict", name = m.name.text, first = other, second = it.name.text))
                            .help(t!("interface_conflict_help", name = m.name.text, type = s.name.text)),
                    );
                } else if s.fields.iter().any(|p| p.name.text == m.name.text) {
                    self.diags.push(
                        Diag::new(n.span, t!("field_and_method", name = m.name.text, type = s.name.text))
                            .help(t!("field_and_method_help", name = m.name.text)),
                    );
                } else {
                    self.functions.insert(mine, want);
                    self.defaults.push((s.name.text.clone(), k, j));
                }
            }
        }
    }

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
            // `x: var T` changes like a `var`; `x: move T` cannot change, like `move self`.
            let kind = if p.mode == Mode::Var { VarKind::Var } else { VarKind::Param };
            params.insert(p.name.text.clone(), Variable { ty: type_of(&p.ty, &self.structs), kind, span: p.name.span });
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

    /// Builds the type of `f.structs[i]` (a struct or an enum) after those of its fields' types.
    /// `open`: the types being built; meeting one again = it would contain itself.
    fn define(&mut self, f: &File, decls: &HashMap<&str, usize>, i: usize, open: &mut Vec<usize>) -> Option<Type> {
        let s = &f.structs[i];
        if let Some(d) = self.structs.get(&s.name.text) {
            return Some(d.clone());
        }
        if open.contains(&i) {
            return None;
        }
        open.push(i);
        let ty = match &s.variants {
            None => {
                let fields = self.fields(f, decls, &s.fields, open);
                for (name, ty) in &fields {
                    if s.copy && !is_copy(ty) && *ty != Type::Error {
                        let span = s.fields.iter().find(|p| p.name.text == *name).unwrap().name.span;
                        let msg = t!("copy_struct_field", type = s.name.text, name = name, field_type = ty.text());
                        self.diags.push(Diag::new(span, msg).help(t!("copy_struct_field_help", type = s.name.text)));
                    }
                }
                Type::Struct(Rc::new(StructDef { name: s.name.text.clone(), fields, copy: s.copy }))
            }
            Some(variants) => {
                let mut seen: HashMap<&str, Span> = HashMap::new();
                let mut defs = vec![];
                for v in variants {
                    let fields = self.fields(f, decls, &v.fields, open);
                    if let Some(first) = seen.insert(&v.name.text, v.name.span) {
                        let (line, _) = line_col(self.src, first.start);
                        self.error(v.name.span, t!("variant_exists", name = v.name.text, line = line));
                    } else {
                        defs.push((v.name.text.clone(), fields));
                    }
                }
                Type::Enum(Rc::new(EnumDef { name: s.name.text.clone(), variants: defs }))
            }
        };
        open.pop();
        self.structs.insert(s.name.text.clone(), ty.clone());
        Some(ty)
    }

    /// The fields of a struct or of a variant, each name once.
    fn fields(&mut self, f: &File, decls: &HashMap<&str, usize>, ps: &[Param], open: &mut Vec<usize>) -> Vec<(String, Type)> {
        let mut fields: Vec<(String, Type)> = vec![];
        for p in ps {
            let ty = self.field_type(f, decls, &p.ty, open);
            if let Some(k) = ps.iter().position(|q| q.name.text == p.name.text)
                && ps[k].name.span != p.name.span
            {
                let (line, _) = line_col(self.src, ps[k].name.span.start);
                self.error(p.name.span, t!("field_exists", name = p.name.text, line = line));
                continue;
            }
            fields.push((p.name.text.clone(), ty));
        }
        fields
    }

    /// The type of a field, building the structs it uses first (see `define`).
    fn field_type(&mut self, f: &File, decls: &HashMap<&str, usize>, t: &TypeAst, open: &mut Vec<usize>) -> Type {
        match t {
            // ponytail: `next: Node?` would need a pointer too; for now it contains itself like a field.
            TypeAst::Optional(t) => match self.field_type(f, decls, t, open) {
                Type::Error => Type::Error,
                t => Type::Optional(Box::new(t)),
            },
            TypeAst::Result(t, e) => match (self.field_type(f, decls, t, open), self.field_type(f, decls, e, open)) {
                (Type::Error, _) | (_, Type::Error) => Type::Error,
                (t, e) => Type::Result(Box::new(t), Box::new(e)),
            },
            TypeAst::Tuple(ts) => {
                let ts: Vec<Type> = ts.iter().map(|t| self.field_type(f, decls, t, open)).collect();
                if ts.contains(&Type::Error) { Type::Error } else { Type::Tuple(ts) }
            }
            TypeAst::Generic(n, ts) => {
                // ponytail: `struct Node { kids: List<Node> }` would need a type that refers to itself.
                if let [TypeAst::Name(e)] = &ts[..]
                    && decls.get(e.text.as_str()).is_some_and(|j| open.contains(j))
                {
                    self.error(e.span, t!("list_of_itself", name = e.text));
                    return Type::Error;
                }
                let mut inner = vec![];
                self.generic(n, ts, |c, t| inner.push(c.field_type(f, decls, t, open)));
                match &inner[..] {
                    [t] if n.text == "List" && *t != Type::Error => Type::List(Box::new(t.clone())),
                    _ => Type::Error,
                }
            }
            TypeAst::Name(n) => {
                if let Some(t) = primitive(&n.text) {
                    return t;
                }
                // A type of the library (`IoError`).
                if !decls.contains_key(n.text.as_str())
                    && let Some(t) = self.structs.get(&n.text)
                {
                    return t.clone();
                }
                let Some(&j) = decls.get(n.text.as_str()) else {
                    self.error(n.span, t!("type_unknown", name = n.text));
                    return Type::Error;
                };
                match self.define(f, decls, j, open) {
                    Some(t) => t,
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
            TypeAst::Name(n) if n.text == "Self" && !self.structs.contains_key("Self") => {
                self.diags.push(Diag::new(n.span, t!("self_type_outside")).help(t!("self_type_outside_help")));
            }
            TypeAst::Name(n) if self.interfaces.contains_key(&n.text) => self.diags.push(
                Diag::new(n.span, t!("interface_as_type", name = n.text)).help(t!("interface_as_type_help", name = n.text)),
            ),
            TypeAst::Name(n) if type_of(t, &self.structs) == Type::Error => self.error(n.span, t!("type_unknown", name = n.text)),
            TypeAst::Name(_) => {}
            TypeAst::Tuple(ts) => ts.iter().for_each(|t| self.report_unknown(t)),
            TypeAst::Generic(n, ts) => self.generic(n, ts, |c, t| c.report_unknown(t)),
            TypeAst::Optional(t) => self.report_unknown(t),
            TypeAst::Result(t, e) => {
                self.report_unknown(t);
                self.report_unknown(e);
            }
        }
    }

    /// `List<T>` is the only generic type for now (J5: the others); `each` checks `T`.
    fn generic(&mut self, n: &Name, ts: &[TypeAst], mut each: impl FnMut(&mut Self, &TypeAst)) {
        if n.text != "List" {
            self.error(n.span, t!("generic_unknown", name = n.text));
        } else if ts.len() != 1 {
            self.error(n.span, t!("list_one_type", count = ts.len()));
        }
        ts.iter().for_each(|t| each(self, t));
    }

    fn written_type(&mut self, t: &TypeAst) -> Type {
        self.report_unknown(t);
        type_of(t, &self.structs)
    }

    fn lookup(&self, name: &str) -> Option<&Variable> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn contract(&mut self, k: &Contract) {
        self.in_contract = true;
        self.expect(&k.cond, &Type::Bool);
        if let Some(m) = &k.message {
            self.parts(m);
        }
        self.in_contract = false;
    }

    /// Opens a scope where each of `names`, an optional variable, is its value:
    /// `x` is an `Int` inside `if x != none { … }` (PLAN 2.5, like Eiffel's object test).
    fn narrow(&mut self, names: &[&str]) {
        let mut scope = HashMap::new();
        for n in names {
            if let Some(v) = self.lookup(n)
                && let Type::Optional(t) = &v.ty
            {
                scope.insert(n.to_string(), Variable { ty: (**t).clone(), kind: VarKind::Narrowed, span: v.span });
            }
        }
        self.scopes.push(scope);
    }

    /// The `{values}` of a text: anything `print` can show.
    fn parts(&mut self, parts: &[Part]) {
        for p in parts {
            let Part::Code(e) = p else { continue };
            match self.expr(e) {
                Type::Unit => self.error(e.span, t!("interp_no_value")),
                // A `none` written into a text for people is a mistake of older languages
                // (`undefined`, `None`): the program says what to write instead.
                Type::Optional(_) => {
                    let code = &self.src[e.span.start..e.span.end];
                    self.diags.push(
                        Diag::new(e.span, t!("interp_optional", code = code)).help(t!("interp_optional_help", code = code)),
                    );
                }
                Type::Result(..) => {
                    let code = &self.src[e.span.start..e.span.end];
                    self.diags.push(
                        Diag::new(e.span, t!("interp_result", code = code)).help(t!("interp_result_help", code = code)),
                    );
                }
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
        // `(1, none)` where an `(Int, String?)` is expected: each element against its type.
        if let (ExprKind::Tuple(elems), Type::Tuple(ts)) = (&e.kind, expected)
            && elems.len() == ts.len()
        {
            elems.iter().zip(ts).for_each(|(x, t)| self.expect(x, t));
            self.types[e.id] = expected.clone();
            return;
        }
        // `[1, 2]` where a `List<Float>` is expected: each element is checked against `Float`; `[]` takes the type.
        if let (ExprKind::List(elems), Type::List(elem)) = (&e.kind, expected) {
            elems.iter().for_each(|x| self.expect(x, elem));
            self.types[e.id] = expected.clone();
            return;
        }
        if let ExprKind::If { cond, then, otherwise } = &e.kind {
            self.types[e.id] = self.if_value(cond, then, otherwise, Some(expected));
            return;
        }
        if let ExprKind::Match(m) = &e.kind {
            self.types[e.id] = self.match_(m, Some(expected), true);
            return;
        }
        // `ok(v)` / `err(e)` where a `T or E` is expected.
        if let (ExprKind::Outcome { ok, value }, Type::Result(t, er)) = (&e.kind, expected) {
            self.expect(value, if *ok { t } else { er });
            self.types[e.id] = expected.clone();
            return;
        }
        // Where a `T?` is expected: `none`, a `T?`, or a `T` that becomes one (`wraps`).
        if let Type::Optional(inner) = expected {
            if let ExprKind::None = e.kind {
                self.types[e.id] = expected.clone();
                return;
            }
            // Typed by where they go: `[]`, `2` as a Float.
            if matches!(e.kind, ExprKind::List(_) | ExprKind::Int(_) | ExprKind::Unary(UnOp::Neg, _)) {
                self.expect(e, inner);
                self.wraps[e.id] = true;
                return;
            }
            let t = self.expr(e);
            if t == **inner {
                self.wraps[e.id] = true;
            } else {
                self.mismatch(e, t, expected);
            }
            return;
        }
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
                        Step::At(i) => {
                            self.expect(i, &Type::Int);
                            match ty {
                                Type::List(elem) => *elem,
                                Type::Error => Type::Error,
                                t => {
                                    self.error(i.span, t!("index_not_list", type = t.text()));
                                    Type::Error
                                }
                            }
                        }
                    };
                }
                ty
            }
        }
    }

    /// `n` (a variable of this kind) is assigned or given to a `var self` method.
    fn may_change(&mut self, n: &str, span: Span, kind: VarKind) {
        match kind {
            VarKind::Var | VarKind::Receiver(Mode::Var) => {}
            VarKind::Let => self.diags.push(
                Diag::new(span, t!("declared_with_let", name = n)).help(t!("declared_with_let_help", name = n)),
            ),
            VarKind::Param => self.diags.push(
                Diag::new(span, t!("param_immutable", name = n)).help(t!("param_immutable_help", name = n)),
            ),
            VarKind::Counter => self.error(span, t!("counter_immutable", name = n)),
            VarKind::Element => self.error(span, t!("element_immutable", name = n)),
            VarKind::Matched => self.error(span, t!("matched_immutable", name = n)),
            VarKind::Narrowed => self.diags.push(
                Diag::new(span, t!("narrowed_immutable", name = n)).help(t!("narrowed_immutable_help", name = n)),
            ),
            VarKind::Receiver(r) => {
                self.diags.push(Diag::new(span, t!("self_immutable", receiver = r.text())).help(t!("self_immutable_help")))
            }
        }
    }

    /// `rex.rename(...)` with `var self`, or `f(var a)`: the place must be a variable that may
    /// change (or one of its fields). `temporary`: the error for a computed value.
    fn changes(&mut self, place: &Expr, temporary: String) {
        let Some((root, span)) = place_root(place, &self.types) else {
            self.error(place.span, temporary);
            return;
        };
        if let Some(kind) = self.lookup(root).map(|v| v.kind) {
            self.may_change(root, span, kind);
        }
    }

    /// The arguments of a call, checked against the parameters: their types, and the markers
    /// of rule 2.7.4 (`var a` for `x: var T`, `move a` for `x: move T`).
    fn arguments(&mut self, name: &Name, args: &[Expr], params: &[(Mode, Type)]) {
        if params.len() != args.len() {
            let msg = t!("argument_count", name = name.text, expected = params.len(), given = args.len());
            self.error(name.span, msg);
        }
        for (i, a) in args.iter().enumerate() {
            let Some((mode, t)) = params.get(i) else {
                self.expr(a);
                continue;
            };
            let (mark, value) = match &a.kind {
                ExprKind::Mark(m, x) => (*m, &**x),
                _ => (Mode::Read, a),
            };
            self.expect(value, t);
            self.types[a.id] = self.types[value.id].clone();
            let arg = &self.src[value.span.start..value.span.end];
            if *mode == Mode::Var && self.wraps[value.id] {
                // The changed value comes back into the caller's variable: it must be a `T?` too.
                self.error(value.span, t!("var_arg_optional", arg = arg, type = self.types[value.id].text(), declared = t.text()));
            }
            match (mode, mark) {
                (Mode::Var, Mode::Var) => self.changes(value, t!("var_arg_temporary", name = name.text)),
                (Mode::Var, _) => self.error(a.span, t!("var_mark_missing", name = name.text, arg = arg)),
                (_, Mode::Var) | (Mode::Read, Mode::Move) => {
                    let declared = format!("{} {}", mode.word(), t.text());
                    self.diags.push(
                        Diag::new(a.span, t!("mark_unexpected", name = name.text, mark = mark.word()))
                            .help(t!("mark_unexpected_help", declared = declared.trim_start())),
                    );
                }
                // A computed value (`keep(make_name)`) is given without a marker.
                (Mode::Move, Mode::Read) if !is_copy(t) && place_root(value, &self.types).is_some() => self.diags.push(
                    Diag::new(a.span, t!("move_mark_missing", name = name.text, arg = arg))
                        .help(t!("move_mark_missing_help", arg = arg)),
                ),
                _ => {}
            }
        }
    }

    /// `rex.sound`: a method of `rex`'s struct.
    fn method_call(&mut self, receiver: &Expr, owner: &str, name: &Name, args: &[Expr], sig: Signature) -> Type {
        match sig.receiver {
            None => self.diags.push(
                Diag::new(name.span, t!("static_on_value", name = name.text, type = owner))
                    .help(t!("static_on_value_help", name = name.text, type = owner)),
            ),
            Some(Mode::Var) => self.changes(receiver, t!("var_method_temporary", name = name.text)),
            Some(_) => {}
        }
        self.arguments(name, args, &sig.params);
        sig.ret
    }

    /// `l.length`, `l.add(x)`, `l.remove(i)` (PLAN 2.10); `None` for `clone` (shared with the other types).
    fn list_method(&mut self, receiver: &Expr, elem: &Type, name: &Name, args: &[Expr]) -> Option<Type> {
        let (params, result) = match name.text.as_str() {
            "length" => (vec![], Type::Int),
            "add" => (vec![(Mode::Move, elem.clone())], Type::Unit),
            "remove" => (vec![(Mode::Read, Type::Int)], elem.clone()),
            "clone" => return None,
            _ => {
                let t = Type::List(Box::new(elem.clone())).text();
                self.diags.push(
                    Diag::new(name.span, t!("no_method", type = t, name = name.text))
                        .help(t!("methods_of", type = t, methods = "`length`, `add`, `remove`, `clone`")),
                );
                args.iter().for_each(|a| {
                    self.expr(a);
                });
                return Some(Type::Error);
            }
        };
        if name.text != "length" {
            self.changes(receiver, t!("var_method_temporary", name = name.text));
        }
        if params.is_empty() && !args.is_empty() {
            self.error(name.span, t!("no_arguments", name = name.text));
        } else if !params.is_empty() {
            self.arguments(name, args, &params);
        }
        Some(result)
    }

    /// `Dog.create("Rex")`: a struct's function without `self`.
    fn static_call(&mut self, ty: &Name, name: &Name, args: &[Expr]) -> Type {
        let found = match self.structs.get(&ty.text).cloned() {
            None if primitive(&ty.text).is_some() => {
                self.error(name.span, t!("no_static", type = ty.text, name = name.text));
                None
            }
            None => {
                self.error(ty.span, t!("type_unknown", name = ty.text));
                None
            }
            Some(t) => match self.functions.get(&method_name(&ty.text, &name.text)).cloned() {
                None => {
                    self.no_field(&t, name);
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
                    self.no_field(&ty, name);
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

    /// `rex.color`: no such field or method in `Dog` (a struct or an enum).
    fn no_field(&mut self, t: &Type, name: &Name) {
        let owner = t.text();
        let prefix = method_name(&owner, "");
        let mut methods: Vec<&str> = self.functions.keys().filter_map(|k| k.strip_prefix(&prefix)).collect();
        methods.sort();
        let own: Vec<&str> = match t {
            Type::Struct(d) => d.fields.iter().map(|(n, _)| n.as_str()).collect(),
            _ => vec![],
        };
        let fields: Vec<String> = own.into_iter().chain(methods).map(|n| format!("`{n}`")).collect();
        let mut diag = Diag::new(name.span, t!("no_member", type = owner, name = name.text));
        if !fields.is_empty() {
            diag = diag.help(t!("fields_of", type = owner, fields = fields.join(", ")));
        }
        self.diags.push(diag);
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            // `let rex := Dog { ... }`, `let c := Shape.Circle(2.0)`: the type is written on the right (PLAN 2.5).
            Stmt::Declare { mutable, pattern, ty: None, value } if matches!(value.kind, ExprKind::StructLit { .. } | ExprKind::Variant { .. }) => {
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
                self.narrow(&narrowed(cond, true));
                self.block(then, vec![]);
                self.scopes.pop();
                if let Some(o) = otherwise {
                    self.narrow(&narrowed(cond, false));
                    self.block(o, vec![]);
                    self.scopes.pop();
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
            Stmt::Each { name, list, body } => {
                let elem = match self.expr(list) {
                    Type::List(elem) => *elem,
                    Type::Error => Type::Error,
                    t => {
                        self.diags.push(
                            Diag::new(list.span, t!("for_needs_list", type = t.text())).help(t!("for_range_help")),
                        );
                        Type::Error
                    }
                };
                self.loops += 1;
                self.block(body, vec![(name, elem, VarKind::Element)]);
                self.loops -= 1;
            }
            Stmt::Match(m) => {
                self.match_(m, None, false);
            }
            Stmt::Expr(e) => {
                self.expr(e);
                let call = match &e.kind {
                    ExprKind::Call { .. } | ExprKind::Static { .. } => true,
                    ExprKind::Method { receiver, name, .. } => match &self.types[receiver.id] {
                        t @ (Type::Struct(_) | Type::Enum(_)) => self.functions.contains_key(&method_name(&t.text(), &name.text)),
                        Type::List(_) => matches!(name.text.as_str(), "add" | "remove"),
                        Type::Param(p) => p.bounds.iter().any(|b| self.functions.contains_key(&method_name(b, &name.text))),
                        _ => false,
                    },
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
            // Allowed only as an argument (`arguments`).
            ExprKind::Mark(m, x) => {
                self.error(e.span, t!("mark_not_here", mark = m.word()));
                self.expr(x)
            }
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
            ExprKind::List(elems) => {
                let Some((first, others)) = elems.split_first() else {
                    self.diags.push(Diag::new(e.span, t!("empty_list_type")).help(t!("empty_list_type_help")));
                    return Type::Error;
                };
                let t = self.expr(first);
                if t == Type::Unit {
                    self.error(first.span, t!("tuple_no_value"));
                }
                others.iter().for_each(|x| self.expect(x, &t));
                if t == Type::Error { Type::Error } else { Type::List(Box::new(t)) }
            }
            ExprKind::New(ty) => {
                let t = self.written_type(ty);
                if !matches!(t, Type::List(_) | Type::Error) {
                    self.error(e.span, t!("new_not_generic", type = t.text()));
                    return Type::Error;
                }
                t
            }
            ExprKind::Index(x, i) => {
                let t = self.expr(x);
                self.expect(i, &Type::Int);
                match t {
                    Type::List(elem) => *elem,
                    Type::Error => Type::Error,
                    t => {
                        self.error(e.span, t!("index_not_list", type = t.text()));
                        Type::Error
                    }
                }
            }
            ExprKind::StructLit { ty, fields } => self.struct_literal(ty, fields),
            ExprKind::Call { name, args } => self.call(name, args),
            ExprKind::Static { ty, name, args } => self.static_call(ty, name, args),
            ExprKind::Variant { ty, name, args } => self.variant(ty, name, args),
            ExprKind::None => {
                self.diags.push(Diag::new(e.span, t!("none_needs_type")).help(t!("none_needs_type_help")));
                Type::Error
            }
            ExprKind::Outcome { ok, value } => {
                self.expr(value);
                let word = if *ok { "ok" } else { "err" };
                self.diags.push(Diag::new(e.span, t!("outcome_needs_type", word = word)).help(t!("outcome_needs_type_help")));
                Type::Error
            }
            ExprKind::Try(x) => match self.expr(x) {
                // The failure goes up as it is: the function's `or` side must be the same type.
                Type::Result(t, er) => {
                    if self.in_contract {
                        self.error(e.span, t!("try_in_contract"));
                    } else if !matches!(&self.ret, Type::Result(_, r) if *r == er) && self.ret != Type::Error {
                        let ret = if self.ret == Type::Unit { self.ret.text() } else { format!("`{}`", self.ret.text()) };
                        self.diags.push(
                            Diag::new(e.span, t!("try_needs_result", error = er.text(), type = ret))
                                .help(t!("try_needs_result_help", error = er.text())),
                        );
                    }
                    *t
                }
                Type::Optional(inner) => {
                    if self.in_contract {
                        self.error(e.span, t!("try_in_contract"));
                    } else if !matches!(self.ret, Type::Optional(_) | Type::Error) {
                        let ret = if self.ret == Type::Unit { self.ret.text() } else { format!("`{}`", self.ret.text()) };
                        self.diags.push(Diag::new(e.span, t!("try_needs_optional", type = ret)).help(t!("try_needs_optional_help")));
                    }
                    *inner
                }
                Type::Error => Type::Error,
                t => {
                    self.error(e.span, t!("try_not_optional", type = t.text()));
                    Type::Error
                }
            },
            ExprKind::Match(m) => self.match_(m, None, true),
            ExprKind::Method { receiver, name, args } => {
                let t = self.expr(receiver);
                if matches!(t, Type::Struct(_) | Type::Enum(_))
                    && let Some(sig) = self.functions.get(&method_name(&t.text(), &name.text)).cloned()
                {
                    return self.method_call(receiver, &t.text(), name, args, sig);
                }
                if let Type::List(elem) = &t
                    && let Some(r) = self.list_method(receiver, elem, name, args)
                {
                    return r;
                }
                // `self.sound` in a default method: what the interface promises.
                if let Type::Param(p) = &t
                    && let Some((b, sig)) =
                        p.bounds.iter().find_map(|b| Some((b.clone(), self.functions.get(&method_name(b, &name.text))?.clone())))
                {
                    return self.method_call(receiver, &b, name, args, sig);
                }
                for a in args {
                    self.expr(a);
                }
                if t == Type::Error {
                    return Type::Error;
                }
                // `.clone` on any value made of several slots: a deep copy (its Strings are copied too).
                if name.text == "clone"
                    && matches!(t, Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::List(_) | Type::Param(_))
                    && !t.is_field("clone")
                {
                    if !args.is_empty() {
                        self.error(name.span, t!("no_arguments", name = name.text));
                    }
                    return t;
                }
                if let Type::Enum(_) = &t {
                    self.no_field(&t, name);
                    return Type::Error;
                }
                if let Type::Param(p) = &t {
                    let prefixes: Vec<String> = p.bounds.iter().map(|b| format!("{b}.")).collect();
                    let mut known: Vec<String> = (self.functions.keys())
                        .filter_map(|k| prefixes.iter().find_map(|pre| k.strip_prefix(pre.as_str())))
                        .map(|n| format!("`{n}`"))
                        .collect();
                    known.sort();
                    known.push("`clone`".into());
                    self.diags.push(
                        Diag::new(name.span, t!("no_method", type = t.text(), name = name.text))
                            .help(t!("methods_of", type = t.text(), methods = known.join(", "))),
                    );
                    return Type::Error;
                }
                if let Type::Struct(d) = &t {
                    let Some((_, ft)) = d.field(&name.text) else {
                        self.no_field(&t, name);
                        return Type::Error;
                    };
                    if !args.is_empty() {
                        self.error(name.span, t!("no_arguments", name = name.text));
                    }
                    return ft.clone();
                }
                let Some((_, _, result, maybe)) = METHODS.iter().find(|(r, n, ..)| *r == t && *n == name.text) else {
                    let known: Vec<String> =
                        METHODS.iter().filter(|(r, ..)| *r == t).map(|(_, n, ..)| format!("`{n}`")).collect();
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
                if *maybe { Type::Optional(Box::new(result.clone())) } else { result.clone() }
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
            ExprKind::If { cond, then, otherwise } => self.if_value(cond, then, otherwise, None),
            ExprKind::Binary(op, a, b) => {
                use BinOp::*;
                match op {
                    Or | And => {
                        self.expect(a, &Type::Bool);
                        // `x != none && x > 3`: `x` is its value on the right.
                        self.narrow(&if *op == And { narrowed(a, true) } else { vec![] });
                        self.expect(b, &Type::Bool);
                        self.scopes.pop();
                        Type::Bool
                    }
                    Eq | Ne => {
                        // `x != none`, `ok(3) = r`: `none` and `ok(…)` take the type of the other side.
                        let (a, b) = if matches!(a.kind, ExprKind::None | ExprKind::Outcome { .. }) { (b, a) } else { (a, b) };
                        let ta = self.expr(a);
                        if let ExprKind::None = b.kind {
                            if matches!(ta, Type::Optional(_) | Type::Error) {
                                self.types[b.id] = ta;
                            } else {
                                self.diags.push(
                                    Diag::new(e.span, t!("never_none", type = ta.text())).help(t!("never_none_help", type = ta.text())),
                                );
                            }
                            return Type::Bool;
                        }
                        match ta {
                            Type::Optional(_) if !is_copy(&ta) => self.diags.push(
                                Diag::new(e.span, t!("optional_compare", op = op.text(), type = ta.text()))
                                    .help(t!("optional_compare_help")),
                            ),
                            Type::Unit => self.error(a.span, t!("expression_nothing")),
                            Type::Tuple(_) => self.diags.push(
                                Diag::new(e.span, t!("tuple_compare", op = op.text())).help(t!("tuple_compare_help")),
                            ),
                            Type::Struct(_) | Type::Param(_) => self.diags.push(
                                Diag::new(e.span, t!("struct_compare", op = op.text(), type = ta.text()))
                                    .help(t!("struct_compare_help")),
                            ),
                            Type::List(_) => self.error(e.span, t!("list_compare", op = op.text())),
                            Type::Enum(_) | Type::Result(..) if !is_copy(&ta) => self.diags.push(
                                Diag::new(e.span, t!("enum_compare", op = op.text(), type = ta.text()))
                                    .help(t!("enum_compare_help")),
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
        let d = match self.structs.get(&ty.text).cloned() {
            Some(Type::Struct(d)) => d,
            other => {
                match other {
                    Some(_) => self.diags.push(
                        Diag::new(ty.span, t!("enum_literal", name = ty.text)).help(t!("enum_literal_help", name = ty.text)),
                    ),
                    None => self.error(ty.span, t!("type_unknown", name = ty.text)),
                }
                fields.iter().for_each(|(_, v)| {
                    self.expr(v);
                });
                return Type::Error;
            }
        };
        let mut given: Vec<&str> = vec![];
        for (name, value) in fields {
            let Some((_, t)) = d.field(&name.text) else {
                self.no_field(&Type::Struct(d.clone()), name);
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

    /// `Shape.Circle(2.0)`: the fields in their declared order.
    fn variant(&mut self, ty: &Name, name: &Name, args: &[Expr]) -> Type {
        let d = match self.structs.get(&ty.text).cloned() {
            Some(Type::Enum(d)) => Some(d),
            Some(_) => {
                self.error(name.span, t!("not_enum", type = ty.text, name = name.text));
                None
            }
            None if primitive(&ty.text).is_some() => {
                self.error(name.span, t!("not_enum", type = ty.text, name = name.text));
                None
            }
            None => {
                self.error(ty.span, t!("type_unknown", name = ty.text));
                None
            }
        };
        let fields = d.as_ref().map(|d| (d.variant(&name.text).map(|(_, f)| f.to_vec()), d));
        let fields = match fields {
            Some((Some(fields), _)) => fields,
            Some((None, d)) => {
                self.diags.push(
                    Diag::new(name.span, t!("no_variant", type = d.name, name = name.text))
                        .help(t!("variants_of", type = d.name, variants = d.names())),
                );
                return self.unchecked(args);
            }
            None => return self.unchecked(args),
        };
        let params: Vec<(Mode, Type)> = fields.into_iter().map(|(_, t)| (Mode::Read, t)).collect();
        self.arguments(name, args, &params);
        Type::Enum(d.unwrap())
    }

    /// Arguments of something unknown: checked alone, for their own errors.
    fn unchecked(&mut self, args: &[Expr]) -> Type {
        for a in args {
            self.expr(a);
        }
        Type::Error
    }

    /// `match x { … }`; `value`: each arm gives a value (of the `expected` type if known).
    fn match_(&mut self, m: &Match, expected: Option<&Type>, value: bool) -> Type {
        let t = self.expr(&m.subject);
        let optional = matches!(t, Type::Optional(_));
        let outcome = matches!(t, Type::Result(..));
        let d = match &t {
            Type::Enum(_) | Type::Optional(_) | Type::Result(..) => enum_view(&t),
            Type::Error => None,
            t => {
                self.error(m.subject.span, t!("match_needs_enum", type = t.text()));
                None
            }
        };
        // Variant number → where its arm is.
        let mut seen: HashMap<usize, Span> = HashMap::new();
        let mut any: Option<Span> = None;
        let mut result: Option<Type> = None;
        for arm in &m.arms {
            let span = match &arm.case {
                Case::Variant { name, .. } => name.span,
                Case::Bind(Pattern::Name(n)) => n.span,
                Case::Bind(Pattern::Tuple(_, s) | Pattern::Ignore(s)) | Case::Any(s) => *s,
            };
            let shown = self.src[span.start..span.end].to_string();
            if let Some(a) = any {
                let (line, _) = line_col(self.src, a.start);
                self.error(span, t!("case_after_any", line = line));
            }
            self.scopes.push(HashMap::new());
            match (arm.case.variant(), &d) {
                (None, Some(d)) => {
                    if any.is_none() && seen.len() == d.variants.len() {
                        self.error(span, t!("case_any_useless", type = d.name));
                    }
                    any = Some(span);
                }
                (None, None) => any = Some(span),
                (Some((name, binds)), d) => {
                    // An optional takes `none` and a name; an enum, its variants.
                    let wrong = match &arm.case {
                        Case::Bind(_) => !optional,
                        _ if optional => name != "none",
                        _ => outcome && !matches!(name, "ok" | "err"),
                    };
                    let fields = match d.as_ref().map(|d| (d.variant(name), d)) {
                        Some((_, d)) if wrong => {
                            if optional {
                                self.error(span, t!("optional_cases", type = d.name));
                            } else if outcome {
                                self.error(span, t!("result_cases", type = t.text()));
                            } else {
                                self.diags.push(
                                    Diag::new(span, t!("bind_on_enum", name = shown, type = d.name))
                                        .help(t!("variants_of", type = d.name, variants = d.names())),
                                );
                            }
                            vec![]
                        }
                        Some((Some((i, fields)), _)) => {
                            if let Some(first) = seen.insert(i, span) {
                                let (line, _) = line_col(self.src, first.start);
                                self.error(span, t!("case_twice", name = shown, line = line));
                            }
                            // `Circle =>` without its fields: they are not needed.
                            if !binds.is_empty() && binds.len() != fields.len() {
                                let names: Vec<String> = fields.iter().map(|(n, _)| format!("`{n}`")).collect();
                                self.error(span, t!("case_fields", name = name, count = fields.len(), fields = names.join(", "), given = binds.len()));
                            }
                            fields.iter().map(|(_, t)| t.clone()).collect()
                        }
                        Some((None, d)) => {
                            self.diags.push(
                                Diag::new(span, t!("no_variant", type = d.name, name = name))
                                    .help(t!("variants_of", type = d.name, variants = d.names())),
                            );
                            vec![]
                        }
                        None => vec![],
                    };
                    for (i, p) in binds.iter().enumerate() {
                        let ty = fields.get(i).cloned().unwrap_or(Type::Error);
                        self.bind(p, &ty, VarKind::Matched);
                    }
                }
            }
            if value {
                let ty = self.block_value(&arm.body, expected.or(result.as_ref()));
                if result.is_none() {
                    result = Some(ty);
                }
            } else {
                self.block(&arm.body, vec![]);
            }
            self.scopes.pop();
        }
        if let Some(d) = &d
            && any.is_none()
        {
            let missing: Vec<String> = (0..d.variants.len())
                .filter(|i| !seen.contains_key(i))
                .map(|i| match &d.variants[i].0 {
                    _ if optional && i == 1 => t!("case_value"),
                    n if outcome => format!("`{n}(…)`"),
                    n => format!("`{n}`"),
                })
                .collect();
            if !missing.is_empty() {
                self.diags.push(
                    Diag::new(m.subject.span, t!("cases_missing", type = d.name, cases = missing.join(", ")))
                        .help(t!("cases_missing_help")),
                );
            }
        }
        result.unwrap_or(Type::Error)
    }

    /// `if c { a } else { b }`; with an `expected` type, each branch is checked against it,
    /// so that `[]` or `2` take it as after a `:=`.
    fn if_value(&mut self, cond: &Expr, then: &Block, otherwise: &Block, expected: Option<&Type>) -> Type {
        self.expect(cond, &Type::Bool);
        self.narrow(&narrowed(cond, true));
        let a = self.block_value(then, expected);
        self.scopes.pop();
        self.narrow(&narrowed(cond, false));
        let b = self.block_value(otherwise, expected);
        self.scopes.pop();
        if a != b && a != Type::Error && b != Type::Error {
            let msg = t!("branches_differ", a = a.text(), b = b.text());
            self.error(otherwise.open, msg);
            return Type::Error;
        }
        a
    }

    /// The type of a block giving a value: that of its last line.
    fn block_value(&mut self, b: &Block, expected: Option<&Type>) -> Type {
        self.scopes.push(HashMap::new());
        let (last, before) = match b.stmts.split_last() {
            Some((l, b)) => (Some(l), b),
            None => (None, &[][..]),
        };
        for s in before {
            self.stmt(s);
        }
        let ty = match last {
            Some(Stmt::Expr(e)) => match expected {
                Some(t) => {
                    self.expect(e, t);
                    t.clone()
                }
                None => self.expr(e),
            },
            Some(Stmt::If { cond, then, otherwise: Some(otherwise) }) => self.if_value(cond, then, otherwise, expected),
            Some(Stmt::Match(m)) => self.match_(m, expected, true),
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
                if self.expr(a) == Type::Unit {
                    self.error(a.span, t!("print_no_value", name = name.text));
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
