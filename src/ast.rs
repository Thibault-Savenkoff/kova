// AST: the program tree, as written. Every node keeps its position (`Span`).
//
//   total := total + i   →   Assign { target: "total", value: Binary(Plus, Var(total), Var(i)) }

use crate::diag::Span;

pub struct File {
    pub header: Option<Header>,
    /// `import geometry as geo`: the other files this one uses.
    pub imports: Vec<Import>,
    pub functions: Vec<Function>,
    pub structs: Vec<Struct>,
    pub interfaces: Vec<Interface>,
    pub tests: Vec<Test>,
    /// `lang fr { ... }`: only in a translation file (PLAN 2.13).
    pub langs: Vec<Lang>,
    /// What a file hides from the other files (not under `public:`): its functions and
    /// types (`geometry.helper`), the fields and methods of its types (`geometry.Point.x`),
    /// with where they are declared.
    pub private: Vec<(String, Span)>,
    /// The modules the program reads, as written after `import` (`geometry`, `shapes.circle`):
    /// filled in the root file once they are all read.
    /// With each one its file, as messages name it (`shapes/circle.kova`).
    pub modules: Vec<(String, String)>,
    /// `extern sqlite from "sqlite3" { … }`: functions of the system's libraries.
    pub externs: Vec<Extern>,
    /// `ui.run(title, state, view, update)`: the call keeps the title and the state; the two
    /// functions are written into the `ui` module made for the program (`src/ui.rs`).
    pub ui_runs: Vec<UiRun>,
    /// Where that module's text starts, the last file of the program: it sees every name.
    pub ui_from: Option<usize>,
}

/// One `ui.run(…)`: where, and the names of its `view` and `update` functions.
pub struct UiRun {
    pub call: Span,
    pub view: Name,
    pub update: Name,
}

/// `extern sqlite from "sqlite3" { fn open(path: String, db: var Pointer) -> Int32 from
/// "sqlite3_open" }` (PLAN Part 4 "`extern`, step 1"): called `sqlite.open(…)`.
pub struct Extern {
    pub name: Name,
    /// The library, as written.
    pub library: CName,
    /// Each function, named `sqlite.open`, without a body; its C name when it is not the same;
    /// the parameters given by value (`frame: copy Rect`; the result is number `params.len()`).
    pub functions: Vec<(Function, Option<CName>, Vec<usize>)>,
    /// Its structs, named `win.Point`, with their C types (PLAN Part 4 "`extern`, step 3"); the
    /// parser also gives the program each one as a Kova struct (`Struct::c_view`).
    pub structs: Vec<Struct>,
}

/// A library's or a C function's name, then the systems that name it otherwise (`from "sqlite3"
/// windows "winsqlite3"`, PLAN Part 4 "`extern`, names per system").
pub struct CName {
    pub text: (String, Span),
    pub on: Vec<(Name, (String, Span))>,
}

/// `import geometry` or `import geometry as geo` (PLAN 2.9).
pub struct Import {
    /// The file's name: `geometry` for `geometry.kova`, next to this file. For a package, its own
    /// name (`package_name`).
    pub module: Name,
    /// The name used in this file: `geo` (the module's own name without `as`).
    pub alias: Name,
    /// `import json from "github.com/ada/kova-json" version "1.2.0"`: the address and the
    /// version, without their quotes.
    pub package: Option<(Name, Name)>,
}

/// A package's own name, whatever name a file gives it: its address's last part, with `_` for
/// what a name cannot hold (`github.com/ada/kova-json` → `kova_json`). No `.`: qualified names
/// are cut at dots.
pub fn package_name(address: &str) -> String {
    let last = address.trim_end_matches('/').rsplit('/').next().unwrap_or(address);
    let name: String = last.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    if name.starts_with(|c: char| c.is_ascii_alphabetic()) { name } else { format!("p_{name}") }
}

/// `lang fr { already_exists := "`{name}` existe déjà" }`.
pub struct Lang {
    #[allow(dead_code)] // read by the tests; later by the programs that pick a language
    pub name: Name,
    /// (key, the `"..."` token); the text is read by `locale::segments`.
    pub entries: Vec<(Name, Span)>,
}

