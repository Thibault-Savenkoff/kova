// Type checking: known names, correct types, `let`/`var` rules.
//
// Kova guesses (almost) nothing: it checks that what is written is consistent.
//
//   let x: Int := true    →  error: different types: `Int` expected, `Bool` given
//   a := 2  (a is a let)  →  error: `a` is declared with let
//
// The result: the type of each expression (indexed by `Expr::id`).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::*;
use crate::diag::{Diag, Span, file_number, line_col};
use crate::locale::{self, t};

#[derive(Clone, PartialEq, Debug)]
pub enum Type {
    Int,
    Float,
    Bool,
    Char,
    /// UTF-8 text, owned: moved when given away, freed at the end of its block (PLAN 2.7).
    String,
    /// An integer without limits, owned like a `String` (its decimal digits, `kir.rs`).
    BigInt,
    /// A day, `2026-10-23`: its number of days since 1970-01-01, in one slot (PLAN Part 4, "Dates").
    Date,
    /// A day and a time, `2026-10-23 14:05:30`, without time zone: its seconds since 1970-01-01 00:00.
    DateTime,
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
    /// `Map<String, Int>`: keys and their values, in the order they were added. Laid out like a
    /// `List<(String, Int)>` (PLAN Part 4, "J6, step 4").
    Map(Box<Type>, Box<Type>),
    /// `Set<String>`: values without duplicates, in the order they were added. Laid out like a
    /// `List<String>` (PLAN Part 4, "Slices, `Set<T>`, default arguments").
    Set(Box<Type>),
    /// `any Animal`: a value of any type following the interface, known at run time (PLAN 2.5).
    /// One slot: a box holding the real type's number, then its slots.
    Any(String),
    /// A type known only by what it promises: `Self` in an interface. Replaced by the real type
    /// (`Dog`) in the KIR, one copy of the function per real type (`substitute`).
    Param(Rc<ParamDef>),
    /// `{Int -> Bool}`: a block, only as a parameter (it never outlives its call); `Unit` if it gives nothing.
    Block(Vec<Type>, Box<Type>),
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

    /// The name its methods are found under: `Stack` for `Stack<Int>` (`Stack.push`).
    pub fn owner(&self) -> String {
        match self {
            Type::Struct(d) => d.name.clone(),
            t => t.text(),
        }
    }

