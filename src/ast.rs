// AST: the program tree, as written. Every node keeps its position (`Span`).
//
//   total := total + i   →   Assign { target: "total", value: Binary(Plus, Var(total), Var(i)) }

use crate::diag::Span;

pub struct File {
    pub header: Option<Header>,
    pub functions: Vec<Function>,
    pub tests: Vec<Test>,
    /// `lang fr { ... }`: only in a translation file (PLAN 2.13).
    pub langs: Vec<Lang>,
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

pub struct Function {
    pub name: Name,
    pub params: Vec<Param>,
    pub ret: Option<TypeAst>,
    pub contracts: Vec<Contract>,
    pub body: Block,
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
    pub message: Option<String>,
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

/// A type as written: `Int`, `(Int, Float)`, `((Int, Int), Bool)`.
pub enum TypeAst {
    Name(Name),
    Tuple(Vec<TypeAst>),
}

impl TypeAst {
    /// The type as in the code: `(Int, Float)`.
    pub fn text(&self) -> String {
        match self {
            TypeAst::Name(n) => n.text.clone(),
            TypeAst::Tuple(ts) => format!("({})", ts.iter().map(TypeAst::text).collect::<Vec<_>>().join(", ")),
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

/// What receives a value in `:=`: `x`, `t.0`, `(a, b)`.
pub enum Target {
    /// A variable, followed by field numbers: `t.1.0` = `name: t, fields: [1, 0]`.
    Place { name: Name, fields: Vec<usize>, span: Span },
    Tuple(Vec<Target>, Span),
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

pub struct Param {
    pub name: Name,
    pub ty: TypeAst,
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
    Var(String),
    /// `result` in an `ensure`.
    Result,
    Call { name: Name, args: Vec<Expr> },
    /// `x.to_float()`: built-in method (J2: only conversions).
    Method { receiver: Box<Expr>, name: Name, args: Vec<Expr> },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `(1, 2.5)`
    Tuple(Vec<Expr>),
    /// `t.0`
    Field(Box<Expr>, usize),
    /// `mode = build` (`negated`: `mode != build`); `build` false = `run`. Known before running.
    InMode { build: bool, negated: bool },
    /// `if c { 1 } else { 2 }` giving a value: the last line of the chosen branch.
    If { cond: Box<Expr>, then: Block, otherwise: Block },
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