/// `header { mode := build  lock := true }`: the file's settings (PLAN 2.11).
pub struct Header {
    /// The `{` and the `}`.
    pub open: Span,
    pub close: Span,
    /// (key, value): `mode := build` → (`mode`, `build`).
    pub settings: Vec<(Name, Name)>,
}

/// `struct Dog { name: String, age: Int }`; `copy struct` = copied like an `Int` (PLAN 2.6).
/// Also `enum Shape { Circle(radius: Float), Square(side: Float) }`: then `variants` is `Some`.
pub struct Struct {
    pub copy: bool,
    pub name: Name,
    /// `struct Stack<T>`: its type parameters.
    pub generics: Vec<Generic>,
    /// `struct Dog : Animal, Aging`: the interfaces it follows.
    pub interfaces: Vec<Name>,
    pub fields: Vec<Param>,
    pub variants: Option<Vec<Variant>>,
    /// Its methods (`fn sound(self)`) and static functions (`fn create(name: String) -> Dog`).
    pub methods: Vec<Function>,
}

impl Struct {
    /// A struct of an `extern` block as the program sees it: a `copy` struct whose `Int32`s are
    /// `Int`s (not `copy` if a field cannot be one: the type checker says why that field is wrong).
    pub fn kova_view(&self) -> Struct {
        fn c_like(t: &TypeAst) -> bool {
            match t {
                TypeAst::Name(n) => ["Int", "Int32", "Float", "Bool", "Pointer"].contains(&n.text.as_str()) || n.text.contains('.'),
                TypeAst::Optional(t) => c_like(t),
                TypeAst::Block(..) => true,
                _ => false,
            }
        }
        fn kova(t: &TypeAst) -> TypeAst {
            match t {
                TypeAst::Name(n) if n.text == "Int32" => TypeAst::Name(Name { text: "Int".into(), span: n.span }),
                TypeAst::Name(n) if n.text == "WideString" => TypeAst::Name(Name { text: "String".into(), span: n.span }),
                // A function C calls: the program holds its address.
                TypeAst::Block(..) => TypeAst::Name(Name { text: "Pointer".into(), span: Span::new(0, 0) }),
                TypeAst::Optional(t) => TypeAst::Optional(Box::new(kova(t))),
                t => t.clone(),
            }
        }
        let fields = self.fields.iter().map(|p| Param { ty: kova(&p.ty), ..p.clone() }).collect();
        let copy = self.fields.iter().all(|p| c_like(&p.ty));
        Struct { copy, name: self.name.clone(), generics: vec![], interfaces: vec![], fields, variants: None, methods: vec![] }
    }
}

/// `interface Animal { fn sound(self) -> String … }`: what a type promises (PLAN 2.6).
pub struct Interface {
    pub name: Name,
    /// `Equal`, `Comparable`, `Display`: given by Kova (`parser::PRELUDE`), not written in the file.
    pub builtin: bool,
    /// Each method, and whether it has a body (a default method, used when the type doesn't
    /// write its own); without a body, its `body` is empty.
    pub methods: Vec<(Function, bool)>,
}

/// `Circle(radius: Float)` in an `enum`; `Red` has no fields.
pub struct Variant {
    pub name: Name,
    pub fields: Vec<Param>,
}

/// `match shape { Circle(r) => …, Square(s) => … }`, as a statement or giving a value.
#[derive(Clone)]
pub struct Match {
    /// `match var self.left`: `Var`, the names of its cases change the value matched.
    pub mode: Mode,
    pub subject: Expr,
    pub arms: Vec<Arm>,
    /// The `}`: where missing cases are reported.
    pub close: Span,
}

/// `Circle(r) => 3.14 * r * r`: a single statement after `=>` becomes a block.
#[derive(Clone)]
pub struct Arm {
    pub case: Case,
    /// `Saturday, Sunday =>`: the other variants of the arm, without fields (PLAN Part 4, "Grouped cases").
    pub also: Vec<Name>,
    pub body: Block,
}