    /// The type as in the code: `Int`, `(Int, Float)`.
    pub fn text(&self) -> String {
        match self {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::Bool => "Bool".into(),
            Type::Char => "Char".into(),
            Type::String => "String".into(),
            Type::BigInt => "BigInt".into(),
            Type::Date => "Date".into(),
            Type::DateTime => "DateTime".into(),
            Type::Tuple(ts) => format!("({})", ts.iter().map(Type::text).collect::<Vec<_>>().join(", ")),
            Type::Struct(d) if d.args.is_empty() => d.name.clone(),
            Type::Struct(d) => format!("{}<{}>", d.name, d.args.iter().map(Type::text).collect::<Vec<_>>().join(", ")),
            Type::Enum(d) => d.name.clone(),
            Type::Optional(t) => format!("{}?", t.text()),
            Type::Result(t, e) if **t == Type::Unit => format!("Nothing or {}", e.text()),
            Type::Result(t, e) => format!("{} or {}", t.text(), e.text()),
            Type::List(t) => format!("List<{}>", t.text()),
            Type::Map(k, v) => format!("Map<{}, {}>", k.text(), v.text()),
            Type::Set(t) => format!("Set<{}>", t.text()),
            Type::Param(p) => p.name.clone(),
            Type::Any(i) => format!("any {i}"),
            Type::Block(ps, r) => {
                let r = (**r != Type::Unit).then(|| r.text());
                crate::ast::block_text(ps.iter().map(Type::text).collect(), r)
            }
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
        Type::Map(k, v) => Type::Map(sub(k), sub(v)),
        Type::Set(t) => Type::Set(sub(t)),
        Type::Block(ps, r) => Type::Block(ps.iter().map(|t| substitute(t, s)).collect(), sub(r)),
        // `Stack<T>` → `Stack<Int>`: its fields change with it.
        Type::Struct(d) if !d.args.is_empty() => Type::Struct(Rc::new(StructDef {
            name: d.name.clone(),
            args: d.args.iter().map(|t| substitute(t, s)).collect(),
            fields: d.fields.iter().map(|(n, t)| (n.clone(), substitute(t, s))).collect(),
            copy: d.copy,
        })),
        _ => t.clone(),
    }
}

/// `Stack<T>` as declared, under its own name (`T` may stand for a `Stack<Int>`).
fn is_template(t: &Type, name: &str) -> bool {
    matches!(t, Type::Struct(d) if !d.args.is_empty() && d.name == name)
}

/// `Stack<Int>` from the struct `Stack<T>` as declared (`None` if it has no type parameters).
pub fn instance(template: &Type, types: &[Type]) -> Option<Type> {
    let Type::Struct(d) = template else { return None };
    let names = d.args.iter().map(|a| match a {
        Type::Param(p) => p.name.clone(),
        _ => unreachable!("a declared struct has its parameters as arguments"),
    });
    (!d.args.is_empty() && d.args.len() == types.len()).then(|| substitute(template, &names.zip(types.iter().cloned()).collect::<Vec<_>>()))
}

#[derive(PartialEq, Debug)]
pub struct StructDef {
    pub name: String,
    /// `Stack<Int>`: the types given for its type parameters (`Stack<T>` itself: `T`).
    pub args: Vec<Type>,
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
        // `Nothing or IoError`: `ok` alone, without a value.
        Type::Result(ok, err) => Some(Rc::new(EnumDef {
            name: t.text(),
            variants: vec![
                ("err".into(), vec![("error".into(), (**err).clone())]),
                ("ok".into(), if **ok == Type::Unit { vec![] } else { vec![("value".into(), (**ok).clone())] }),
            ],
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
    &[
        ("read_file", "fn read_file(path: String) -> String or IoError"),
        ("write_file", "fn write_file(path: String, text: String) -> Nothing or IoError"),
        ("append_file", "fn append_file(path: String, text: String) -> Nothing or IoError"),
        ("exists", "fn exists(path: String) -> Bool"),
        ("list_dir", "fn list_dir(path: String) -> List<String> or IoError"),
        ("make_dir", "fn make_dir(path: String) -> Nothing or IoError"),
        ("remove_file", "fn remove_file(path: String) -> Nothing or IoError"),
        ("now", "fn now -> Float"),
        ("sleep", "fn sleep(seconds: Float)"),
        ("random", "fn random(from..to) -> Int"),
        ("read_line", "fn read_line -> String?"),
        ("args", "fn args -> List<String>"),
        ("exit", "fn exit(code: Int)"),
    ];

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
    (Type::String, "to_float", Type::Float, true),
    (Type::Int, "to_big", Type::BigInt, false),
    // `none` if it is too big for an `Int`.
    (Type::BigInt, "to_int", Type::Int, true),
    (Type::BigInt, "clone", Type::BigInt, false),
    // Every type has `.clone` (PLAN Part 4, "J5"); for these, a plain copy.
    (Type::Int, "clone", Type::Int, false),
    (Type::Float, "clone", Type::Float, false),
    (Type::Bool, "clone", Type::Bool, false),
    (Type::Char, "clone", Type::Char, false),
    // `Display`: the text `print` writes.
    (Type::Int, "to_string", Type::String, false),
    (Type::Float, "to_string", Type::String, false),
    (Type::Bool, "to_string", Type::String, false),
    (Type::Char, "to_string", Type::String, false),
    (Type::String, "to_string", Type::String, false),
    (Type::BigInt, "to_string", Type::String, false),
    (Type::Date, "year", Type::Int, false),
    (Type::Date, "month", Type::Int, false),
    (Type::Date, "day", Type::Int, false),
    (Type::Date, "clone", Type::Date, false),
    (Type::Date, "to_string", Type::String, false),
    (Type::DateTime, "year", Type::Int, false),
    (Type::DateTime, "month", Type::Int, false),
    (Type::DateTime, "day", Type::Int, false),
    (Type::DateTime, "hour", Type::Int, false),
    (Type::DateTime, "minute", Type::Int, false),
    (Type::DateTime, "second", Type::Int, false),
    (Type::DateTime, "clone", Type::DateTime, false),
    (Type::DateTime, "to_string", Type::String, false),
];

/// How a `Date` or a `DateTime` is made: the name of each static function, its number of `Int`s.
pub fn date_forms(t: &Type) -> &'static [(&'static str, usize)] {
    match t {
        Type::Date => &[("year(…) month(…) day(…)", 3)],
        _ => &[("year(…) month(…) day(…) hour(…) minute(…)", 5), ("year(…) month(…) day(…) hour(…) minute(…) second(…)", 6)],
    }
}

/// Year, month, day (then hour, minute, second): does this day exist? The same rules as the
/// checks `kir.rs` emits (`date_made`).
pub fn date_exists(v: &[i64]) -> bool {
    let (y, m, d) = (v[0], v[1], v[2]);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    (1..=9999).contains(&y) && (1..=12).contains(&m) && (1..=days[m as usize - 1]).contains(&d)
}

pub fn time_exists(v: &[i64]) -> bool {
    v.len() < 5 || ((0..=23).contains(&v[3]) && (0..=59).contains(&v[4]) && v.get(5).is_none_or(|s| (0..=59).contains(s)))
}

/// The methods of a `List`, a `Map` and the `String` methods of `string_method`, for the messages and the editor.
pub const LIST_METHODS: &[&str] = &["length", "add", "remove", "each", "filter", "map", "any", "all", "fold", "sorted", "join", "clone"];
pub const MAP_METHODS: &[&str] = &["length", "contains", "remove", "keys", "values", "clone"];
pub const SET_METHODS: &[&str] = &["length", "add", "contains", "remove", "clone"];
pub const STRING_METHODS: &[&str] =
    &["trim", "contains", "starts_with", "ends_with", "replace", "split", "words", "chars", "to_upper", "to_lower", "find", "repeat"];

/// `` `length`, `add` ``.
fn quoted(names: &[&str]) -> String {
    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}

pub const NUMBER_METHODS: &[&str] =
    &["abs", "min", "max", "pow", "sqrt", "floor", "ceil", "round", "sin", "cos", "tan", "asin", "acos", "atan", "ln", "log10", "exp"];

/// `n.abs`, `a.max(b)`, `x.sqrt`, `x.round` on an `Int` or a `Float` (Part 4, "Standard library
/// v2"): their parameters, their result. `floor`, `ceil` and `round` give an `Int`.
pub fn number_method(t: &Type, name: &str) -> Option<(Vec<Type>, Type)> {
    let (int, float) = (|| Type::Int, || Type::Float);
    Some(match (t, name) {
        (Type::Int | Type::Float, "abs") => (vec![], t.clone()),
        (Type::Int | Type::Float, "min" | "max") => (vec![t.clone()], t.clone()),
        (Type::Int | Type::Float, "pow") => (vec![int()], t.clone()),
        (Type::Float, "sqrt") => (vec![], float()),
        // `libm`'s (musl's, in Rust): the same digits in run and build (`shared::math`).
        (Type::Float, "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "ln" | "log10" | "exp") => (vec![], float()),
        (Type::Float, "floor" | "ceil" | "round") => (vec![], int()),
        _ => return None,
    })
}

/// The String methods with arguments or a List result (PLAN 2.10): their parameters, their result.
/// Each one is a `kir::StrOp` (`kir::string_op`).
pub fn string_method(name: &str) -> Option<(Vec<Type>, Type)> {
    let s = || Type::String;
    let list = |t| Type::List(Box::new(t));
    Some(match name {
        "trim" => (vec![], s()),
        "contains" | "starts_with" | "ends_with" => (vec![s()], Type::Bool),
        "replace" => (vec![s(), s()], s()),
        "split" => (vec![s()], list(s())),
        "words" => (vec![], list(s())),
        "chars" => (vec![], list(Type::Char)),
        "to_upper" | "to_lower" => (vec![], s()),
        "find" => (vec![s()], Type::Optional(Box::new(Type::Int))),
        "repeat" => (vec![Type::Int], s()),
        _ => return None,
    })
}

/// The built-in interfaces each primitive follows (for `<T: Comparable>`, PLAN Part 4, "J5").
fn primitive_follows(t: &Type, interface: &str) -> bool {
    match interface {
        "Equal" | "Display" => {
            matches!(t, Type::Int | Type::Float | Type::Bool | Type::Char | Type::String | Type::BigInt | Type::Date | Type::DateTime)
        }
        "Comparable" => matches!(t, Type::Int | Type::Float | Type::Char | Type::String | Type::BigInt | Type::Date | Type::DateTime),
        _ => false,
    }
}

/// Copied when given away (like an `Int`), instead of moved (like a `String`).
pub fn is_copy(t: &Type) -> bool {
    match t {
        Type::String | Type::BigInt | Type::List(_) | Type::Map(..) | Type::Set(_) => false,
        Type::Tuple(ts) => ts.iter().all(is_copy),
        Type::Struct(d) => d.copy,
        // An enum is a value like an `Int` when nothing in it is owned: `Light.Red`, `Shape.Circle(2.0)`.
        Type::Enum(d) => d.variants.iter().all(|(_, fs)| fs.iter().all(|(_, t)| is_copy(t))),
        Type::Optional(t) => is_copy(t),
        Type::Result(t, e) => is_copy(t) && is_copy(e),
        // Checked once for every real type: as if it were owned.
        Type::Param(_) => false,
        Type::Any(_) => false,
        // Borrowed for the call it is given to, never owned.
        Type::Block(..) => true,
        Type::Int | Type::Float | Type::Bool | Type::Char | Type::Date | Type::DateTime | Type::Unit | Type::Error => true,
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
    /// Type name → the interfaces it follows (`struct Dog : Equal, Display`).
    pub follows: HashMap<String, Vec<String>>,
    /// By the `Expr::id` of a call to a generic function: the type given for each of its
    /// type parameters, in order (`larger(3, 4)` → `[Int]`).
    pub type_args: HashMap<usize, Vec<Type>>,
    /// By `Expr::id`: the interface `I` when this value goes where an `any I` is expected.
    pub boxes: Vec<Option<String>>,
    /// Interface → the types an `any` of it may hold; a value's number is its place here.
    pub impls: HashMap<String, Vec<Type>>,
    /// `Animal.sound` → the types of its parameters (without `self`) and of its result.
    pub methods: HashMap<String, (Vec<Type>, Type)>,
    /// For the editor (`kova lsp`): where each expression is written, by `Expr::id`.
    pub spans: Vec<Span>,
    /// For the editor: each name met (variable, function, type, field, method), where it is
    /// written, where it is declared, and what hovering it shows.
    pub seen: Vec<Seen>,
    /// For the editor: the variables declared without their type written anywhere
    /// (`for score in scores`, `Circle(r) =>`), shown in grey after their name.
    pub hints: Vec<(Span, Type)>,
    /// For the editor: each `match` on an enum, an optional or a result (from its subject to its
    /// `}`), with the cases it doesn't handle as they are written: `Circle(radius)`, `none`.
    pub matches: Vec<(Span, Vec<String>)>,
    /// The `Expr::id`s that call a block: `test(x)`, and `body` alone (a block without parameters).
    pub invoked: HashSet<usize>,
    /// By the `Expr::id` of a block: the variables from around it that it uses (filled by
    /// `ownership.rs`); the KIR gives them to the block in its box.
    pub captures: HashMap<usize, Vec<String>>,
}

/// A name met while checking: `rex` at `span`, declared at `decl`, shown as `rex: Dog`.
pub struct Seen {
    pub span: Span,
    pub decl: Span,
    pub text: String,
}

impl Info {
    /// Does the struct or enum `t` follow `interface`?
    pub fn follows(&self, t: &Type, interface: &str) -> bool {
        self.follows.get(&t.text()).is_some_and(|is| is.iter().any(|i| i == interface))
    }
}

impl Info {
    /// The type of the value `e` gives where it goes: a `T?` if its `T` is wrapped.
    pub fn given(&self, e: &Expr) -> Type {
        let mut t = self.types[e.id].clone();
        if let Some(i) = &self.boxes[e.id] {
            t = Type::Any(i.clone());
        }
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
    /// `<T: Comparable>`: one `Type::Param` each, deduced at each call.
    generics: Vec<Rc<ParamDef>>,
}

impl Signature {
    fn new(receiver: Option<Mode>, params: Vec<(Mode, Type)>, ret: Type) -> Signature {
        Signature { receiver, params, ret, generics: vec![] }
    }

    fn substitute(&self, s: &[(String, Type)]) -> Signature {
        let params = self.params.iter().map(|(m, t)| (*m, substitute(t, s))).collect();
        Signature { receiver: self.receiver, params, ret: substitute(&self.ret, s), generics: self.generics.clone() }
    }
}

/// Does `t` contain the type parameter `name`?
fn mentions(t: &Type, name: &str) -> bool {
    match t {
        Type::Param(p) => p.name == name,
        Type::Tuple(ts) => ts.iter().any(|t| mentions(t, name)),
        Type::Optional(t) | Type::List(t) | Type::Set(t) => mentions(t, name),
        Type::Result(a, b) | Type::Map(a, b) => mentions(a, name) || mentions(b, name),
        Type::Struct(d) => d.args.iter().any(|t| mentions(t, name)),
        Type::Block(ps, r) => ps.iter().any(|t| mentions(t, name)) || mentions(r, name),
        _ => false,
    }
}

/// Deduces the type parameters of `param` from the type `given` of an argument:
/// `List<T>` and `List<Int>` → `T` = `Int`. The first argument giving a parameter decides it.
fn deduce(param: &Type, given: &Type, generics: &[Rc<ParamDef>], found: &mut Vec<(String, Type)>) {
    match (param, given) {
        (_, Type::Error) => {}
        (Type::Param(p), t) if generics.iter().any(|g| g.name == p.name) => {
            if !found.iter().any(|(n, _)| *n == p.name) {
                found.push((p.name.clone(), t.clone()));
            }
        }
        (Type::Tuple(ps), Type::Tuple(ts)) => ps.iter().zip(ts).for_each(|(p, t)| deduce(p, t, generics, found)),
        (Type::Optional(p), Type::Optional(t)) | (Type::List(p), Type::List(t)) | (Type::Set(p), Type::Set(t)) => deduce(p, t, generics, found),
        (Type::Result(p, q), Type::Result(t, u)) | (Type::Map(p, q), Type::Map(t, u)) => {
            deduce(p, t, generics, found);
            deduce(q, u, generics, found);
        }
        (Type::Struct(p), Type::Struct(t)) if p.name == t.name => {
            p.args.iter().zip(&t.args).for_each(|(p, t)| deduce(p, t, generics, found));
        }
        (Type::Block(ps, p), Type::Block(ts, t)) => {
            ps.iter().zip(ts).for_each(|(p, t)| deduce(p, t, generics, found));
            deduce(p, t, generics, found);
        }
        // A plain value given for `T?` is wrapped, as everywhere: `or_else(5, 0)` gives `T = Int`.
        (Type::Optional(p), t) => deduce(p, t, generics, found),
        _ => {}
    }
}

/// A function's heading as written: `fn less(self, other: Self) -> Bool`.
pub fn heading_text(f: &Function) -> String {
    let mode = |m: Mode| match m {
        Mode::Read => "",
        Mode::Var => "var ",
        Mode::Move => "move ",
    };
    let mut text = String::from("fn");
    for (i, (part, params)) in f.parts().into_iter().enumerate() {
        let mut written: Vec<String> = f.receiver.iter().filter(|_| i == 0).map(|&m| format!("{}self", mode(m))).collect();
        written.extend(params.iter().map(|p| format!("{}: {}{}", p.name.text, mode(p.mode), p.ty.text())));
        text += &format!(" {part}");
        if i == 0 {
            text += &generics_text(f);
        }
        if !written.is_empty() {
            text += &format!("({})", written.join(", "));
        }
    }
    let ret = f.ret.as_ref().map_or(String::new(), |t| format!(" -> {}", t.text()));
    text + &ret
}

/// `<T: Comparable + Display, K>`, or nothing when the function has no type parameters.
pub fn generics_text(f: &Function) -> String {
    if f.generics.is_empty() {
        return String::new();
    }
    let one = |g: &Generic| {
        let bounds: Vec<&str> = g.bounds.iter().map(|b| b.text.as_str()).collect();
        if bounds.is_empty() { g.name.text.clone() } else { format!("{}: {}", g.name.text, bounds.join(" + ")) }
    };
    format!("<{}>", f.generics.iter().map(one).collect::<Vec<_>>().join(", "))
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
        // `ages["Rex"]` is not a place: reading it gives a copy (a Map has no slot per key).
        ExprKind::Index(x, i) if matches!(types[x.id], Type::Map(..)) || i.range().is_some() => None,
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
    follows: HashMap<String, Vec<String>>,
    type_args: HashMap<usize, Vec<Type>>,
    /// Each `any Animal` written, checked at the end (see `any_type`).
    any_checks: Vec<Name>,
    /// By `Expr::id`: the interface of the `any` this value becomes (`Info::boxes`).
    boxes: Vec<Option<String>>,
    /// `Sorted<Dog>`: where, the struct, its parameter, the type given, the interface to follow.
    bounds: Vec<(Span, String, String, Type, String)>,
    /// One table per open block, from outside to inside.
    scopes: Vec<HashMap<String, Variable>>,
    ret: Type,
    /// The type of `result`: `Some` only inside an `ensure`.
    result: Option<Type>,
    loops: usize,
    /// In a contract: `x?` cannot leave the function from there.
    in_contract: bool,
    /// Number of open blocks (`{ x: Int => … }`): `return`, `break`, `continue` and `x?` cannot leave them.
    in_block: usize,
    invoked: HashSet<usize>,
    types: Vec<Type>,
    wraps: Vec<bool>,
    /// `File::private`, without the methods a type writes for an interface (always visible).
    private: HashMap<String, Span>,
    /// The `ages["Rex"]` targets met (`target`), to refuse `ages["Rex"] += 1`.
    map_entries: Vec<Span>,
    /// Name → where it is declared and what hovering it shows (`places`).
    places: HashMap<String, (Span, String)>,
    spans: Vec<Span>,
    seen: Vec<Seen>,
    hints: Vec<(Span, Type)>,
    matches: Vec<(Span, Vec<String>)>,
    diags: Vec<Diag>,
}

/// Where each name of the program is declared, and what hovering it shows: functions
/// (`geometry.distance`), types, fields, methods and variants (`geometry.Point.x`).
fn places(f: &File) -> HashMap<String, (Span, String)> {
    let mut p = HashMap::new();
    let field = |owner: &str, x: &Param| (method_name(owner, &x.name.text), (x.name.span, format!("{}: {}", x.name.text, x.ty_text())));
    for func in &f.functions {
        p.insert(func.name.text.clone(), (func.name.span, heading_text(func)));
    }
    for s in &f.structs {
        let word = if s.variants.is_some() { "enum" } else { "struct" };
        p.insert(s.name.text.clone(), (s.name.span, format!("{word} {}", s.name.text)));
        p.extend(s.fields.iter().map(|x| field(&s.name.text, x)));
        for v in s.variants.iter().flatten() {
            let fields: Vec<String> = v.fields.iter().map(|x| format!("{}: {}", x.name.text, x.ty_text())).collect();
            let shown = if fields.is_empty() { v.name.text.clone() } else { format!("{}({})", v.name.text, fields.join(", ")) };
            p.insert(method_name(&s.name.text, &v.name.text), (v.name.span, shown));
        }
        for m in &s.methods {
            p.insert(method_name(&s.name.text, &m.name.text), (m.name.span, heading_text(m)));
        }
    }
    for i in f.interfaces.iter().filter(|i| !i.builtin) {
        p.insert(i.name.text.clone(), (i.name.span, format!("interface {}", i.name.text)));
        for (m, _) in &i.methods {
            p.insert(method_name(&i.name.text, &m.name.text), (m.name.span, heading_text(m)));
        }
    }
    p
}

pub fn check(f: &File, nb_exprs: usize, src: &str) -> (Info, Vec<Diag>) {
    let mut c = Checker {
        src,
        functions: HashMap::new(),
        structs: Structs::new(),
        interfaces: HashMap::new(),
        defaults: vec![],
        follows: HashMap::new(),
        type_args: HashMap::new(),
        bounds: vec![],
        any_checks: vec![],
        boxes: vec![None; nb_exprs],
        scopes: vec![],
        ret: Type::Unit,
        result: None,
        loops: 0,
        in_contract: false,
        in_block: 0,
        invoked: HashSet::new(),
        types: vec![Type::Error; nb_exprs],
        wraps: vec![false; nb_exprs],
        private: f.private.iter().cloned().collect(),
        map_entries: vec![],
        places: places(f),
        spans: vec![Span::new(0, 0); nb_exprs],
        seen: vec![],
        hints: vec![],
        matches: vec![],
        diags: vec![],
    };
    // Hovering a declaration shows it too.
    c.seen = c.places.values().map(|(decl, text)| Seen { span: *decl, decl: *decl, text: text.clone() }).collect();
    // A method that an interface promises is reached through the interface: it stays visible.
    for s in &f.structs {
        let promised = s.interfaces.iter().filter_map(|n| f.interfaces.iter().find(|i| i.name.text == n.text));
        for (m, _) in promised.flat_map(|i| &i.methods) {
            c.private.remove(&method_name(&s.name.text, &m.name.text));
        }
    }

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
    let read_file = Signature::new(None, vec![(Mode::Read, string())], Type::Result(Box::new(string()), Box::new(io_error())));
    c.functions.insert("read_file".into(), read_file);
    let io = |ok: Type| Type::Result(Box::new(ok), Box::new(io_error()));
    let paths = |n: usize| vec![(Mode::Read, string()); n];
    for (name, n, result) in [
        ("write_file", 2, io(Type::Unit)),
        ("append_file", 2, io(Type::Unit)),
        ("exists", 1, Type::Bool),
        ("list_dir", 1, io(Type::List(Box::new(string())))),
        ("make_dir", 1, io(Type::Unit)),
        ("remove_file", 1, io(Type::Unit)),
    ] {
        c.functions.insert(name.into(), Signature::new(None, paths(n), result));
    }
    c.functions.insert("now".into(), Signature::new(None, vec![], Type::Float));
    c.functions.insert("sleep".into(), Signature::new(None, vec![(Mode::Read, Type::Float)], Type::Unit));
    c.functions.insert("read_line".into(), Signature::new(None, vec![], Type::Optional(Box::new(string()))));
    c.functions.insert("args".into(), Signature::new(None, vec![], Type::List(Box::new(string()))));
    c.functions.insert("exit".into(), Signature::new(None, vec![(Mode::Read, Type::Int)], Type::Unit));
    let mut decls: HashMap<&str, usize> = HashMap::new();
    for (i, s) in f.structs.iter().enumerate() {
        let name = s.name.text.as_str();
        if primitive(name).is_some() || c.structs.contains_key(name) || matches!(name, "Self" | "List" | "Map") || builtin_interface(f, name) {
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
        if it.builtin {
            c.interfaces.insert(name.to_string(), i);
        } else if primitive(name).is_some() || c.structs.contains_key(name) || name == "Self" || builtin_interface(f, name) {
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
        let generics = c.open_generics(func, true);
        for t in func.params.iter().map(|p| &p.ty).chain(&func.ret) {
            c.report_unknown(t);
        }
        let params: Vec<(Mode, Type)> = func.params.iter().map(|p| (p.mode, type_of(&p.ty, &c.structs))).collect();
        let ret = func.ret.as_ref().map_or(Type::Unit, |t| type_of(t, &c.structs));
        // Each type parameter is deduced from the arguments: it must appear in a parameter.
        for (g, written) in generics.iter().zip(&func.generics) {
            if !params.iter().any(|(_, t)| mentions(t, &g.name)) {
                c.diags.push(Diag::new(written.name.span, t!("generic_unused", name = g.name)).help(t!("generic_unused_help", name = g.name)));
            }
        }
        c.close_generics(func);
        let name = func.name.text.as_str();
        if BUILTINS.contains(&name) || LIBRARY.iter().any(|(n, _)| *n == name) {
            c.error(func.name.span, t!("builtin_function", name = name));
        } else if c.functions.contains_key(name) {
            c.error(func.name.span, t!("function_exists", name = name));
        } else {
            c.functions.insert(name.to_string(), Signature { receiver: None, params, ret, generics });
        }
    }
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) != Some(&i) {
            continue;
        }
        let mut seen: HashMap<&str, Span> = HashMap::new();
        let me = c.structs[&s.name.text].clone();
        // `struct Stack<T>`: its methods use `T`, given at each call by the receiver's type.
        let (args, generics) = struct_params(&me);
        let saved = c.open_struct(&args);
        c.structs.insert("Self".into(), me);
        for m in &s.methods {
            c.open_generics(m, false);
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
                let generics = generics.clone();
                c.functions.insert(method_name(&s.name.text, name), Signature { receiver: m.receiver, params, ret, generics });
            }
        }
        c.structs.remove("Self");
        c.close_struct(saved);
    }
    // The interfaces' methods, `Animal.sound`, with `Self` standing for the type that follows it.
    for (i, it) in f.interfaces.iter().enumerate() {
        if c.interfaces.get(it.name.text.as_str()) != Some(&i) {
            continue;
        }
        c.structs.insert("Self".into(), self_param(&it.name.text));
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for (m, _) in &it.methods {
            c.open_generics(m, false);
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
            c.functions.insert(method_name(&it.name.text, name), Signature::new(m.receiver, params, ret));
        }
        c.structs.remove("Self");
    }
    // `struct Dog : Animal`: Dog writes each method Animal asks for, or takes its default one.
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) != Some(&i) {
            continue;
        }
        match (s.generics.first(), s.interfaces.first()) {
            (Some(_), Some(it)) => c.error(it.span, t!("generic_struct_interface_later", name = s.name.text, interface = it.text)),
            _ => c.conforms(f, s),
        }
    }
    // `struct Point : Equal` without `equals`: every field must have `=` (known once all conform).
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) == Some(&i) {
            c.auto_equal(s);
        }
    }

