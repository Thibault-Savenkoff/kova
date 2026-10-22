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
    /// What an imported file hides from the other files (not under `public:`): its functions and
    /// types (`geometry.helper`), the fields and methods of its types (`geometry.Point.x`),
    /// with where they are declared.
    pub private: Vec<(String, Span)>,
}

/// `import geometry` or `import geometry as geo` (PLAN 2.9).
pub struct Import {
    /// The file's name: `geometry` for `geometry.kova`, next to this file.
    pub module: Name,
    /// The name used in this file: `geo` (the module's own name without `as`).
    pub alias: Name,
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
pub struct Match {
    pub subject: Expr,
    pub arms: Vec<Arm>,
    /// The `}`: where missing cases are reported.
    pub close: Span,
}

/// `Circle(r) => 3.14 * r * r`: a single statement after `=>` becomes a block.
pub struct Arm {
    pub case: Case,
    pub body: Block,
}

pub enum Case {
    /// `Circle(r)`, `Red`: a variant (without its type's name) and what receives its fields.
    /// `none` on an optional (`T?`), `ok(v)` and `err(e)` on a result are variants too.
    Variant { name: Name, binds: Vec<Pattern> },
    /// `n =>` on an optional: a name alone receives the value (PLAN Part 4, J4 step 2).
    Bind(Pattern),
    /// `_`: every other case.
    Any(Span),
}

impl Case {
    /// The variant of the arm and what receives its fields; the value of an optional is `some`.
    pub fn variant(&self) -> Option<(&str, Vec<&Pattern>)> {
        match self {
            Case::Variant { name, binds } => Some((&name.text, binds.iter().collect())),
            Case::Bind(p) => Some(("some", vec![p])),
            Case::Any(_) => None,
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
    pub ret: Option<TypeAst>,
    pub contracts: Vec<Contract>,
    pub body: Block,
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
pub struct Contract {
    pub kind: ContractKind,
    pub cond: Expr,
    /// The text after `else`; its `{values}` are computed only if the contract fails.
    pub message: Option<Vec<Part>>,
}

/// A piece of a text with `{}`: `"Hello {name}!"` = `Text("Hello ")`, `Code(name)`, `Text("!")`.
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
        }
    }
}

/// What receives a value in a `let`: `x`, `(a, b)`, `(a, _)`.
pub enum Pattern {
    Name(Name),
    Tuple(Vec<Pattern>, Span),
    /// `_`: the value is ignored.
    Ignore(Span),
}

/// What receives a value in `:=`: `x`, `t.0`, `rex.age`, `(a, b)`.
pub enum Target {
    /// A variable, followed by fields: `t.1.age` = `name: t, fields: [Index(1), Name(age)]`.
    Place { name: Name, fields: Vec<Step>, span: Span },
    Tuple(Vec<Target>, Span),
}

/// One step into a place: `.0` (tuple), `.age` (struct) or `[i]` (List element).
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
pub struct Param {
    pub name: Name,
    pub mode: Mode,
    pub ty: TypeAst,
}

impl Param {
    /// The type as declared: `String`, `var String`.
    pub fn ty_text(&self) -> String {
        format!("{} {}", self.mode.word(), self.ty.text()).trim_start().to_string()
    }
}

pub struct Block {
    /// Position of the opening `{`.
    pub open: Span,
    pub stmts: Vec<Stmt>,
    /// Position of the closing `}` (for "missing return").
    pub close: Span,
}

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
    /// `for i in a..b` (`inclusive` = false) and `for i in a..=b` / `from i := a to b` (`inclusive` = true).
    Loop { counter: Name, start: Expr, end: Expr, inclusive: bool, body: Block },
    /// `for score in scores`: each element of a List, borrowed (PLAN 2.7.9).
    Each { name: Name, list: Expr, body: Block },
    Match(Match),
    Expr(Expr),
}

pub struct Expr {
    /// Unique number: index in the type table (`types::Info`).
    pub id: usize,
    pub span: Span,
    pub kind: ExprKind,
}

pub enum ExprKind {
    Int(u64),
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
    Variant { ty: Name, name: Name, args: Vec<Expr> },
    /// `x.to_float`, `rex.name`: a built-in method or a struct's field (decided by the type checker).
    Method { receiver: Box<Expr>, name: Name, args: Vec<Expr> },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `(1, 2.5)`
    Tuple(Vec<Expr>),
    /// `[12, 5, 18]`: a new List.
    List(Vec<Expr>),
    /// `List<Int>()`: a new empty List of this type.
    New(TypeAst),
    /// `scores[i]`: an element of a List (a place, like a field).
    Index(Box<Expr>, Box<Expr>),
    /// `t.0`
    Field(Box<Expr>, usize),
    /// `Dog { name := "Rex", age := 3 }`: the fields in the written order.
    StructLit { ty: Name, types: Vec<TypeAst>, fields: Vec<(Name, Expr)> },
    /// `mode = build` (`negated`: `mode != build`); `build` false = `run`. Known before running.
    InMode { build: bool, negated: bool },
    /// `if c { 1 } else { 2 }` giving a value: the last line of the chosen branch.
    If { cond: Box<Expr>, then: Block, otherwise: Block },
    /// `match` giving a value: the last line of the chosen arm.
    Match(Box<Match>),
    /// `none`: no value, in an optional (`T?`); its type comes from where it goes.
    None,
    /// `x?`: the value of the optional `x`, or else the function returns `none` at once;
    /// on a result, its `ok` value, or else the function returns its `err` at once.
    Try(Box<Expr>),
    /// `ok(value)` or `err(error)`: a result; its type comes from where it goes.
    Outcome { ok: bool, value: Box<Expr> },
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
        }
    }
}