#[derive(Clone)]
pub enum Case {
    /// `Circle(r)`, `Red`: a variant (without its type's name) and what receives its fields.
    /// `none` on an optional (`T?`), `ok(v)` and `err(e)` on a result are variants too.
    Variant { name: Name, binds: Vec<Pattern> },
    /// `n =>` on an optional: a name alone receives the value (PLAN Part 4, J4 step 2).
    Bind(Pattern),
    /// `0`, `1, 2`, `'a'`, `"yes"`, `1..=5`: values written out (ranges too, but not of texts),
    /// on an `Int`, a `Char` or a `String`
    /// (PLAN Part 4, "Match on values").
    Values(Vec<Expr>),
    /// `_`: every other case.
    Any(Span),
}

impl Case {
    /// The variant of the arm and what receives its fields; the value of an optional is `some`.
    pub fn variant(&self) -> Option<(&str, Vec<&Pattern>)> {
        match self {
            Case::Variant { name, binds } => Some((&name.text, binds.iter().collect())),
            Case::Bind(p) => Some(("some", vec![p])),
            Case::Values(_) | Case::Any(_) => None,
        }
    }
}

/// How a function receives a value (PLAN 2.7.4): a parameter `x: var T`, or the value
/// a method is called on (`var self`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// `self`, `x: T`: read only.
    Read,
    /// `var self`, `x: var T`: may change it (the caller's variable is updated).
    Var,
    /// `move self`, `x: move T`: takes it (the caller's variable can no longer be used).
    Move,
}

impl Mode {
    /// As written before a type or an argument: ``, `var`, `move`.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Read => "",
            Mode::Var => "var",
            Mode::Move => "move",
        }
    }
    /// As written for a method: `self`, `var self`, `move self`.
    pub fn text(self) -> &'static str {
        match self {
            Mode::Read => "self",
            Mode::Var => "var self",
            Mode::Move => "move self",
        }
    }
}

pub struct Function {
    pub name: Name,
    /// `<T: Comparable + Display, U>`: its type parameters, deduced at each call.
    pub generics: Vec<Generic>,
    /// `Some` for a method with `self`; `None` for a function or a static function.
    pub receiver: Option<Mode>,
    pub params: Vec<Param>,
    /// `fn put(value: Int) at(index: Int)`: where each part after the first starts in `params`
    /// (empty for an ordinary function). Its name is then `put(…) at(…)`.
    pub breaks: Vec<usize>,
    pub ret: Option<TypeAst>,
    pub contracts: Vec<Contract>,
    /// `mode build else skip` under the signature (PLAN 2.14).
    pub only: Option<OnlyIn>,
    pub body: Block,
}

/// `mode build` or `mode run` under a function's signature: what the call does in the other mode.
pub struct OnlyIn {
    pub build: bool,
    /// `mode build`, for the line of the message that stops the program.
    pub span: Span,
    pub otherwise: Otherwise,
}

pub enum Otherwise {
    /// No `else`: the function runs in both modes, the user notices nothing.
    Discreet,
    /// `else "message"`: the program stops at the call.
    Stop(Vec<Part>),
    /// `else skip`: the call does nothing.
    Skip,
    /// `else return 0`: the call gives this value.
    Return(Expr),
}

impl Function {
    /// Each part of the name with its parameters: `put` with `value`, then `at` with `index`.
    pub fn parts(&self) -> Vec<(&str, &[Param])> {
        let mut names: Vec<&str> =
            if self.breaks.is_empty() { vec![&self.name.text] } else { self.name.text.trim_end_matches("(…)").split("(…) ").collect() };
        // `walk far(…)`: a first part without arguments is followed by a space only.
        if !self.breaks.is_empty()
            && let Some((first, second)) = names[0].split_once(' ')
        {
            names.splice(0..1, [first, second]);
        }
        let mut bounds = vec![0];
        bounds.extend(&self.breaks);
        bounds.push(self.params.len());
        names.into_iter().zip(bounds.windows(2)).map(|(n, w)| (n, &self.params[w[0]..w[1]])).collect()
    }
}