    // What a file shows must not mention what it hides: since types are written, another file
    // could not hold the value (`let a: geometry.Hidden := …` is refused).
    let public = |n: &str| !f.private.iter().any(|(p, _)| p == n);
    for func in f.functions.iter().filter(|g| public(&g.name.text)) {
        for t in func.params.iter().map(|p| &p.ty).chain(&func.ret) {
            c.exposes(&func.name.text, t);
        }
    }
    for s in f.structs.iter().filter(|s| public(&s.name.text)) {
        for p in s.fields.iter().filter(|p| public(&method_name(&s.name.text, &p.name.text))) {
            c.exposes(&method_name(&s.name.text, &p.name.text), &p.ty);
        }
        for m in s.methods.iter().filter(|m| public(&method_name(&s.name.text, &m.name.text))) {
            for t in m.params.iter().map(|p| &p.ty).chain(&m.ret) {
                c.exposes(&method_name(&s.name.text, &m.name.text), t);
            }
        }
    }

    // 2. `fn main`: without parameters or result. Required only to run or build (`main.rs`):
    // a module opened alone in the editor, or given to `kova test`, needs none.
    if let Some(m) = f.functions.iter().find(|f| f.name.text == "main")
        && (!m.params.is_empty() || m.ret.is_some())
    {
        c.error(m.name.span, t!("main_signature"));
    }

    // A translation file contains only `lang` blocks.
    if !f.langs.is_empty() {
        let others = f.header.iter().map(|h| h.open).chain(f.functions.iter().map(|f| f.name.span));
        let others = others.chain(f.structs.iter().map(|s| s.name.span)).chain(f.tests.iter().map(|t| t.span));
        let others = others.chain(f.interfaces.iter().filter(|i| !i.builtin).map(|i| i.name.span));
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
            let saved = c.open_struct(&struct_params(&ty).0);
            c.structs.insert("Self".into(), ty.clone());
            for m in &s.methods {
                c.body(m, Some(&ty));
            }
            c.structs.remove("Self");
            c.close_struct(saved);
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

    for (span, ty, name, t, bound) in std::mem::take(&mut c.bounds) {
        if !c.satisfies(&t, &bound) {
            c.diags.push(
                Diag::new(span, t!("generic_bound", function = ty, name = name, type = t.text(), bound = bound))
                    .help(bound_help(&t, &bound)),
            );
        }
    }

    // `any Comparable` cannot be: `less` takes another `Self`, of a real type nobody knows.
    for n in std::mem::take(&mut c.any_checks) {
        let it = &f.interfaces[c.interfaces[&n.text]];
        let uses_self = |m: &Function| {
            let sig = &c.functions[&method_name(&it.name.text, &m.name.text)];
            sig.params.iter().map(|(_, t)| t).chain([&sig.ret]).any(|t| mentions(t, "Self"))
        };
        if let Some((m, _)) = it.methods.iter().find(|(m, _)| c.functions.contains_key(&method_name(&it.name.text, &m.name.text)) && uses_self(m)) {
            c.diags.push(Diag::new(n.span, t!("any_self", name = n.text, method = m.name.text)).help(t!("any_self_help", name = n.text)));
        }
    }
    // The types an `any I` may hold, numbered in this order (the number is kept in its box).
    let mut impls: HashMap<String, Vec<Type>> = HashMap::new();
    for (i, s) in f.structs.iter().enumerate() {
        if decls.get(s.name.text.as_str()) != Some(&i) || !s.generics.is_empty() {
            continue;
        }
        let t = &c.structs[&s.name.text];
        for it in c.follows.get(&s.name.text).into_iter().flatten() {
            impls.entry(it.clone()).or_default().push(t.clone());
        }
    }
    // Each interface method, for the KIR's dispatch: its parameters' types and its result.
    let methods = (f.interfaces.iter())
        .flat_map(|it| it.methods.iter().map(move |(m, _)| method_name(&it.name.text, &m.name.text)))
        .filter_map(|q| Some((q.clone(), c.functions.get(&q).map(|s| (s.params.iter().map(|(_, t)| t.clone()).collect(), s.ret.clone()))?)))
        .collect();

    let moves = vec![false; c.types.len()];
    let (defaults, follows, type_args) = (c.defaults, c.follows, c.type_args);
    let boxes = c.boxes;
    let (spans, seen, hints, matches, invoked) = (c.spans, c.seen, c.hints, c.matches, c.invoked);
    let info = Info {
        types: c.types,
        moves,
        wraps: c.wraps,
        boxes,
        structs: c.structs,
        defaults,
        follows,
        type_args,
        impls,
        methods,
        spans,
        seen,
        hints,
        matches,
        invoked,
        captures: HashMap::new(),
    };
    (info, c.diags)
}

/// The help for a `Stack<Int>` that does not follow `interface`: it cannot yet (J5, step 4).
fn later(t: &Type, interface: &str) -> Option<String> {
    match t {
        Type::Struct(d) if !d.args.is_empty() => Some(t!("generic_interface_later_help", name = d.name, interface = interface)),
        _ => None,
    }
}

/// How `t` can follow `bound`: `struct Dog : Comparable`, or `<T: Comparable>` for a `T`.
fn bound_help(t: &Type, bound: &str) -> String {
    match t {
        Type::Param(p) => t!("param_bound_help", type = p.name, bound = bound),
        t => later(t, bound).unwrap_or_else(|| t!("generic_bound_help", type = t.text(), bound = bound)),
    }
}

/// The type parameters of `struct Stack<T>`, as types and as definitions.
fn struct_params(t: &Type) -> (Vec<Type>, Vec<Rc<ParamDef>>) {
    let Type::Struct(d) = t else { return (vec![], vec![]) };
    let defs = d.args.iter().filter_map(|a| if let Type::Param(p) = a { Some(p.clone()) } else { None }).collect();
    (d.args.clone(), defs)
}

/// `Equal`, `Comparable`, `Display` (`parser::PRELUDE`).
fn builtin_interface(f: &File, name: &str) -> bool {
    f.interfaces.iter().any(|i| i.builtin && i.name.text == name)
}

/// `Self` inside `interface Animal`.
fn self_param(interface: &str) -> Type {
    Type::Param(Rc::new(ParamDef { name: "Self".into(), bounds: vec![interface.to_string()] }))
}

pub fn primitive(name: &str) -> Option<Type> {
    Some(match name {
        "Int" => Type::Int,
        "Float" => Type::Float,
        "Bool" => Type::Bool,
        "Char" => Type::Char,
        "String" => Type::String,
        "BigInt" => Type::BigInt,
        "Date" => Type::Date,
        "DateTime" => Type::DateTime,
        _ => return None,
    })
}

/// `12`, `-3`: an `Int` written directly.
fn int_written(e: &Expr) -> Option<i64> {
    match &e.kind {
        ExprKind::Int(v) => i64::try_from(*v).ok(),
        ExprKind::Unary(UnOp::Neg, x) => int_written(x).map(|v| -v),
        _ => None,
    }
}

/// `Nothing`, written as the success of a result.
fn is_nothing(t: &TypeAst) -> bool {
    matches!(t, TypeAst::Name(n) if n.text == "Nothing")
}

/// The written type; `Error` if it contains an unknown name.
pub fn type_of(t: &TypeAst, structs: &Structs) -> Type {
    match t {
        TypeAst::Any(n) => Type::Any(n.text.clone()),
        // `Stack` alone is not a type: `Stack<Int>` is.
        TypeAst::Name(n) => primitive(&n.text)
            .or_else(|| structs.get(&n.text).filter(|t| !is_template(t, &n.text)).cloned())
            .unwrap_or(Type::Error),
        TypeAst::Tuple(ts) => {
            let ts: Vec<Type> = ts.iter().map(|t| type_of(t, structs)).collect();
            if ts.contains(&Type::Error) { Type::Error } else { Type::Tuple(ts) }
        }
        TypeAst::Generic(n, ts) if n.text == "List" && ts.len() == 1 => match type_of(&ts[0], structs) {
            Type::Error => Type::Error,
            t => Type::List(Box::new(t)),
        },
        TypeAst::Generic(n, ts) if n.text == "Map" && ts.len() == 2 => match (type_of(&ts[0], structs), type_of(&ts[1], structs)) {
            (Type::Error, _) | (_, Type::Error) => Type::Error,
            (k, v) => Type::Map(Box::new(k), Box::new(v)),
        },
        TypeAst::Generic(n, ts) if n.text == "Set" && ts.len() == 1 => match type_of(&ts[0], structs) {
            Type::Error => Type::Error,
            t => Type::Set(Box::new(t)),
        },
        TypeAst::Generic(n, ts) => {
            let types: Vec<Type> = ts.iter().map(|t| type_of(t, structs)).collect();
            if types.contains(&Type::Error) {
                return Type::Error;
            }
            structs.get(&n.text).and_then(|t| instance(t, &types)).unwrap_or(Type::Error)
        }
        TypeAst::Optional(t) => match type_of(t, structs) {
            Type::Error => Type::Error,
            t => Type::Optional(Box::new(t)),
        },
        // `Nothing or IoError`: a success without a value, only written there.
        TypeAst::Result(t, e) => match (if is_nothing(t) { Type::Unit } else { type_of(t, structs) }, type_of(e, structs)) {
            (Type::Error, _) | (_, Type::Error) => Type::Error,
            (t, e) => Type::Result(Box::new(t), Box::new(e)),
        },
        TypeAst::Block(ps, r) => {
            let ps: Vec<Type> = ps.iter().map(|t| type_of(t, structs)).collect();
            let r = r.as_ref().map_or(Type::Unit, |r| type_of(r, structs));
            if ps.contains(&Type::Error) || r == Type::Error { Type::Error } else { Type::Block(ps, Box::new(r)) }
        }
    }
}

/// Does the block always end with a `return`?
fn always_returns(b: &Block) -> bool {
    b.stmts.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If { then, otherwise: Some(otherwise), .. } => always_returns(then) && always_returns(otherwise),
        Stmt::Match(m) => !m.arms.is_empty() && m.arms.iter().all(|a| always_returns(&a.body)),
        // `exit(1)` stops the program: nothing after it runs.
        Stmt::Expr(Expr { kind: ExprKind::Call { name, .. }, .. }) => name.text == "exit",
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
            self.hidden(&n.text, n.span);
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
            self.follows.entry(s.name.text.clone()).or_default().push(n.text.clone());
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
                } else if it.builtin && it.name.text == "Equal" {
                    // Compared field by field: checked by `auto_equal`.
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

    /// `struct Point : Equal` without `equals`: `=` compares the fields (an enum's variant, then
    /// its fields) one by one, so each of them must have `=` too.
    fn auto_equal(&mut self, s: &Struct) {
        let Some(n) = s.interfaces.iter().find(|n| n.text == "Equal") else { return };
        if !self.follows(&s.name.text, "Equal") || self.functions.contains_key(&method_name(&s.name.text, "equals")) {
            return;
        }
        let fields = s.fields.iter().chain(s.variants.iter().flatten().flat_map(|v| &v.fields));
        for p in fields {
            let t = type_of(&p.ty, &self.structs);
            if !self.equatable(&t) {
                self.diags.push(
                    Diag::new(n.span, t!("equal_field", type = s.name.text, field = p.name.text, field_type = t.text()))
                        .help(t!("equal_field_help", type = s.name.text)),
                );
            }
        }
    }

    fn follows(&self, name: &str, interface: &str) -> bool {
        self.follows.get(name).is_some_and(|is| is.iter().any(|i| i == interface))
    }

    /// Does `t` follow `interface` (`Equal`, `Comparable`, `Display`): written for a struct or an
    /// enum, given for the primitives, promised for a `Self`.
    fn has(&self, t: &Type, interface: &str) -> bool {
        match t {
            Type::Struct(d) => self.follows(&d.name, interface),
            Type::Enum(d) => self.follows(&d.name, interface),
            Type::Param(p) => p.bounds.iter().any(|b| b == interface),
            Type::Any(i) => i == interface,
            Type::Error => true,
            t => primitive_follows(t, interface),
        }
    }

    /// Can two values of type `t` be compared with `=`?
    fn equatable(&self, t: &Type) -> bool {
        match t {
            Type::Tuple(ts) => ts.iter().all(|t| self.equatable(t)),
            Type::Optional(t) => self.equatable(t),
            Type::Result(a, b) => self.equatable(a) && self.equatable(b),
            // `light = Light.Red`: an enum of copied values is compared like an `Int` (J4).
            Type::Enum(d) => is_copy(t) || self.follows(&d.name, "Equal"),
            t => self.has(t, "Equal"),
        }
    }

    /// The struct or enum that keeps `t` from having `=`: `Dog` in `(Int, Dog?)`.
    fn missing_equal(&self, t: &Type) -> Option<Type> {
        match t {
            Type::Tuple(ts) => ts.iter().find_map(|t| self.missing_equal(t)),
            Type::Optional(t) => self.missing_equal(t),
            Type::Result(a, b) => self.missing_equal(a).or_else(|| self.missing_equal(b)),
            t if self.equatable(t) => None,
            t => Some(t.clone()),
        }
    }

    /// Makes the type parameters of `func` known as types (`T` in `fn larger<T: Comparable>`)
    /// until `close_generics`, after checking them; reported once, when `report` is set.
    fn open_generics(&mut self, func: &Function, report: bool) -> Vec<Rc<ParamDef>> {
        if !report {
            if let Some(g) = func.generics.first() {
                self.error(g.name.span, t!("generic_method_later"));
            }
            return vec![];
        }
        let made = self.params(&func.generics, &|_| false);
        for p in &made {
            self.structs.insert(p.name.clone(), Type::Param(p.clone()));
        }
        made
    }

    /// The type parameters as written, checked (`taken`: also refused as a name).
    fn params(&mut self, generics: &[Generic], taken: &dyn Fn(&str) -> bool) -> Vec<Rc<ParamDef>> {
        let mut made: Vec<Rc<ParamDef>> = vec![];
        for g in generics {
            let name = g.name.text.as_str();
            let other = matches!(self.structs.get(name), Some(t) if !matches!(t, Type::Param(_)));
            if primitive(name).is_some() || other || taken(name) || self.interfaces.contains_key(name) || name == "Self" {
                self.error(g.name.span, t!("generic_name_taken", name = name));
                continue;
            }
            if made.iter().any(|m| m.name == name) {
                self.error(g.name.span, t!("generic_twice", name = name));
                continue;
            }
            for b in &g.bounds {
                if !self.interfaces.contains_key(&b.text) {
                    let msg = if self.structs.contains_key(&b.text) || primitive(&b.text).is_some() {
                        t!("bound_not_interface", name = b.text)
                    } else {
                        t!("interface_unknown", name = b.text)
                    };
                    self.error(b.span, msg);
                }
            }
            let bounds = g.bounds.iter().map(|b| b.text.clone()).collect();
            made.push(Rc::new(ParamDef { name: name.to_string(), bounds }));
        }
        made
    }

    /// While checking `struct Stack<T>` and its methods, `T` is a type; gives back what the
    /// names stood for before (a struct being built inside another one's fields).
    fn open_struct(&mut self, args: &[Type]) -> Vec<(String, Option<Type>)> {
        let params = args.iter().filter_map(|a| match a {
            Type::Param(p) => Some(p.clone()),
            _ => None,
        });
        params.map(|p| (p.name.clone(), self.structs.insert(p.name.clone(), Type::Param(p)))).collect()
    }

    fn close_struct(&mut self, saved: Vec<(String, Option<Type>)>) {
        for (name, before) in saved {
            match before {
                Some(t) => self.structs.insert(name, t),
                None => self.structs.remove(&name),
            };
        }
    }

    fn close_generics(&mut self, func: &Function) {
        for g in &func.generics {
            if matches!(self.structs.get(&g.name.text), Some(Type::Param(_))) {
                self.structs.remove(&g.name.text);
            }
        }
    }

    /// Does the type given for a type parameter keep the promise `bound`?
    fn satisfies(&self, t: &Type, bound: &str) -> bool {
        if bound == "Equal" { self.equatable(t) } else { self.has(t, bound) }
    }

    fn body(&mut self, func: &Function, owner: Option<&Type>) {
        // Checked once, with each `T` known only by what it promises.
        let opened = !func.generics.is_empty() && owner.is_none();
        if opened {
            let generics = self.functions.get(&func.name.text).map(|s| s.generics.clone()).unwrap_or_default();
            for p in generics {
                self.structs.insert(p.name.clone(), Type::Param(p));
            }
        }
        self.body_inner(func, owner);
        if opened {
            self.close_generics(func);
        }
    }

    fn body_inner(&mut self, func: &Function, owner: Option<&Type>) {
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
            let ty = type_of(&p.ty, &self.structs);
            if matches!(ty, Type::Block(..)) && p.mode != Mode::Read {
                self.error(p.name.span, t!("block_param_mode", name = p.name.text, mode = p.mode.word()));
            }
            self.mention_variable(&p.name.text, &ty, p.name.span, p.name.span);
            params.insert(p.name.text.clone(), Variable { ty, kind, span: p.name.span });
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
        let args: Vec<Type> = self.params(&s.generics, &|n| decls.contains_key(n)).into_iter().map(Type::Param).collect();
        let saved = self.open_struct(&args);
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
                Type::Struct(Rc::new(StructDef { name: s.name.text.clone(), args, fields, copy: s.copy }))
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
        self.close_struct(saved);
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
            // Refused by the parser: a block type is only a parameter's.
            TypeAst::Block(..) => Type::Error,
            TypeAst::Any(n) => {
                self.any_type(n);
                Type::Any(n.text.clone())
            }
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
                let inner: Vec<Type> = ts.iter().map(|t| self.field_type(f, decls, t, open)).collect();
                let template = match decls.get(n.text.as_str()) {
                    Some(&j) => match self.define(f, decls, j, open) {
                        None => return self.contains_itself(n),
                        t => t,
                    },
                    None => self.structs.get(&n.text).cloned(),
                };
                self.generic(n, template.as_ref(), &inner)
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
                    Some(Type::Struct(d)) if !d.args.is_empty() => {
                        self.generic_without_types(n);
                        Type::Error
                    }
                    Some(t) => t,
                    None => self.contains_itself(n),
                }
            }
        }
    }

    fn report_unknown(&mut self, t: &TypeAst) {
        if let TypeAst::Name(n) | TypeAst::Generic(n, _) | TypeAst::Any(n) = t {
            self.hidden(&n.text, n.span);
        }
        match t {
            TypeAst::Any(n) => self.any_type(n),
            TypeAst::Name(n) if n.text == "Self" && !self.structs.contains_key("Self") => {
                self.diags.push(Diag::new(n.span, t!("self_type_outside")).help(t!("self_type_outside_help")));
            }
            TypeAst::Name(n) if self.interfaces.contains_key(&n.text) => self.diags.push(
                Diag::new(n.span, t!("interface_as_type", name = n.text)).help(t!("interface_as_type_help", name = n.text)),
            ),
            TypeAst::Name(n) if self.structs.get(&n.text).is_some_and(|t| is_template(t, &n.text)) => {
                self.generic_without_types(n)
            }
            TypeAst::Name(n) if n.text == "Nothing" => {
                self.diags.push(Diag::new(n.span, t!("nothing_only_in_result")).help(t!("nothing_only_in_result_help")));
            }
            TypeAst::Name(n) if type_of(t, &self.structs) == Type::Error => self.error(n.span, t!("type_unknown", name = n.text)),
            TypeAst::Name(_) => {}
            TypeAst::Tuple(ts) => ts.iter().for_each(|t| self.report_unknown(t)),
            TypeAst::Generic(n, ts) => {
                ts.iter().for_each(|t| self.report_unknown(t));
                let types: Vec<Type> = ts.iter().map(|t| type_of(t, &self.structs)).collect();
                let template = self.structs.get(&n.text).cloned();
                self.generic(n, template.as_ref(), &types);
            }
            TypeAst::Optional(t) => self.report_unknown(t),
            TypeAst::Result(t, e) => {
                if !is_nothing(t) {
                    self.report_unknown(t);
                }
                self.report_unknown(e);
            }
            TypeAst::Block(ps, r) => ps.iter().chain(r.as_deref()).for_each(|t| self.report_unknown(t)),
        }
    }

    /// `List<Int>`, or `Stack<Int>` from `template`, the struct `Stack<T>` (`None`: unknown name).
    fn generic(&mut self, n: &Name, template: Option<&Type>, types: &[Type]) -> Type {
        if n.text == "List" {
            return match types {
                [Type::Error] => Type::Error,
                [t] => Type::List(Box::new(t.clone())),
                _ => {
                    self.error(n.span, t!("list_one_type", count = types.len()));
                    Type::Error
                }
            };
        }
        if n.text == "Set" {
            return match types {
                [Type::Error] => Type::Error,
                // Found by `=`, like a Map's keys.
                [t] if *t == Type::Float || !self.equatable(t) => {
                    self.diags.push(Diag::new(n.span, t!("set_element", type = t.text())).help(t!("set_element_help")));
                    Type::Error
                }
                [t] => Type::Set(Box::new(t.clone())),
                _ => {
                    self.error(n.span, t!("set_one_type", count = types.len()));
                    Type::Error
                }
            };
        }
        if n.text == "Map" {
            return match types {
                [Type::Error, _] | [_, Type::Error] => Type::Error,
                // A key is found by `=`; a Float key would get lost (`NaN` is not equal to itself).
                [k, _] if *k == Type::Float || !self.equatable(k) => {
                    self.diags.push(Diag::new(n.span, t!("map_key", type = k.text())).help(t!("map_key_help")));
                    Type::Error
                }
                [k, v] => Type::Map(Box::new(k.clone()), Box::new(v.clone())),
                _ => {
                    self.error(n.span, t!("map_two_types", count = types.len()));
                    Type::Error
                }
            };
        }
        let d = match template {
            Some(t @ Type::Struct(d)) if is_template(t, &n.text) => d.clone(),
            Some(Type::Param(_)) | None => {
                self.error(n.span, t!("type_unknown", name = n.text));
                return Type::Error;
            }
            Some(_) => {
                self.diags.push(Diag::new(n.span, t!("not_generic", name = n.text)).help(t!("not_generic_help", name = n.text)));
                return Type::Error;
            }
        };
        if d.args.len() != types.len() {
            let example = format!("{}<{}>", d.name, d.args.iter().map(Type::text).collect::<Vec<_>>().join(", "));
            self.error(n.span, t!("generic_count", name = n.text, expected = d.args.len(), given = types.len(), example = example));
            return Type::Error;
        }
        if types.contains(&Type::Error) {
            return Type::Error;
        }
        // Checked at the end: what each type follows is only known once all are read.
        for (p, t) in d.args.iter().zip(types) {
            if let Type::Param(p) = p {
                for b in &p.bounds {
                    self.bounds.push((n.span, n.text.clone(), p.name.clone(), t.clone(), b.clone()));
                }
            }
        }
        instance(&Type::Struct(d), types).unwrap()
    }

    fn contains_itself(&mut self, n: &Name) -> Type {
        self.diags.push(Diag::new(n.span, t!("struct_contains_itself", name = n.text)).help(t!("struct_contains_itself_help", name = n.text)));
        Type::Error
    }

    /// `any Animal`: `Animal` must be an interface; whether its methods allow it is checked
    /// at the end, once they are all read (`any_checks`).
    fn any_type(&mut self, n: &Name) {
        if self.interfaces.contains_key(&n.text) {
            self.any_checks.push(n.clone());
            return;
        }
        let msg = if self.structs.contains_key(&n.text) || primitive(&n.text).is_some() {
            t!("any_not_interface", name = n.text)
        } else {
            t!("interface_unknown", name = n.text)
        };
        self.error(n.span, msg);
    }

    /// A value going where an `any Animal` is expected: a struct or an enum following `Animal`.
    fn boxable(&self, t: &Type, interface: &str) -> bool {
        match t {
            Type::Struct(d) => d.args.is_empty() && self.follows(&d.name, interface),
            Type::Enum(d) => self.follows(&d.name, interface),
            _ => false,
        }
    }

    /// `Stack` written without its types.
    fn generic_without_types(&mut self, n: &Name) {
        let Some(Type::Struct(d)) = self.structs.get(&n.text) else { return };
        let example = format!("{}<{}>", d.name, d.args.iter().map(Type::text).collect::<Vec<_>>().join(", "));
        self.diags.push(Diag::new(n.span, t!("generic_struct_types", name = n.text)).help(t!("generic_struct_types_help", example = example)));
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
                // A text for people shows what the type chose to show (`to_string`), never its
                // inside by accident (PLAN Part 4, "J5").
                t if !self.has(&t, "Display") => {
                    let code = &self.src[e.span.start..e.span.end];
                    let help = match &t {
                        t if later(t, "Display").is_some() => later(t, "Display").unwrap(),
                        Type::Struct(d) => t!("interp_display_help", name = d.name, kind = "struct"),
                        Type::Enum(d) => t!("interp_display_help", name = d.name, kind = "enum"),
                        Type::Param(p) => t!("param_display_help", type = p.name),
                        Type::Any(i) => t!("any_display_help", code = code, interface = i),
                        _ => t!("interp_display_list_help", code = code),
                    };
                    self.diags.push(Diag::new(e.span, t!("interp_display", code = code, type = t.text())).help(help));
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
        self.mention_variable(&name.text, &ty, name.span, name.span);
        if matches!(kind, VarKind::Element | VarKind::Matched) {
            self.hints.push((name.span, ty.clone()));
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
        self.spans[e.id] = e.span;
        // `[]` where the written type is wrong: already reported.
        if let (ExprKind::List(elems), Type::Error) = (&e.kind, expected)
            && elems.is_empty()
        {
            return;
        }
        // `(1, none)` where an `(Int, String?)` is expected: each element against its type.
        if let (ExprKind::Tuple(elems), Type::Tuple(ts)) = (&e.kind, expected)
            && elems.len() == ts.len()
        {
            elems.iter().zip(ts).for_each(|(x, t)| self.expect(x, t));
            self.types[e.id] = expected.clone();
            return;
        }
        if let (ExprKind::Block { params, body }, Type::Block(ps, r)) = (&e.kind, expected) {
            self.types[e.id] = self.block_literal(params, body, Some(ps), Some(r));
            return;
        }
        // `repeat(n - 1, body)`: where a block is expected, its name gives it along without calling it.
        if let (ExprKind::Var(n), Type::Block(..)) = (&e.kind, expected)
            && matches!(self.lookup(n), Some(Variable { ty: Type::Block(..), .. }))
        {
            let t = self.passed(e);
            if t != *expected {
                self.mismatch(e, t, expected);
            }
            return;
        }
        // `[1, 2]` where a `List<Float>` is expected: each element is checked against `Float`; `[]` takes the type.
        if let (ExprKind::List(elems), Type::List(elem)) = (&e.kind, expected) {
            elems.iter().for_each(|x| self.expect(x, elem));
            self.types[e.id] = expected.clone();
            return;
        }
        // `[]` and `["Rex" := 3]` where a `Map<String, Int>` is expected.
        if let (ExprKind::List(elems), Type::Map(..)) = (&e.kind, expected)
            && elems.is_empty()
        {
            self.types[e.id] = expected.clone();
            return;
        }
        // `["a", "b"]` where a `Set<String>` is expected.
        if let (ExprKind::List(elems), Type::Set(t)) = (&e.kind, expected) {
            elems.iter().for_each(|x| self.expect(x, t));
            self.types[e.id] = expected.clone();
            return;
        }
        if let (ExprKind::MapLit(pairs), Type::Map(k, v)) = (&e.kind, expected) {
            for (key, value) in pairs {
                self.expect(key, k);
                self.expect(value, v);
            }
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
        // `ok` alone (an empty tuple, made by the parser) where `Nothing or E` is expected.
        if let (ExprKind::Tuple(elems), Type::Unit) = (&e.kind, expected)
            && elems.is_empty()
        {
            self.types[e.id] = Type::Unit;
            return;
        }
        if let (ExprKind::Outcome { ok, value }, Type::Result(t, er)) = (&e.kind, expected) {
            let alone = matches!(&value.kind, ExprKind::Tuple(elems) if elems.is_empty());
            if *ok && (**t == Type::Unit) != alone {
                if alone {
                    self.diags.push(Diag::new(e.span, t!("ok_needs_value", type = t.text())));
                } else {
                    self.expr(value);
                    self.diags.push(Diag::new(e.span, t!("ok_without_value", type = expected.text())).help(t!("ok_without_value_help")));
                }
                self.types[e.id] = expected.clone();
                return;
            }
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
            if matches!(e.kind, ExprKind::List(_) | ExprKind::MapLit(_) | ExprKind::Int(_) | ExprKind::Unary(UnOp::Neg, _)) {
                self.expect(e, inner);
                self.wraps[e.id] = true;
                return;
            }
            let t = self.expr(e);
            if t == **inner {
                self.wraps[e.id] = true;
            } else if let Type::Any(i) = &**inner
                && self.boxable(&t, i)
            {
                self.boxes[e.id] = Some(i.clone());
                self.wraps[e.id] = true;
            } else {
                self.mismatch(e, t, expected);
            }
            return;
        }
        // `let a: any Animal := rex`: `rex` goes in a box that remembers it is a `Dog`.
        if let Type::Any(i) = expected {
            let t = self.expr(e);
            if self.boxable(&t, i) {
                self.boxes[e.id] = Some(i.clone());
            } else if t != *expected && t != Type::Error {
                let mut d = Diag::new(e.span, t!("types_differ", expected = expected.text(), given = t.text()));
                if let Type::Param(p) = &t {
                    d = d.help(t!("any_param_later", type = p.name, interface = i));
                } else if matches!(t, Type::Struct(_) | Type::Enum(_)) && !matches!(&t, Type::Struct(s) if !s.args.is_empty()) {
                    d = d.help(t!("any_needs_follow", type = t.text(), interface = i));
                } else if !matches!(t, Type::Any(_)) {
                    d = d.help(t!("any_struct_only", interface = i));
                }
                self.diags.push(d);
            }
            return;
        }
        if *expected == Type::BigInt && self.as_big(e) {
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

    /// A number written directly, like `3` or `-2`, is a `BigInt` where one is expected
    /// (`let f: BigInt := 1`, `f * 2`), whatever its size. A variable still needs `.to_big`.
    fn as_big(&mut self, e: &Expr) -> bool {
        let inner = match &e.kind {
            ExprKind::Int(_) | ExprKind::BigInt(_) => None,
            ExprKind::Unary(UnOp::Neg, x) if matches!(x.kind, ExprKind::Int(_) | ExprKind::BigInt(_)) => Some(x.id),
            _ => return false,
        };
        self.types[e.id] = Type::BigInt;
        self.spans[e.id] = e.span;
        if let Some(id) = inner {
            self.types[id] = Type::BigInt;
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
        if matches!(b.kind, ExprKind::BigInt(_)) && self.as_big(a) {
            self.expr(b);
            return Type::BigInt;
        }
        let tb = self.expr(b);
        if tb == Type::Float && self.as_float(a) {
            return Type::Float;
        }
        if tb == Type::BigInt && self.as_big(a) {
            return Type::BigInt;
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
            } else if let (Type::Int, ExprKind::BigInt(_)) = (expected, &e.kind) {
                d = d.help(t!("big_literal_help"));
            } else if matches!((expected, &t), (Type::Int, Type::BigInt) | (Type::BigInt, Type::Int)) {
                d = d.help(t!("no_big_conversion"));
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
        // `` `Int`, `Float` or `BigInt` ``
        let mut names: Vec<String> = allowed.iter().map(|t| format!("`{}`", t.text())).collect();
        let last = names.pop().unwrap_or_default();
        let names = if names.is_empty() { last } else { format!("{} {} {last}", names.join(", "), t!("word_or")) };
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
                for (k, step) in fields.iter().enumerate() {
                    ty = match step {
                        Step::Index(i) => self.field(ty, *i, *span),
                        Step::Name(f) => self.named_field(ty, f),
                        // `ages["Rex"] := 3`: adds the key or replaces its value; only as a whole.
                        Step::At(i) if matches!(ty, Type::Map(..)) => {
                            let Type::Map(key, value) = ty else { unreachable!() };
                            self.expect(i, &key);
                            if k + 1 < fields.len() {
                                self.diags.push(Diag::new(*span, t!("map_entry_part")).help(t!("map_entry_part_help")));
                                return Type::Error;
                            }
                            self.map_entries.push(*span);
                            *value
                        }
                        // `l[0..2] := …`: a slice is a new List, not a part of `l`.
                        Step::At(i) if i.range().is_some() => {
                            self.diags.push(Diag::new(i.span, t!("slice_target")).help(t!("slice_target_help")));
                            return Type::Error;
                        }
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
    /// `typed`: the types of the arguments already computed (to deduce a generic's `T`).
    fn arguments(&mut self, name: &Name, args: &[Expr], params: &[(Mode, Type)], typed: &[Option<Type>]) {
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
            match typed.get(i).cloned().flatten() {
                // Already typed to deduce `T`: a `T` given for a `T?` still becomes one.
                Some(given) if matches!(t, Type::Optional(inner) if **inner == given) => self.wraps[value.id] = true,
                Some(given) => self.mismatch(value, given, t),
                None => self.expect(value, t),
            }
            self.types[a.id] = self.types[value.id].clone();
            let arg = &self.src[value.span.start..value.span.end];
            if *mode == Mode::Var && (self.wraps[value.id] || self.boxes[value.id].is_some()) {
                // The changed value comes back into the caller's variable: it must be a `T?` too.
                let (given, declared) = (self.types[value.id].text(), t.text());
                let msg = if self.wraps[value.id] {
                    t!("var_arg_optional", arg = arg, type = given, declared = declared)
                } else {
                    t!("var_arg_any", arg = arg, type = given, declared = declared)
                };
                self.error(value.span, msg);
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
                (Mode::Move, Mode::Read) if !is_copy(t) && place_root(value, &self.types).is_some() && !self.invoked.contains(&value.id) => self.diags.push(
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
        self.arguments(name, args, &sig.params, &[]);
        sig.ret
    }

    /// A method of `Stack<T>` called on a `Stack<Int>`: its `T` is `Int` (kept for the KIR copy).
    fn for_types(&mut self, id: usize, sig: Signature, types: Vec<Type>) -> Signature {
        if types.is_empty() {
            return sig;
        }
        let s: Vec<(String, Type)> = sig.generics.iter().map(|g| g.name.clone()).zip(types.iter().cloned()).collect();
        self.type_args.insert(id, types);
        sig.substitute(&s)
    }

    /// `l.length`, `l.add(x)`, `l.remove(i)` (PLAN 2.10); `None` for `clone` (shared with the other types).
    fn list_method(&mut self, receiver: &Expr, elem: &Type, name: &Name, args: &[Expr]) -> Option<Type> {
        let (params, result) = match name.text.as_str() {
            // `l.sorted`: in the order of `<`.
            "sorted" if args.is_empty() => {
                if !self.has(elem, "Comparable") {
                    let example = format!("l.sorted {{ a: {t}, b: {t} => … }}", t = elem.text());
                    self.diags.push(Diag::new(name.span, t!("sorted_needs_order", type = elem.text())).help(t!("sorted_needs_order_help", example = example)));
                }
                return Some(Type::List(Box::new(elem.clone())));
            }
            "each" | "filter" | "map" | "any" | "all" | "fold" | "sorted" => return Some(self.block_method(elem, name, args)),
            "length" => (vec![], Type::Int),
            "add" => (vec![(Mode::Move, elem.clone())], Type::Unit),
            "remove" => (vec![(Mode::Read, Type::Int)], elem.clone()),
            // `words.join(", ")`: only Strings are joined (a number has no single text).
            "join" if *elem == Type::String => (vec![(Mode::Read, Type::String)], Type::String),
            "join" => {
                self.error(name.span, t!("join_needs_strings", type = Type::List(Box::new(elem.clone())).text()));
                args.iter().for_each(|a| {
                    self.expr(a);
                });
                return Some(Type::Error);
            }
            "clone" => return None,
            _ => {
                let t = Type::List(Box::new(elem.clone())).text();
                self.diags.push(
                    Diag::new(name.span, t!("no_method", type = t, name = name.text))
                        .help(t!("methods_of", type = t, methods = quoted(LIST_METHODS))),
                );
                args.iter().for_each(|a| {
                    self.expr(a);
                });
                return Some(Type::Error);
            }
        };
        if !matches!(name.text.as_str(), "length" | "join") {
            self.changes(receiver, t!("var_method_temporary", name = name.text));
        }
        if params.is_empty() && !args.is_empty() {
            self.error(name.span, t!("no_arguments", name = name.text));
        } else if !params.is_empty() {
            self.arguments(name, args, &params, &[]);
        }
        Some(result)
    }

    /// `l.each { x: Int => … }`, `l.filter`, `l.map`, `l.any`, `l.all`, `l.fold(start) { acc: A, x: T => … }`.
    fn block_method(&mut self, elem: &Type, name: &Name, args: &[Expr]) -> Type {
        let fold = name.text == "fold";
        let expected = if fold { 2 } else { 1 };
        let block = args.last().filter(|b| {
            args.len() == expected && (matches!(b.kind, ExprKind::Block { .. }) || matches!(&b.kind, ExprKind::Var(n) if matches!(self.lookup(n), Some(Variable { ty: Type::Block(..), .. }))))
        });
        let Some(block) = block else {
            let example = match name.text.as_str() {
                "fold" => "l.fold(0) { sum: Int, x: Int => sum + x }".to_string(),
                "sorted" => "l.sorted { a: Int, b: Int => a > b }".to_string(),
                n => format!("l.{n} {{ x: Int => … }}"),
            };
            self.error(name.span, t!("block_expected", name = name.text, example = example));
            args.iter().for_each(|a| {
                self.expr(a);
            });
            return Type::Error;
        };
        let acc = if fold { self.expr(&args[0]) } else { Type::Unit };
        let (params_wanted, result) = match name.text.as_str() {
            "each" => (vec![elem.clone()], Some(Type::Unit)),
            "filter" | "any" | "all" => (vec![elem.clone()], Some(Type::Bool)),
            "map" => (vec![elem.clone()], None),
            // `{ a: T, b: T => … }`: `true` when `a` comes before `b`.
            "sorted" => (vec![elem.clone(), elem.clone()], Some(Type::Bool)),
            _ => (vec![acc.clone(), elem.clone()], Some(acc.clone())),
        };
        let t = match &block.kind {
            ExprKind::Block { params, body } => {
                self.spans[block.id] = block.span;
                let t = self.block_literal(params, body, Some(&params_wanted), result.as_ref());
                self.types[block.id] = t.clone();
                t
            }
            // `l.each(f)`: a block parameter given along.
            _ => {
                let t = self.passed(block);
                let r = match (&result, &t) {
                    (Some(r), _) => r.clone(),
                    (None, Type::Block(_, r)) => (**r).clone(),
                    _ => Type::Error,
                };
                let wanted = Type::Block(params_wanted, Box::new(r));
                if t != wanted {
                    self.mismatch(block, t.clone(), &wanted);
                }
                t
            }
        };
        match (name.text.as_str(), t) {
            ("each", _) => Type::Unit,
            ("filter" | "sorted", _) => Type::List(Box::new(elem.clone())),
            ("any" | "all", _) => Type::Bool,
            ("map", Type::Block(_, r)) if *r != Type::Error => Type::List(r),
            ("map", _) => Type::Error,
            _ => acc,
        }
    }

    /// Checks a block `{ x: Int => … }` against the parameters and the result expected (if known):
    /// its type, `{Int -> Bool}`. Its parameters are borrowed, like a function's.
    fn block_literal(&mut self, params: &[Param], body: &Block, wanted: Option<&[Type]>, result: Option<&Type>) -> Type {
        let written: Vec<Type> = params.iter().map(|p| self.written_type(&p.ty)).collect();
        if let Some(ps) = wanted {
            let expected = Type::Block(ps.to_vec(), Box::new(result.cloned().unwrap_or(Type::Unit))).text();
            if ps.len() != written.len() {
                let span = params.first().map_or(body.open, |p| p.name.span.to(params.last().unwrap().name.span));
                self.diags.push(Diag::new(span, t!("block_params", type = expected)));
            } else {
                for ((p, w), t) in params.iter().zip(&written).zip(ps) {
                    if w != t && *w != Type::Error && *t != Type::Error {
                        let msg = t!("block_param_type", name = p.name.text, wanted = t.text(), type = expected);
                        self.error(p.name.span, msg);
                    }
                }
            }
        }
        self.scopes.push(HashMap::new());
        for (p, t) in params.iter().zip(&written) {
            self.declare(&p.name, t.clone(), VarKind::Param);
        }
        self.in_block += 1;
        // A loop around the block is not the block's: `break` cannot leave it.
        let loops = std::mem::take(&mut self.loops);
        let r = match result {
            Some(Type::Unit) => {
                self.block(body, vec![]);
                Type::Unit
            }
            r if matches!(body.stmts.last(), Some(Stmt::Expr(_) | Stmt::If { otherwise: Some(_), .. } | Stmt::Match(_))) => {
                self.block_value(body, r)
            }
            _ => {
                self.block(body, vec![]);
                self.diags.push(Diag::new(body.close, t!("block_needs_value")).help(t!("block_needs_value_help")));
                Type::Error
            }
        };
        self.in_block -= 1;
        self.loops = loops;
        self.scopes.pop();
        Type::Block(written, Box::new(r))
    }

    /// A block given along by its name (`repeat(n - 1, body)`, not called), or a block written there.
    fn passed(&mut self, value: &Expr) -> Type {
        let ExprKind::Var(n) = &value.kind else { return self.expr(value) };
        let Some(Variable { ty: ty @ Type::Block(..), span, .. }) = self.lookup(n) else { return self.expr(value) };
        let (ty, decl) = (ty.clone(), *span);
        self.mention_variable(n, &ty, value.span, decl);
        self.spans[value.id] = value.span;
        self.types[value.id] = ty.clone();
        ty
    }

    /// `return`, `break`, `continue` or `x?` inside a block.
    fn block_exit(&mut self, span: Span, word: &str) {
        self.diags.push(Diag::new(span, t!("block_exit", word = word)).help(t!("block_exit_help")));
    }

    /// `ages.length`, `ages.contains("Rex")`, `ages.remove("Rex")` (an `Int?`: `none` if absent).
    /// `None` for `.clone`, common to every type.
    fn map_method(&mut self, receiver: &Expr, t: &Type, k: &Type, v: &Type, name: &Name, args: &[Expr]) -> Option<Type> {
        let (params, result) = match name.text.as_str() {
            "length" => (vec![], Type::Int),
            "contains" => (vec![(Mode::Read, k.clone())], Type::Bool),
            "remove" => (vec![(Mode::Read, k.clone())], Type::Optional(Box::new(v.clone()))),
            // Copies, in the order the keys were added.
            "keys" => (vec![], Type::List(Box::new(k.clone()))),
            "values" => (vec![], Type::List(Box::new(v.clone()))),
            "clone" => return None,
            _ => {
                self.diags.push(
                    Diag::new(name.span, t!("no_method", type = t.text(), name = name.text))
                        .help(t!("methods_of", type = t.text(), methods = quoted(MAP_METHODS))),
                );
                args.iter().for_each(|a| {
                    self.expr(a);
                });
                return Some(Type::Error);
            }
        };
        if name.text == "remove" {
            self.changes(receiver, t!("var_method_temporary", name = name.text));
        }
        if params.is_empty() && !args.is_empty() {
            self.error(name.span, t!("no_arguments", name = name.text));
        } else if !params.is_empty() {
            self.arguments(name, args, &params, &[]);
        }
        Some(result)
    }

    /// `seen.add("Rex")` (nothing if already there), `seen.contains("Rex")`, `seen.remove("Rex")`
    /// (`true` if it was there). `None` for `.clone`, common to every type.
    fn set_method(&mut self, receiver: &Expr, t: &Type, elem: &Type, name: &Name, args: &[Expr]) -> Option<Type> {
        let (params, result) = match name.text.as_str() {
            "length" => (vec![], Type::Int),
            "add" => (vec![(Mode::Move, elem.clone())], Type::Unit),
            "contains" => (vec![(Mode::Read, elem.clone())], Type::Bool),
            "remove" => (vec![(Mode::Read, elem.clone())], Type::Bool),
            "clone" => return None,
            _ => {
                self.diags.push(
                    Diag::new(name.span, t!("no_method", type = t.text(), name = name.text))
                        .help(t!("methods_of", type = t.text(), methods = quoted(SET_METHODS))),
                );
                args.iter().for_each(|a| {
                    self.expr(a);
                });
                return Some(Type::Error);
            }
        };
        if matches!(name.text.as_str(), "add" | "remove") {
            self.changes(receiver, t!("var_method_temporary", name = name.text));
        }
        if params.is_empty() && !args.is_empty() {
            self.error(name.span, t!("no_arguments", name = name.text));
        } else if !params.is_empty() {
            self.arguments(name, args, &params, &[]);
        }
        Some(result)
    }

    /// `Date.year(2026) month(10) day(23)`, each part an `Int`; a day that cannot exist is an error
    /// when its numbers are written directly, else a runtime error (`kir.rs` `date_made`).
    fn date_made(&mut self, t: Type, ty: &Name, name: &Name, args: &[Expr]) -> Type {
        let forms = date_forms(&t);
        let Some(&(_, n)) = forms.iter().find(|(f, _)| *f == name.text) else {
            let help = if t == Type::Date { t!("date_made_help") } else { t!("datetime_made_help") };
            self.diags.push(Diag::new(name.span, t!("no_static", type = ty.text, name = name.text)).help(help));
            return self.unchecked(args);
        };
        self.arguments(name, args, &vec![(Mode::Read, Type::Int); n], &[]);
        let numbers: Option<Vec<i64>> = args.iter().map(int_written).collect();
        if let Some(v) = numbers
            && v.len() == n
        {
            let span = args[0].span.to(args[n - 1].span);
            if !date_exists(&v) {
                self.error(span, t!("fault_bad_date"));
            } else if !time_exists(&v) {
                self.error(span, t!("fault_bad_time"));
            }
        }
        t
    }

    /// `Dog.create("Rex")`: a struct's function without `self`.
    fn static_call(&mut self, id: usize, ty: &Name, types: &[TypeAst], name: &Name, args: &[Expr]) -> Type {
        // `Float.pi`, like Smalltalk's `Float pi`.
        if ty.text == "Float" && name.text == "pi" {
            if !args.is_empty() {
                self.error(name.span, t!("no_arguments", name = name.text));
            }
            return Type::Float;
        }
        if let t @ (Type::Date | Type::DateTime) = primitive(&ty.text).unwrap_or(Type::Error) {
            return self.date_made(t, ty, name, args);
        }
        if let kind @ ("List" | "Map" | "Set") = ty.text.as_str() {
            let help = match kind {
                "List" => t!("list_made_help"),
                "Map" => t!("map_made_help"),
                _ => t!("set_made_help"),
            };
            self.diags.push(Diag::new(ty.span, t!("no_static", type = kind, name = name.text)).help(help));
            return self.unchecked(args);
        }
        // `Stack<Int>.empty`: the function of `Stack<T>` with `T` = `Int`.
        let mut given = vec![];
        if !types.is_empty() || self.structs.get(&ty.text).is_some_and(|t| is_template(t, &ty.text)) {
            match self.with_types(ty, types) {
                Type::Struct(d) => given = d.args.clone(),
                _ => {
                    self.unchecked(args);
                    return Type::Error;
                }
            }
        }
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
                Some(sig) => {
                    if !self.hidden(&ty.text, ty.span) {
                        self.hidden(&method_name(&ty.text, &name.text), name.span);
                    }
                    Some(sig)
                }
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
        let sig = self.for_types(id, sig, given);
        self.arguments(name, args, &sig.params, &[]);
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

    /// `t` is written in what `owner` shows to the other files: it must not name a private type.
    fn exposes(&mut self, owner: &str, t: &TypeAst) {
        match t {
            TypeAst::Name(n) | TypeAst::Any(n) | TypeAst::Generic(n, _) if self.private.contains_key(&n.text) => {
                self.diags.push(
                    Diag::new(n.span, t!("private_in_public", name = owner, type = n.text))
                        .help(t!("private_in_public_help", name = owner, type = n.text)),
                );
            }
            TypeAst::Name(_) | TypeAst::Any(_) => {}
            TypeAst::Generic(_, ts) | TypeAst::Tuple(ts) => ts.iter().for_each(|t| self.exposes(owner, t)),
            TypeAst::Optional(t) => self.exposes(owner, t),
            TypeAst::Result(t, e) => {
                self.exposes(owner, t);
                self.exposes(owner, e);
            }
            TypeAst::Block(ps, r) => ps.iter().chain(r.as_deref()).for_each(|t| self.exposes(owner, t)),
        }
    }

    /// `name` (`geometry.helper`, `geometry.Point.x`) is private in another file than `span`'s.
    fn hidden(&mut self, name: &str, span: Span) -> bool {
        self.mention(name, span);
        let Some(decl) = self.private.get(name) else { return false };
        if file_number(self.src, decl.start) == file_number(self.src, span.start) {
            return false;
        }
        let file = format!("{}.kova", name.split('.').next().unwrap_or(name));
        self.diags.push(Diag::new(span, t!("private_name", name = name, file = file)).help(t!("private_name_help", file = file)));
        true
    }

    /// Notes for the editor that `name` (`geometry.Point.x`) is written at `span`.
    fn mention(&mut self, name: &str, span: Span) {
        // `rex.introduce`: a default method is declared in the interface (`Animal.introduce`).
        let place = self.places.get(name).or_else(|| {
            let (owner, method) = name.rsplit_once('.')?;
            let interfaces = self.follows.get(owner)?;
            interfaces.iter().find_map(|i| self.places.get(&method_name(i, method)))
        });
        if let Some((decl, text)) = place {
            self.seen.push(Seen { span, decl: *decl, text: text.clone() });
        }
    }

    /// Notes for the editor that the variable `name` of type `ty`, declared at `decl`, is written at `span`.
    fn mention_variable(&mut self, name: &str, ty: &Type, span: Span, decl: Span) {
        self.seen.push(Seen { span, decl, text: format!("{name}: {}", ty.text()) });
    }

    /// The type of `x.name` when `x` has type `ty` and `name` is not a method.
    fn named_field(&mut self, ty: Type, name: &Name) -> Type {
        match &ty {
            Type::Struct(d) => match d.field(&name.text) {
                Some((_, t)) => {
                    self.hidden(&method_name(&ty.owner(), &name.text), name.span);
                    t.clone()
                }
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
        let prefix = method_name(&t.owner(), "");
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
            // `let rex := Dog { ... }`, `let c := Shape.Circle(2.0)`, `let d := Date.year(2026) month(10) day(23)`:
            // the type is written on the right (PLAN 2.5).
            Stmt::Declare { mutable, pattern, ty: None, value }
                if matches!(&value.kind, ExprKind::StructLit { .. } | ExprKind::Variant { .. })
                    || matches!(&value.kind, ExprKind::Static { ty, .. } if matches!(ty.text.as_str(), "Date" | "DateTime")) =>
            {
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
                let mark = self.map_entries.len();
                let ty = self.target(target, &mut vec![]);
                // `counts[w] += 1`: the key may be missing, its value is an `Int?`.
                if op.is_some() && self.map_entries.len() > mark {
                    let text = &self.src[target.span().start..target.span().end];
                    self.diags.push(Diag::new(target.span(), t!("map_entry_op", target = text)).help(t!("map_entry_op_help", target = text)));
                    self.expect(value, &ty);
                    return;
                }
                let concat = *op == Some(BinOp::Plus) && ty == Type::String;
                if op.is_some() && !concat && !matches!(ty, Type::Int | Type::Float | Type::BigInt | Type::Error) {
                    let text = &self.src[target.span().start..target.span().end];
                    self.error(target.span(), t!("not_numeric", target = text));
                }
                self.expect(value, &ty);
            }
            Stmt::Return { span, .. } if self.in_block > 0 => self.block_exit(*span, "return"),
            Stmt::Break(span) if self.in_block > 0 && self.loops == 0 => self.block_exit(*span, "break"),
            Stmt::Continue(span) if self.in_block > 0 && self.loops == 0 => self.block_exit(*span, "continue"),
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
            Stmt::Each { name, value: Some(value), list, body } => {
                let (k, v) = match self.expr(list) {
                    Type::Map(k, v) => (*k, *v),
                    Type::Error => (Type::Error, Type::Error),
                    t => {
                        self.error(value.span, t!("for_two_names", type = t.text()));
                        (Type::Error, Type::Error)
                    }
                };
                self.loops += 1;
                self.block(body, vec![(name, k, VarKind::Element), (value, v, VarKind::Element)]);
                self.loops -= 1;
            }
            Stmt::Each { name, value: None, list, body } => {
                let elem = match self.expr(list) {
                    Type::List(elem) | Type::Set(elem) => *elem,
                    Type::Map(..) => {
                        self.diags.push(Diag::new(name.span, t!("for_map_one_name")).help(t!("for_map_one_name_help", name = name.text)));
                        Type::Error
                    }
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
                // `save(text)?`: the call is what counts.
                let called = if let ExprKind::Try(x) = &e.kind { x } else { e };
                let call = match &called.kind {
                    ExprKind::Call { .. } | ExprKind::Static { .. } => true,
                    ExprKind::Var(_) => self.invoked.contains(&called.id),
                    ExprKind::Method { receiver, name, .. } => match &self.types[receiver.id] {
                        t @ (Type::Struct(_) | Type::Enum(_)) => self.functions.contains_key(&method_name(&t.owner(), &name.text)),
                        Type::List(_) => matches!(name.text.as_str(), "add" | "remove" | "each"),
                        Type::Map(..) => name.text == "remove",
                        Type::Set(_) => matches!(name.text.as_str(), "add" | "remove"),
                        Type::Param(p) => p.bounds.iter().any(|b| self.functions.contains_key(&method_name(b, &name.text))),
                        // An unknown method is already reported.
                        Type::Any(_) => true,
                        _ => false,
                    },
                    _ => false,
                };
                // `write_file(p, text)` alone: its failure would go unseen.
                if call && matches!(self.types[e.id], Type::Result(..)) {
                    let code = &self.src[e.span.start..e.span.end];
                    self.diags.push(Diag::new(e.span, t!("result_ignored")).help(t!("result_ignored_help", code = code)));
                }
                // `s.push(3)`: the unknown method is already reported.
                if !call && self.types[e.id] != Type::Error {
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
        self.spans[e.id] = e.span;
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
            ExprKind::BigInt(_) => Type::BigInt,
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
                Some(v) => {
                    let (ty, decl) = (v.ty.clone(), v.span);
                    self.mention_variable(name, &ty, e.span, decl);
                    match ty {
                        // `body` alone calls the block, like a function without parameters (Eiffel).
                        Type::Block(ps, r) if ps.is_empty() => {
                            self.invoked.insert(e.id);
                            *r
                        }
                        Type::Block(..) => {
                            self.error(e.span, t!("block_needs_args", name = name));
                            Type::Error
                        }
                        ty => ty,
                    }
                }
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
            ExprKind::MapLit(pairs) => {
                let (k, v) = (self.expr(&pairs[0].0), self.expr(&pairs[0].1));
                for (key, value) in &pairs[1..] {
                    self.expect(key, &k);
                    self.expect(value, &v);
                }
                if k == Type::Error || v == Type::Error {
                    return Type::Error;
                }
                let n = Name { text: "Map".into(), span: e.span };
                self.generic(&n, None, &[k, v])
            }
            ExprKind::New(ty) => {
                let t = self.written_type(ty);
                if !matches!(t, Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Error) {
                    self.error(e.span, t!("new_not_generic", type = t.text()));
                    return Type::Error;
                }
                t
            }
            ExprKind::Index(x, i) => {
                let t = self.expr(x);
                // `ages["Rex"]`: an `Int?`, `none` if the key is not there.
                if let Type::Map(k, v) = t {
                    self.expect(i, &k);
                    return Type::Optional(v);
                }
                // `scores[1..3]`: a new List; `name[0..3]`: a new String.
                if let Some((_, a, b)) = i.range() {
                    self.expect(a, &Type::Int);
                    self.expect(b, &Type::Int);
                    return match t {
                        Type::List(_) | Type::String | Type::Error => t,
                        t => {
                            self.error(e.span, t!("index_not_list", type = t.text()));
                            Type::Error
                        }
                    };
                }
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
            ExprKind::StructLit { ty, types, fields } => self.struct_literal(ty, types, fields),
            ExprKind::Call { name, args } => self.call(e.id, name, args),
            ExprKind::Static { ty, types, name, args } => self.static_call(e.id, ty, types, name, args),
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
            ExprKind::Try(x) if self.in_block > 0 => {
                self.expr(x);
                self.block_exit(e.span, "?");
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
            ExprKind::Block { params, body } => self.block_literal(params, body, None, None),
            ExprKind::Method { receiver, name, args } => {
                let t = self.expr(receiver);
                if matches!(t, Type::Struct(_) | Type::Enum(_))
                    && let Some(sig) = self.functions.get(&method_name(&t.owner(), &name.text)).cloned()
                {
                    let given = if let Type::Struct(d) = &t { d.args.clone() } else { vec![] };
                    let sig = self.for_types(e.id, sig, given);
                    self.hidden(&method_name(&t.owner(), &name.text), name.span);
                    return self.method_call(receiver, &t.text(), name, args, sig);
                }
                // `a.sound` on an `any Animal`: the method of the real type, chosen at run time.
                if let Type::Any(i) = &t
                    && let Some(sig) = self.functions.get(&method_name(i, &name.text)).cloned()
                {
                    if sig.receiver == Some(Mode::Move) {
                        self.error(name.span, t!("any_move_self_later", name = name.text));
                    }
                    return self.method_call(receiver, i, name, args, sig);
                }
                if let Type::List(elem) = &t
                    && let Some(r) = self.list_method(receiver, elem, name, args)
                {
                    return r;
                }
                if let Type::Map(k, v) = &t
                    && let Some(r) = self.map_method(receiver, &t, k, v, name, args)
                {
                    return r;
                }
                if let Type::Set(elem) = &t
                    && let Some(r) = self.set_method(receiver, &t, elem, name, args)
                {
                    return r;
                }
                // `self.sound` in a default method: what the interface promises.
                if let Type::Param(p) = &t
                    && let Some((b, sig)) =
                        p.bounds.iter().find_map(|b| Some((b.clone(), self.functions.get(&method_name(b, &name.text))?.clone())))
                {
                    // `Self` in the interface is the receiver's type (`T` in `a.equals(b)`).
                    let sig = sig.substitute(&[("Self".to_string(), t.clone())]);
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
                    && matches!(t, Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Param(_) | Type::Any(_))
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
                    self.hidden(&method_name(&t.owner(), &name.text), name.span);
                    if !args.is_empty() {
                        self.error(name.span, t!("no_arguments", name = name.text));
                    }
                    return ft.clone();
                }
                if t == Type::Float && name.text == "pow" && args.len() == 1 {
                    // `x.pow(2)`: an `Int`, squared again and again; `x.pow(0.5)`: a `Float`,
                    // `libm`'s `pow` (`shared::math`).
                    let given = self.expr(&args[0]);
                    let param = if given == Type::Float { Type::Float } else { Type::Int };
                    self.arguments(name, args, &[(Mode::Read, param)], &[Some(given)]);
                    return Type::Float;
                }
                if let Some((params, result)) = number_method(&t, &name.text) {
                    let params: Vec<(Mode, Type)> = params.into_iter().map(|p| (Mode::Read, p)).collect();
                    if params.is_empty() && !args.is_empty() {
                        self.error(name.span, t!("no_arguments", name = name.text));
                    } else if !params.is_empty() {
                        self.arguments(name, args, &params, &[]);
                    }
                    return result;
                }
                if t == Type::String
                    && let Some((params, result)) = string_method(&name.text)
                {
                    let params: Vec<(Mode, Type)> = params.into_iter().map(|p| (Mode::Read, p)).collect();
                    self.arguments(name, args, &params, &[]);
                    return result;
                }
                let Some((_, _, result, maybe)) = METHODS.iter().find(|(r, n, ..)| *r == t && *n == name.text) else {
                    let mut known: Vec<String> =
                        METHODS.iter().filter(|(r, ..)| *r == t).map(|(_, n, ..)| format!("`{n}`")).collect();
                    if t == Type::String {
                        known.extend(STRING_METHODS.iter().map(|n| format!("`{n}`")));
                    }
                    known.extend(NUMBER_METHODS.iter().filter(|n| number_method(&t, n).is_some()).map(|n| format!("`{n}`")));
                    let mut d = Diag::new(name.span, t!("no_method", type = t.text(), name = name.text));
                    if t == Type::Int && number_method(&Type::Float, &name.text).is_some() {
                        let code = format!("{}.to_float.{}", &self.src[receiver.span.start..receiver.span.end], name.text);
                        d = d.help(t!("float_method", name = name.text, code = code));
                    } else if !known.is_empty() {
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
                self.one_of(x, &[Type::Int, Type::Float, Type::BigInt], "`-`")
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
                        match self.missing_equal(&ta) {
                            None => {}
                            Some(Type::Unit) => self.error(a.span, t!("expression_nothing")),
                            Some(Type::List(_)) => self.error(e.span, t!("list_compare", op = op.text())),
                            Some(Type::Map(..)) => self.error(e.span, t!("map_compare", op = op.text())),
                            Some(Type::Set(_)) => self.error(e.span, t!("set_compare", op = op.text())),
                            Some(Type::Param(p)) => self.error(e.span, t!("param_compare", op = op.text(), type = p.name)),
                            Some(culprit @ Type::Any(_)) => self.diags.push(
                                Diag::new(e.span, t!("compare_needs_equal", op = op.text(), type = ta.text())).help(t!("any_compare_help", type = culprit.text())),
                            ),
                            Some(culprit) => {
                                let (name, kind) = match &culprit {
                                    Type::Enum(d) => (d.name.clone(), "enum"),
                                    t => (t.text(), "struct"),
                                };
                                let help = later(&culprit, "Equal").unwrap_or_else(|| t!("compare_needs_equal_help", name = name, kind = kind));
                                self.diags.push(Diag::new(e.span, t!("compare_needs_equal", op = op.text(), type = ta.text())).help(help));
                            }
                        }
                        self.like(a, ta, b);
                        Type::Bool
                    }
                    Lt | Le | Gt | Ge => {
                        let ta = self.expr(a);
                        if !self.has(&ta, "Comparable") {
                            let help = match &ta {
                                t if later(t, "Comparable").is_some() => later(t, "Comparable").unwrap(),
                                Type::Struct(d) => t!("order_needs_comparable_help", name = d.name, kind = "struct"),
                                Type::Enum(d) => t!("order_needs_comparable_help", name = d.name, kind = "enum"),
                                Type::Param(p) => t!("param_order_help", type = p.name),
                                _ => t!("order_primitives_help"),
                            };
                            self.diags.push(Diag::new(e.span, t!("order_needs_comparable", op = op.text(), type = ta.text())).help(help));
                        }
                        self.like(a, ta, b);
                        Type::Bool
                    }
                    Range | RangeInclusive => {
                        self.expr(a);
                        self.expr(b);
                        self.error(e.span, t!("range_only_in_for", op = op.text()));
                        Type::Error
                    }
                    Rem => self.same_type(a, b, &[Type::Int, Type::BigInt], "`%`"),
                    Plus => self.same_type(a, b, &[Type::Int, Type::Float, Type::String, Type::BigInt], "`+`"),
                    Minus | Times | Div => {
                        self.same_type(a, b, &[Type::Int, Type::Float, Type::BigInt], &format!("`{}`", op.text()))
                    }
                }
            }
        }
    }

    /// `Dog { name := "Rex", age := 3 }`: every field exactly once, in any order.
    fn struct_literal(&mut self, ty: &Name, types: &[TypeAst], fields: &[(Name, Expr)]) -> Type {
        if ty.text == "List" {
            self.diags.push(Diag::new(ty.span, t!("list_literal")).help(t!("list_made_help")));
            return self.unchecked_fields(fields);
        }
        if ty.text == "Map" {
            self.diags.push(Diag::new(ty.span, t!("map_literal")).help(t!("map_made_help")));
            return self.unchecked_fields(fields);
        }
        let d = match self.structs.get(&ty.text).cloned() {
            Some(t @ Type::Struct(_)) if !types.is_empty() || is_template(&t, &ty.text) => match self.with_types(ty, types) {
                Type::Struct(d) => d,
                _ => return self.unchecked_fields(fields),
            },
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
        let owner = Type::Struct(d.clone()).owner();
        if !self.hidden(&owner, ty.span) {
            // Only its own file builds a value whose fields it hides (PLAN 2.5).
            let hides = d.fields.iter().find(|(n, _)| {
                self.private.get(&method_name(&owner, n)).is_some_and(|s| file_number(self.src, s.start) != file_number(self.src, ty.span.start))
            });
            if let Some((field, _)) = hides {
                let file = format!("{}.kova", owner.split('.').next().unwrap_or(&owner));
                self.diags.push(
                    Diag::new(ty.span, t!("private_literal", type = ty.text, field = field, file = file))
                        .help(t!("private_literal_help", type = ty.text)),
                );
            }
        }
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
            self.mention(&method_name(&owner, &name.text), name.span);
            self.expect(value, t);
        }
        let missing: Vec<String> =
            d.fields.iter().filter(|(n, _)| !given.contains(&n.as_str())).map(|(n, _)| format!("`{n}`")).collect();
        if !missing.is_empty() {
            self.error(ty.span, t!("fields_missing", type = d.name, fields = missing.join(", ")));
        }
        Type::Struct(d)
    }

    fn unchecked_fields(&mut self, fields: &[(Name, Expr)]) -> Type {
        fields.iter().for_each(|(_, v)| {
            self.expr(v);
        });
        Type::Error
    }

    /// `Stack<Int>` before `{` or `.`: written in full, since Kova does not deduce it (PLAN Part 4).
    fn with_types(&mut self, ty: &Name, types: &[TypeAst]) -> Type {
        if types.is_empty() {
            self.generic_without_types(ty);
            return Type::Error;
        }
        let given: Vec<Type> = types.iter().map(|t| self.written_type(t)).collect();
        let template = self.structs.get(&ty.text).cloned();
        self.generic(ty, template.as_ref(), &given)
    }

    /// `Shape.Circle(2.0)`: the fields in their declared order.
    fn variant(&mut self, ty: &Name, name: &Name, args: &[Expr]) -> Type {
        self.hidden(&ty.text, ty.span);
        self.mention(&method_name(&ty.text, &name.text), name.span);
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
        self.arguments(name, args, &params, &[]);
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
                        Some((Some((i, fields)), d)) => {
                            // `Circle(r) =>` is `Shape.Circle`, for the editor (F2 renames both).
                            if !optional && !outcome {
                                self.mention(&method_name(&d.name, name), span);
                            }
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
        if let Some(d) = &d {
            // `Circle(radius)`: the variant and its fields' names; the value of an optional: `value`.
            let written = |i: usize| match &d.variants[i] {
                (_, fs) if optional && i == 1 => fs[0].0.clone(),
                (n, fs) if fs.is_empty() => n.clone(),
                (n, fs) => format!("{n}({})", fs.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>().join(", ")),
            };
            let todo = if any.is_some() { vec![] } else { (0..d.variants.len()).filter(|i| !seen.contains_key(i)).map(written).collect() };
            self.matches.push((m.subject.span.to(m.close), todo));
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

    fn call(&mut self, id: usize, name: &Name, args: &[Expr]) -> Type {
        // `test(x)`: a block parameter, called.
        if let Some(Variable { ty: Type::Block(ps, r), span, .. }) = self.lookup(&name.text) {
            let (ps, r, decl) = (ps.clone(), (**r).clone(), *span);
            self.mention_variable(&name.text, &Type::Block(ps.clone(), Box::new(r.clone())), name.span, decl);
            self.invoked.insert(id);
            let params: Vec<(Mode, Type)> = ps.into_iter().map(|t| (Mode::Read, t)).collect();
            self.arguments(name, args, &params, &[]);
            return r;
        }
        self.hidden(&name.text, name.span);
        // `random(1..=6)`: a range of `Int`s, written there (a range is not a value of its own).
        if name.text == "random" && self.lookup("random").is_none() {
            match args {
                [r] if r.range().is_some() => {
                    let (_, a, b) = r.range().unwrap();
                    self.expect(a, &Type::Int);
                    self.expect(b, &Type::Int);
                    self.types[r.id] = Type::Unit;
                }
                _ => {
                    self.unchecked(args);
                    self.diags.push(Diag::new(name.span, t!("random_range")).help(t!("random_range_help")));
                }
            }
            return Type::Int;
        }
        if BUILTINS.contains(&name.text.as_str()) {
            if args.len() != 1 {
                self.error(name.span, t!("builtin_one_arg", name = name.text));
            }
            for a in args {
                match self.expr(a) {
                    Type::Unit => self.error(a.span, t!("print_no_value", name = name.text)),
                    Type::Block(..) => self.error(a.span, t!("print_block", name = name.text)),
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
        let sig = sig.clone();
        if sig.generics.is_empty() {
            self.arguments(name, args, &sig.params, &[]);
            return sig.ret;
        }
        // `larger(3, 4)`: `T` is deduced from the arguments, then must keep its promises.
        let (mut found, mut typed) = (vec![], vec![None; args.len()]);
        for (i, a) in args.iter().enumerate() {
            let Some((_, p)) = sig.params.get(i) else { continue };
            if sig.generics.iter().any(|g| mentions(p, &g.name)) {
                let value = match &a.kind {
                    ExprKind::Mark(_, x) => &**x,
                    _ => a,
                };
                let t = if matches!(p, Type::Block(..)) { self.passed(value) } else { self.expr(value) };
                deduce(p, &t, &sig.generics, &mut found);
                typed[i] = Some(t);
            }
        }
        let errors = typed.iter().flatten().any(|t| *t == Type::Error);
        for g in &sig.generics {
            match found.iter().find(|(n, _)| *n == g.name) {
                None if !errors => self.error(name.span, t!("generic_not_deduced", name = g.name, function = name.text)),
                None => {}
                // `identity(f)` would give the block back, outliving its call.
                Some((_, t)) if matches!(t, Type::Block(..)) => {
                    self.error(name.span, t!("block_generic", name = g.name, function = name.text));
                }
                Some((_, t)) => {
                    let broken: Vec<&String> = g.bounds.iter().filter(|b| !self.satisfies(t, b)).collect();
                    for b in broken {
                        self.diags.push(
                            Diag::new(name.span, t!("generic_bound", function = name.text, name = g.name, type = t.text(), bound = b))
                                .help(bound_help(t, b)),
                        );
                    }
                }
            }
        }
        let sig = sig.substitute(&found);
        self.arguments(name, args, &sig.params, &typed);
        self.type_args.insert(id, sig.generics.iter().map(|g| found.iter().find(|(n, _)| *n == g.name).map_or(Type::Error, |(_, t)| t.clone())).collect());
        sig.ret
    }
}