/// `T: Comparable + Display` in `fn larger<T: Comparable + Display>`.
pub struct Generic {
    pub name: Name,
    /// The interfaces the type given for it must follow.
    pub bounds: Vec<Name>,
}

/// `test "carré de 3" { check carre(3) = 9 }`: only run by `kova test`.
pub struct Test {
    pub name: String,
    /// The `"..."` text, for messages.
    pub span: Span,
    pub body: Block,
}

/// `require b != 0 else "message"` or `ensure result >= 0`.
#[derive(Clone)]
pub struct Contract {
    pub kind: ContractKind,
    pub cond: Expr,
    /// The text after `else`; its `{values}` are computed only if the contract fails.
    pub message: Option<Vec<Part>>,
}

/// A piece of a text with `{}`: `"Hello {name}!"` = `Text("Hello ")`, `Code(name)`, `Text("!")`.
#[derive(Clone)]
pub enum Part {
    Text(String),
    Code(Expr),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ContractKind {
    /// Checked before each call.
    Require,
    /// Checked at each exit; `result` = the returned value.
    Ensure,
    /// Checked where it is written.
    Check,
}

impl ContractKind {
    pub fn keyword(self) -> &'static str {
        match self {
            ContractKind::Require => "require",
            ContractKind::Ensure => "ensure",
            ContractKind::Check => "check",
        }
    }
}

/// A type as written: `Int`, `(Int, Float)`, `((Int, Int), Bool)`, `List<Int>`.
#[derive(Clone)]
pub enum TypeAst {
    Name(Name),
    Tuple(Vec<TypeAst>),
    /// `List<Int>`: a generic type and its arguments.
    Generic(Name, Vec<TypeAst>),
    /// `Int?`: an `Int` or `none`.
    Optional(Box<TypeAst>),
    /// `String or IoError`: a success (`ok`) or a failure (`err`).
    Result(Box<TypeAst>, Box<TypeAst>),
    /// `any Animal`: a value of any type following the interface.
    Any(Name),
    /// `{Int -> Bool}`: a block taking an `Int` and giving a `Bool` (only a parameter's type).
    Block(Vec<TypeAst>, Option<Box<TypeAst>>),
    /// A block's parameter whose type comes from what the block is given to.
    Inferred,
}

impl TypeAst {
    /// The type as in the code: `(Int, Float)`.
    pub fn text(&self) -> String {
        match self {
            TypeAst::Name(n) => n.text.clone(),
            TypeAst::Tuple(ts) => format!("({})", ts.iter().map(TypeAst::text).collect::<Vec<_>>().join(", ")),
            TypeAst::Generic(n, ts) => format!("{}<{}>", n.text, ts.iter().map(TypeAst::text).collect::<Vec<_>>().join(", ")),
            TypeAst::Optional(t) => format!("{}?", t.text()),
            TypeAst::Result(t, e) => format!("{} or {}", t.text(), e.text()),
            TypeAst::Any(n) => format!("any {}", n.text),
            TypeAst::Block(ps, r) => block_text(ps.iter().map(TypeAst::text).collect(), r.as_ref().map(|r| r.text())),
            TypeAst::Inferred => String::new(),
        }
    }
}

/// A block's type as in the code: `{Int -> Bool}`, `{Int}`, `{-> Bool}`, `{}`.
pub fn block_text(params: Vec<String>, result: Option<String>) -> String {
    match result {
        Some(r) if params.is_empty() => format!("{{-> {r}}}"),
        Some(r) => format!("{{{} -> {r}}}", params.join(", ")),
        None => format!("{{{}}}", params.join(", ")),
    }
}

/// What receives a value in a `let`: `x`, `(a, b)`, `(a, _)`.
#[derive(Clone)]
pub enum Pattern {
    Name(Name),
    Tuple(Vec<Pattern>, Span),
    /// `_`: the value is ignored.
    Ignore(Span),
}

/// What receives a value in `:=`: `x`, `t.0`, `rex.age`, `(a, b)`.
#[derive(Clone)]
pub enum Target {
    /// A variable, followed by fields: `t.1.age` = `name: t, fields: [Index(1), Name(age)]`.
    Place { name: Name, fields: Vec<Step>, span: Span },
    Tuple(Vec<Target>, Span),
}

/// One step into a place: `.0` (tuple), `.age` (struct) or `[i]` (List element).
#[derive(Clone)]
pub enum Step {
    Index(usize),
    Name(Name),
    At(Expr),
}

impl Step {
    /// The step, to compare two targets: `0`, `age`; each `[i]` is different (its expression's id).
    pub fn text(&self) -> String {
        match self {
            Step::Index(i) => i.to_string(),
            Step::Name(n) => n.text.clone(),
            Step::At(e) => format!("[{}]", e.id),
        }
    }
}

impl Target {
    pub fn span(&self) -> Span {
        match self {
            Target::Place { span, .. } | Target::Tuple(_, span) => *span,
        }
    }
}

/// A name and its position (variable, function or type).
#[derive(Clone)]
pub struct Name {
    pub text: String,
    pub span: Span,
}

/// A parameter `x: var T` or a struct's field (always `Read`).
#[derive(Clone)]
pub struct Param {
    pub name: Name,
    pub mode: Mode,
    pub ty: TypeAst,
    /// `port(p: Int := 80)`: the value given when the call leaves the part out, and its text.
    pub default: Option<(Expr, String)>,
}

impl Param {
    /// The type as declared: `String`, `var String`.
    pub fn ty_text(&self) -> String {
        format!("{} {}", self.mode.word(), self.ty.text()).trim_start().to_string()
    }
}

#[derive(Clone)]
pub struct Block {
    /// Position of the opening `{`.
    pub open: Span,
    pub stmts: Vec<Stmt>,
    /// Position of the closing `}` (for "missing return").
    pub close: Span,
}

#[derive(Clone)]
pub enum Stmt {
    /// `let x: Int := 5`, `var (a, b): (Int, Int) := f()` (`var` if `mutable`).
    Declare { mutable: bool, pattern: Pattern, ty: Option<TypeAst>, value: Expr },
    /// `x := 5`, `(a, b) := (b, a)`, or `x += 5` (`op` = `Some(Plus)`).
    Assign { target: Target, op: Option<BinOp>, value: Expr },
    Return { value: Option<Expr>, span: Span },
    /// `check cond else "message"`.
    Check(Contract),
    Break(Span),
    Continue(Span),
    If { cond: Expr, then: Block, otherwise: Option<Block> },
    While { cond: Expr, body: Block },
    /// `for i in a..b` (`inclusive` = false) and `for i in a..=b` (`inclusive` = true).
    /// `step`: what is added at each turn, 1 unless the loop says.
    Loop { counter: Name, start: Expr, end: Expr, inclusive: bool, step: i64, body: Block },
    /// `for score in scores`: each element of a List, borrowed (PLAN 2.7.9).
    /// `for x in l`, or `for name, age in ages` on a Map (`value`).
    /// `for x in var l`: `mode` = `Var`, each `x` changes its element (PLAN 2.7.9).
    Each { name: Name, value: Option<Name>, mode: Mode, list: Expr, body: Block },
    Match(Match),
    Expr(Expr),
}

#[derive(Clone)]
pub struct Expr {
    /// Unique number: index in the type table (`types::Info`).
    pub id: usize,
    pub span: Span,
    pub kind: ExprKind,
}

impl Expr {
    /// `1..3` (`false`) or `1..=3` (`true`), with its two ends: in `l[1..3]`, a slice.
    pub fn range(&self) -> Option<(bool, &Expr, &Expr)> {
        match &self.kind {
            ExprKind::Binary(op @ (BinOp::Range | BinOp::RangeInclusive), a, b) => Some((*op == BinOp::RangeInclusive, a, b)),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub enum ExprKind {
    Int(u64),
    /// `123456789012345678901234567890`: too big for an `Int`, a `BigInt` (its digits).
    BigInt(String),
    Float(f64),
    Char(char),
    Bool(bool),
    /// `"hello"`, escapes already decoded: a new `String`.
    Str(String),
    /// `"Hello {name}!"`: a new `String` made of its parts.
    Interp(Vec<Part>),
    Var(String),
    /// `result` in an `ensure`.
    Result,
    Call { name: Name, args: Vec<Expr> },
    /// `var a` or `move a` in a call's arguments (never `Read`).
    Mark(Mode, Box<Expr>),
    /// `Dog.create("Rex")`: a struct's function without `self`.
    /// `Stack<Int>.empty`: `types` = the type arguments written after the struct's name.
    Static { ty: Name, types: Vec<TypeAst>, name: Name, args: Vec<Expr> },
    /// `Shape.Circle(2.0)`, `Light.Red`: a new value of an enum.
    Variant { ty: Name, types: Vec<TypeAst>, name: Name, args: Vec<Expr> },
    /// `x.to_float`, `rex.name`: a built-in method or a struct's field (decided by the type checker).
    Method { receiver: Box<Expr>, name: Name, args: Vec<Expr> },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `(1, 2.5)`
    Tuple(Vec<Expr>),
    /// `[12, 5, 18]`: a new List.
    List(Vec<Expr>),
    /// `Set["a", "b"]`: a new Set (`Set[]` is typed by where it goes).
    SetLit(Vec<Expr>),
    /// `["Rex" := 3, "Fido" := 5]`: a new Map (`[]` is an empty one, typed by where it goes).
    MapLit(Vec<(Expr, Expr)>),
    /// `List<Int>()`: a new empty List of this type.
    New(TypeAst),
    /// `scores[i]`: an element of a List (a place, like a field); `scores[1..3]`: a new List of
    /// copies of some elements (not a place).
    Index(Box<Expr>, Box<Expr>),
    /// `t.0`
    Field(Box<Expr>, usize),
    /// `Dog { name := "Rex", age := 3 }`: the fields in the written order. `base`: the value of
    /// `Dog { ...rex, age := 4 }`, which gives the fields not written.
    StructLit { ty: Name, types: Vec<TypeAst>, fields: Vec<(Name, Expr)>, base: Option<Box<Expr>> },
    /// `ages["Rex"] else 0`: the value of an optional or a result, or else the value on the right.
    Else(Box<Expr>, Box<Expr>),
    /// `if c { 1 } else { 2 }` giving a value: the last line of the chosen branch.
    If { cond: Box<Expr>, then: Block, otherwise: Block },
    /// `match` giving a value: the last line of the chosen arm.
    Match(Box<Match>),
    /// `none`: no value, in an optional (`T?`); its type comes from where it goes.
    None,
    /// `x?`: the value of the optional `x`, or else the function returns `none` at once;
    /// on a result, its `ok` value, or else the function returns its `err` at once.
    Try(Box<Expr>),
    /// `a is Dog`: true if the `any` value `a` holds a `Dog`; inside `if a is Dog { … }`, `a` is it.
    Is(Box<Expr>, TypeAst),
    /// `ok(value)` or `err(error)`: a result; its type comes from where it goes.
    Outcome { ok: bool, value: Box<Expr> },
    /// `{ x: Int => x > 2 }`: a block, only written as a call's argument; its value is its last line.
    Block { params: Vec<Param>, body: Block },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Range,
    RangeInclusive,
    Plus,
    Minus,
    Times,
    Div,
    Rem,
    BitOr,
    BitXor,
    BitAnd,
    ShiftLeft,
    ShiftRight,
}

impl BinOp {
    pub fn text(self) -> &'static str {
        use BinOp::*;
        match self {
            Or => "||",
            And => "&&",
            Eq => "=",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            Range => "..",
            RangeInclusive => "..=",
            Plus => "+",
            Minus => "-",
            Times => "*",
            Div => "/",
            Rem => "%",
            BitOr => "|",
            BitXor => "^",
            BitAnd => "&",
            ShiftLeft => "<<",
            ShiftRight => ">>",
        }
    }
}
