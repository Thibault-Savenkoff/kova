// KIR: the program reduced to very simple instructions, shared by both backends.
//
// Each function has numbered 64-bit slots ("locals"): integer or float
// (a Bool is 0 or 1). A Kova value takes one or more slots (`Slots`):
// an `Int` takes one, an `(Int, Float)` tuple two, an empty result none.
// A function also has blocks. A block = a list of instructions + an end
// (jump, branch or return). There is no more `for`, `from` or `&&`:
//
//   from i := 1 to 3 { print(i) }
//
//   b0: %0 := 1          %1 := 3          jump b1
//   b1: %2 := %0 <= %1                    if %2 then b2 else b4
//   b2: print %0                          jump b3
//   b3: %3 := %0 = %1                     if %3 then b4 else b5
//   b5: %0 := %0 + 1                      jump b1
//   b4: ...
//
// Runtime errors (overflow, division by zero) are checked inside `Binary`
// and `Neg`: each backend translates them identically.
//
// A `String` is one slot holding a handle (an address in native code, a number in the
// interpreter); 0 = no String. Each one has a single owner (`ownership.rs`):
//   - a variable frees its String at the end of its block (`Free`); a move empties the
//     variable first (slot := 0), and freeing 0 does nothing;
//   - a temporary (`a + b` given to `print`) is freed at the end of its statement.
//
// Contracts become ordinary tests followed by a `Fail` (message in pieces):
//   - `require`: at the start of the function. It receives 2 hidden parameters, the line and
//     column of the call, so that the message says who called it wrongly;
//   - `ensure`: before each `return`, with `result` = the returned value;
//   - `check`: where it is written.

use std::collections::HashMap;

use crate::ast::{self, BinOp, Block, Contract, ContractKind, Expr, ExprKind, Part, Pattern, Mode, Step, Stmt, Target, UnOp};
use crate::diag::{Span, line_col};
use crate::locale;
use crate::types::{Info, Type, method_name, type_of};

pub type Local = usize;
pub type BlockId = usize;
/// The slots holding a Kova value.
pub type Slots = Vec<Local>;

/// The machine type of a slot.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scalar {
    /// Int, Bool, Char.
    Int,
    Float,
}

/// The elements of a tuple or the fields of a struct, in the order of their slots.
fn members(t: &Type) -> Vec<Type> {
    match t {
        Type::Tuple(ts) => ts.clone(),
        Type::Struct(d) => d.fields.iter().map(|(_, t)| t.clone()).collect(),
        _ => unreachable!("tuple or struct checked by the type checker"),
    }
}

/// Where member `i` (see `members`) lies in the slots of a value of type `t`: (start, length).
fn slice_of(t: &Type, i: usize) -> (usize, usize) {
    let ts = members(t);
    let start = ts[..i].iter().map(|t| scalars(t).len()).sum();
    (start, scalars(&ts[i]).len())
}

fn element(t: &Type, i: usize) -> Type {
    members(t)[i].clone()
}

/// The number of the field `name` in a struct of type `t`.
fn field_index(t: &Type, name: &str) -> usize {
    let Type::Struct(d) = t else { unreachable!("struct checked by the type checker") };
    d.field(name).expect("field checked by the type checker").0
}



/// The slots needed for a value of this type, in order.
pub fn scalars(t: &Type) -> Vec<Scalar> {
    match t {
        Type::Int | Type::Bool | Type::Char | Type::String => vec![Scalar::Int],
        Type::Float => vec![Scalar::Float],
        Type::Tuple(ts) => ts.iter().flat_map(scalars).collect(),
        Type::Struct(_) => members(t).iter().flat_map(scalars).collect(),
        Type::Unit => vec![],
        Type::Error => unreachable!("erroneous type after type checking"),
    }
}

/// For each slot of a value of this type: does it hold a `String`?
fn strings(t: &Type) -> Vec<bool> {
    match t {
        Type::String => vec![true],
        Type::Tuple(_) | Type::Struct(_) => members(t).iter().flat_map(strings).collect(),
        _ => vec![false; scalars(t).len()],
    }
}

pub struct Program {
    pub functions: Vec<Function>,
    pub main: usize,
    /// The `test` blocks (name, number of their function): translated only for `kova test`.
    pub tests: Vec<(String, usize)>,
}

pub struct Function {
    pub name: String,
    /// The parameters are the locals `0..nb_params`.
    pub nb_params: usize,
    /// The type of each local.
    pub locals: Vec<Scalar>,
    pub returns: Vec<Scalar>,
    pub blocks: Vec<KBlock>,
}

pub struct KBlock {
    pub insts: Vec<Inst>,
    pub end: End,
}

/// Position in the source file, for runtime error messages.
#[derive(Clone, Copy)]
pub struct Loc {
    pub line: usize,
    pub col: usize,
}

pub enum Inst {
    /// A constant; for a float slot, `v` holds the bits of the Float.
    Const(Local, i64),
    Copy(Local, Local),
    /// `dst := a op b`; `+ - * / %` check overflow and division by zero.
    Binary(Local, Op, Local, Local, Loc),
    /// `dst := -a`, checked (`-(-9223372036854775808)` overflows).
    Neg(Local, Local, Loc),
    /// `dst := !a` (Bool).
    Not(Local, Local),
    /// `dst := a op b` on Floats; `/` checks division by zero.
    /// Comparisons give a Bool (integer slot).
    FBinary(Local, Op, Local, Local, Loc),
    FNeg(Local, Local),
    Convert(Local, Conversion, Local, Loc),
    /// `results := f(args)`.
    Call(Slots, usize, Slots),
    /// `dst := "text"`: a new String.
    Text(Local, String),
    /// `dst := op(args)`, done by the runtime (`kova_str_*` in `runtime/runtime.c`).
    Str(Local, StrOp, Slots),
    /// Frees a String, then sets its slot to 0 (freeing 0 does nothing).
    Free(Local),
    /// `print` (`newline` = true) or `write`.
    Print { value: Local, show: Show, newline: bool },
    /// Broken contract: writes the message to stderr and stops (code 101).
    Fail(Vec<Piece>),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StrOp {
    /// `a + b`: a new String.
    Concat,
    /// `a = b`: 1 or 0.
    Eq,
    /// `s.length`: the number of characters.
    Length,
    /// `s.clone`: a new String.
    Clone,
    /// The text of a value, as `print` writes it: a new String.
    Format(Show),
}

/// A piece of the message of a broken contract.
pub enum Piece {
    Text(String),
    /// A text of `locale::RUNTIME` (`contract_values`): translated when the message is written,
    /// so that an executable speaks the language of whoever runs it.
    Label(&'static str),
    Value(Local, Show),
    /// The name of the running file (unknown at translation time: `kova lsp` translates text).
    File,
}

/// How to show a slot. Same order as in `runtime/runtime.c`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Show {
    Int = 0,
    Bool = 1,
    Float = 2,
    Char = 3,
    String = 4,
}

impl Show {
    /// A value of one slot; a tuple or a struct is shown by `value_pieces`.
    pub fn of(t: &Type) -> Show {
        match t {
            Type::Int => Show::Int,
            Type::Bool => Show::Bool,
            Type::Float => Show::Float,
            Type::Char => Show::Char,
            Type::String => Show::String,
            other => unreachable!("`{}` is not shown in one slot", other.text()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Conversion {
    /// `n.to_float()`
    IntToFloat,
    /// `x.to_int()`: drops the decimals; error if NaN or too big.
    FloatToInt,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Op {
    Plus,
    Minus,
    Times,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

pub enum End {
    Jump(BlockId),
    Branch(Local, BlockId, BlockId),
    Return(Slots),
    /// Never reached (the type checker guarantees a `return` before).
    Unreachable,
}

/// Runtime errors.
#[derive(Clone, Copy)]
pub enum Fault {
    Overflow,
    DivisionByZero,
    BadConversion,
}

impl Fault {
    /// Its message, a key of `locale::RUNTIME`.
    pub fn key(self) -> &'static str {
        match self {
            Fault::Overflow => "fault_overflow",
            Fault::DivisionByZero => "fault_division_by_zero",
            Fault::BadConversion => "fault_bad_conversion",
        }
    }
}

/// A Float the way Kova shows it: `3.0`, `0.30000000000000004`, `1e16`, `NaN`, `-inf`.
///
/// The shortest digits that give back exactly the number (on a tie, the even digit,
/// like Python). Scientific notation below 0.0001 and from 10^16 on.
/// The C runtime (`write_float`) does exactly the same computation, step by step.
pub fn float_text(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    let a = x.abs();
    let (digits, exp) = shortest(a);
    let digits: String = digits.iter().map(|d| (b'0' + d) as char).collect();
    let n = digits.len() as i32;
    let body = if a == 0.0 || (1e-4..1e16).contains(&a) {
        if exp >= 0 {
            let int_part: String =
                (0..=exp).map(|i| digits.as_bytes().get(i as usize).map_or('0', |&c| c as char)).collect();
            let decimals = if n > exp + 1 { &digits[exp as usize + 1..] } else { "0" };
            format!("{int_part}.{decimals}")
        } else {
            format!("0.{}{digits}", "0".repeat((-exp - 1) as usize))
        }
    } else if n > 1 {
        format!("{}.{}e{exp}", &digits[..1], &digits[1..])
    } else {
        format!("{digits}e{exp}")
    };
    format!("{}{body}", if x.is_sign_negative() { "-" } else { "" })
}

/// The shortest digits of `a` (≥ 0) that give back `a`, and the exponent of the first one.
fn shortest(a: f64) -> (Vec<u8>, i32) {
    if a == 0.0 {
        return (vec![0], 0);
    }
    // The exact digits: a Float has at most ~770 of them, `{:.800e}` gives them all.
    let s = format!("{a:.800e}");
    let (m, e) = s.split_once('e').unwrap();
    let mut exact: Vec<u8> = m.bytes().filter(|&c| c != b'.').map(|c| c - b'0').collect();
    while exact.len() > 1 && exact.last() == Some(&0) {
        exact.pop();
    }
    let e: i32 = e.parse().unwrap();
    let reads_back = |d: &[u8], e: i32| {
        let d: String = d.iter().map(|d| (b'0' + d) as char).collect();
        format!("{}.{}e{e}", &d[..1], &d[1..]).parse::<f64>().unwrap() == a
    };
    for p in 1..=17 {
        if exact.len() <= p {
            return (exact, e);
        }
        // The two p-digit numbers around `a`: `low` (truncated) and `high`.
        let low = exact[..p].to_vec();
        let rest = &exact[p..];
        let high = plus_one(&low, e);
        let round_up = rest[0] > 5 || (rest[0] == 5 && (rest[1..].iter().any(|&d| d != 0) || low[p - 1] % 2 == 1));
        let (near, far) = if round_up { (high, (low, e)) } else { ((low, e), high) };
        for (mut d, e) in [near, far] {
            if reads_back(&d, e) {
                while d.len() > 1 && d.last() == Some(&0) {
                    d.pop();
                }
                return (d, e);
            }
        }
    }
    unreachable!("17 digits are always enough")
}

/// `digits + 1` on the last digit, with carry (`999` → `100` and exponent + 1).
fn plus_one(digits: &[u8], e: i32) -> (Vec<u8>, i32) {
    let mut d = digits.to_vec();
    for x in d.iter_mut().rev() {
        if *x < 9 {
            *x += 1;
            return (d, e);
        }
        *x = 0;
    }
    d.insert(0, 1);
    d.pop();
    (d, e + 1)
}

pub fn fault_text(f: Fault, file: &str, l: Loc) -> String {
    let text = |k| locale::text(k, &[]);
    format!("{}{}\n  --> {file}:{}:{}\n", text("error_prefix"), text(f.key()), l.line, l.col)
}

// ---- Translation AST → KIR ----

/// `contracts` = false for `kova build --boost`: `require`, `ensure` and `check` disappear.
/// `tests` = true for `kova test`: each `test` block becomes one more function.
/// `built` = true for `kova build`: the value of `mode = build`.
pub fn translate(f: &ast::File, info: &Info, src: &str, contracts: bool, tests: bool, built: bool) -> Program {
    // Every function: the free ones, then the structs' methods (`Dog.sound`), then the tests
    // (a test = a function without parameters, result or contracts).
    type Source<'f> = (String, Option<(Mode, Type)>, &'f [ast::Param], Option<&'f ast::TypeAst>, &'f [Contract], &'f Block);
    let mut all: Vec<Source> =
        f.functions.iter().map(|g| (g.name.text.clone(), None, &g.params[..], g.ret.as_ref(), &g.contracts[..], &g.body)).collect();
    for s in &f.structs {
        let ty = Type::Struct(info.structs[&s.name.text].clone());
        for m in &s.methods {
            let name = method_name(&s.name.text, &m.name.text);
            all.push((name, m.receiver.map(|r| (r, ty.clone())), &m.params, m.ret.as_ref(), &m.contracts, &m.body));
        }
    }
    let signatures: HashMap<String, (Option<Mode>, Vec<Mode>)> =
        all.iter().map(|g| (g.0.clone(), (g.1.as_ref().map(|r| r.0), g.2.iter().map(|p| p.mode).collect()))).collect();
    let n = all.len();
    all.extend(f.tests.iter().filter(|_| tests).map(|t| (t.name.clone(), None, &[][..], None, &[][..], &t.body)));
    let numbers: HashMap<&str, usize> = all[..n].iter().enumerate().map(|(i, g)| (g.0.as_str(), i)).collect();
    // The functions that receive the call location (`main` is called by the runtime).
    let sites: Vec<bool> = all.iter().map(|g| contracts && !g.4.is_empty() && g.0 != "main").collect();
    let functions = all
        .iter()
        .enumerate()
        .map(|(i, (name, receiver, params, ret, written_contracts, body))| {
            let ret = ret.map_or(Type::Unit, |t| type_of(t, &info.structs));
            let kept = |k: ContractKind| written_contracts.iter().filter(move |c: &&Contract| contracts && c.kind == k);
            let mut t = Translator {
                src,
                info,
                numbers: &numbers,
                signatures: &signatures,
                sites: &sites,
                contracts,
                built,
                ensures: kept(ContractKind::Ensure).collect(),
                ret: ret.clone(),
                result: None,
                site: None,
                outs: vec![],
                // 0: the borrowed parameters; 1: the owned ones (`move`), freed at each exit.
                names: vec![HashMap::new(), HashMap::new()],
                locals: vec![],
                blocks: vec![],
                current: 0,
                loops: vec![],
                temps: vec![],
            };
            let mut returns = scalars(&ret);
            if let Some((r, ty)) = receiver {
                let v = t.new_slots(ty);
                t.parameter("self", *r, v, ty.clone(), &mut returns);
            }
            for p in params.iter() {
                let ty = type_of(&p.ty, &info.structs);
                let v = t.new_slots(&ty);
                t.parameter(&p.name.text, p.mode, v, ty, &mut returns);
            }
            t.site = sites[i].then(|| (t.int_local(), t.int_local()));
            let nb_params = t.locals.len();
            t.current = t.new_block();
            for c in kept(ContractKind::Require) {
                t.check(c, t.site);
            }
            t.block(body);
            if ret == Type::Unit {
                t.exit(vec![]);
            } else {
                t.finish(End::Unreachable);
            }
            Function {
                name: name.clone(),
                nb_params,
                locals: t.locals,
                returns,
                blocks: t.blocks.into_iter().map(|(insts, end)| KBlock { insts, end: end.unwrap_or(End::Unreachable) }).collect(),
            }
        })
        .collect();
    let tests = all[n..].iter().enumerate().map(|(i, t)| (t.0.clone(), n + i)).collect();
    Program { functions, main: numbers["main"], tests }
}

struct Translator<'a> {
    src: &'a str,
    info: &'a Info,
    numbers: &'a HashMap<&'a str, usize>,
    /// How each function receives its value (`None`: not a method) and its parameters,
    /// by name (`f`, `Dog.sound`).
    signatures: &'a HashMap<String, (Option<Mode>, Vec<Mode>)>,
    /// By function number: does it receive the call location?
    sites: &'a [bool],
    /// Keep contracts (false with `--boost`).
    contracts: bool,
    /// `kova build` (true) or `kova run` / `kova test` (false).
    built: bool,
    ensures: Vec<&'a Contract>,
    ret: Type,
    /// The value of `result` while checking an `ensure`.
    result: Option<(Slots, Type)>,
    /// The 2 hidden parameters: line and column of the call (see `sites`).
    site: Option<(Local, Local)>,
    /// The slots of `var self` and of the `var` parameters, given back after the result at each exit.
    outs: Slots,
    /// Name → its slots and type, one table per open block.
    names: Vec<HashMap<String, (Slots, Type)>>,
    locals: Vec<Scalar>,
    /// Blocks being built; `None` = end not set yet.
    blocks: Vec<(Vec<Inst>, Option<End>)>,
    current: BlockId,
    /// For each open loop: (target of `continue`, target of `break`, number of open blocks).
    loops: Vec<(BlockId, BlockId, usize)>,
    /// The Strings made by the current statement and not given to anyone yet:
    /// freed at the end of the statement.
    temps: Vec<Local>,
}

impl<'a> Translator<'a> {
    /// A parameter (or `self`) received in these slots.
    fn parameter(&mut self, name: &str, mode: Mode, v: Slots, ty: Type, returns: &mut Vec<Scalar>) {
        match mode {
            // Taken: freed by the function (scope 1 is freed at each exit).
            Mode::Move => {
                self.names[1].insert(name.to_string(), (v, ty));
            }
            // Changed: given back to the caller at each exit, after the result.
            Mode::Var => {
                self.outs.extend(&v);
                returns.extend(scalars(&ty));
                self.names[0].insert(name.to_string(), (v, ty));
            }
            Mode::Read => {
                self.names[0].insert(name.to_string(), (v, ty));
            }
        }
    }

    fn new_local(&mut self, s: Scalar) -> Local {
        self.locals.push(s);
        self.locals.len() - 1
    }
    fn int_local(&mut self) -> Local {
        self.new_local(Scalar::Int)
    }
    /// Fresh slots for a value of type `t`.
    fn new_slots(&mut self, t: &Type) -> Slots {
        scalars(t).into_iter().map(|s| self.new_local(s)).collect()
    }
    fn new_block(&mut self) -> BlockId {
        self.blocks.push((vec![], None));
        self.blocks.len() - 1
    }
    fn emit(&mut self, i: Inst) {
        self.blocks[self.current].0.push(i);
    }
    /// Copies slot by slot.
    fn copy(&mut self, dst: &[Local], src: &[Local]) {
        for (&d, &s) in dst.iter().zip(src) {
            self.emit(Inst::Copy(d, s));
        }
    }
    /// Sets the end of the current block, then continues in a fresh block
    /// (code after a `return` or a `break` lands there, never executed).
    fn finish(&mut self, end: End) {
        self.blocks[self.current].1 = Some(end);
        self.current = self.new_block();
    }
    /// Sets the end of the current block and continues in `next`.
    fn go(&mut self, end: End, next: BlockId) {
        self.blocks[self.current].1 = Some(end);
        self.current = next;
    }
    fn loc(&self, s: Span) -> Loc {
        let (line, col) = line_col(self.src, s.start);
        Loc { line, col }
    }
    fn variable(&self, name: &str) -> (Slots, Type) {
        self.names.iter().rev().find_map(|t| t.get(name)).expect("variable checked by the type checker").clone()
    }
    fn slots_of(&self, name: &str) -> Slots {
        self.variable(name).0
    }

    /// A new value (call result, `a + b`…): its Strings are temporaries until someone takes them.
    fn track(&mut self, v: &[Local], ty: &Type) {
        let s = strings(ty);
        self.temps.extend(v.iter().zip(s).filter(|(_, s)| *s).map(|(&l, _)| l));
    }
    fn untrack(&mut self, v: &[Local]) {
        self.temps.retain(|l| !v.contains(l));
    }
    /// Frees the temporaries made since `mark`.
    fn free_temps(&mut self, mark: usize) {
        let temps: Vec<Local> = self.temps.drain(mark..).collect();
        for l in temps.into_iter().rev() {
            self.emit(Inst::Free(l));
        }
    }
    fn free_value(&mut self, v: &[Local], ty: &Type) {
        for (&l, s) in v.iter().zip(strings(ty)).rev() {
            if s {
                self.emit(Inst::Free(l));
            }
        }
    }
    /// Frees the variables of the blocks `names[from..]` (from the innermost).
    fn free_scopes(&mut self, from: usize) {
        let vars: Vec<(Slots, Type)> = self.names[from..].iter().rev().flat_map(|t| t.values().cloned()).collect();
        for (v, ty) in vars {
            self.free_value(&v, &ty);
        }
    }

    /// `e` is given (`let`, `:=`, `return`…): the value now belongs to the receiver.
    /// A moved variable (marked by `ownership.rs`) is emptied, so that its block frees nothing.
    fn owned(&mut self, e: &Expr) -> Slots {
        if !self.info.moves[e.id] {
            let v = self.expr(e);
            self.untrack(&v);
            return v;
        }
        let ExprKind::Var(name) = &e.kind else { unreachable!("only a variable is moved") };
        let (src, ty) = self.variable(name);
        let v = self.new_slots(&ty);
        self.copy(&v, &src);
        for (&l, s) in src.iter().zip(strings(&ty)) {
            if s {
                self.emit(Inst::Const(l, 0));
            }
        }
        v
    }

    /// A condition: its temporaries are freed right away (a `while` computes it at each turn).
    fn cond1(&mut self, e: &Expr) -> Local {
        let mark = self.temps.len();
        let c = self.expr1(e);
        self.free_temps(mark);
        c
    }

    /// Gives their slots to the names of a pattern: `(a, b)` on 2 slots → `a` the 1st, `b` the 2nd.
    fn bind(&mut self, p: &Pattern, slots: &[Local], ty: &Type) {
        match p {
            Pattern::Name(n) => {
                self.names.last_mut().unwrap().insert(n.text.clone(), (slots.to_vec(), ty.clone()));
            }
            // Nobody receives this part: freed at once.
            Pattern::Ignore(_) => self.free_value(slots, ty),
            Pattern::Tuple(ps, _) => {
                for (i, p) in ps.iter().enumerate() {
                    let (start, n) = slice_of(ty, i);
                    self.bind(p, &slots[start..start + n], &element(ty, i));
                }
            }
        }
    }

    /// The slots designated by a `:=` target, and their type.
    fn target_slots(&self, t: &Target) -> (Slots, Type) {
        match t {
            Target::Tuple(ts, _) => {
                let parts: Vec<(Slots, Type)> = ts.iter().map(|t| self.target_slots(t)).collect();
                let slots = parts.iter().flat_map(|(s, _)| s.clone()).collect();
                (slots, Type::Tuple(parts.into_iter().map(|(_, t)| t).collect()))
            }
            Target::Place { name, fields, .. } => {
                let (mut slots, mut ty) = self.variable(&name.text);
                for step in fields {
                    let i = match step {
                        Step::Index(i) => *i,
                        Step::Name(n) => field_index(&ty, &n.text),
                    };
                    let (start, n) = slice_of(&ty, i);
                    slots = slots[start..start + n].to_vec();
                    ty = element(&ty, i);
                }
                (slots, ty)
            }
        }
    }
    fn constant(&mut self, v: i64) -> Local {
        let l = self.int_local();
        self.emit(Inst::Const(l, v));
        l
    }
    fn float_constant(&mut self, v: f64) -> Local {
        let l = self.new_local(Scalar::Float);
        self.emit(Inst::Const(l, v.to_bits() as i64));
        l
    }
    fn is_float(&self, e: &Expr) -> bool {
        self.info.types[e.id] == Type::Float
    }
    /// `dst := a op b`, as integers or floats depending on the type of `a`.
    fn operation(&mut self, op: Op, a: Local, b: Local, float: bool, loc: Loc) -> Local {
        let float_result = float && matches!(op, Op::Plus | Op::Minus | Op::Times | Op::Div);
        let dst = self.new_local(if float_result { Scalar::Float } else { Scalar::Int });
        self.emit(if float { Inst::FBinary(dst, op, a, b, loc) } else { Inst::Binary(dst, op, a, b, loc) });
        dst
    }

    fn block(&mut self, b: &Block) {
        self.names.push(HashMap::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        self.free_scopes(self.names.len() - 1);
        self.names.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        let mark = self.temps.len();
        self.statement(s);
        self.free_temps(mark);
    }

    fn statement(&mut self, s: &Stmt) {
        match s {
            Stmt::Declare { pattern, value, .. } => {
                let v = self.owned(value);
                let ty = self.info.types[value.id].clone();
                let slots = self.new_slots(&ty);
                self.copy(&slots, &v);
                self.bind(pattern, &slots, &ty);
            }
            Stmt::Assign { target, op, value } => {
                let (slots, ty) = self.target_slots(target);
                match op {
                    // `(a, b) := (b, a)`: read everything before writing, otherwise `a` would be overwritten too early.
                    None => {
                        let v = self.owned(value);
                        let read = self.new_slots(&ty);
                        self.copy(&read, &v);
                        self.free_value(&slots, &ty);
                        self.copy(&slots, &read);
                    }
                    Some(BinOp::Plus) if ty == Type::String => {
                        let v = self.expr1(value);
                        let r = self.int_local();
                        self.emit(Inst::Str(r, StrOp::Concat, vec![slots[0], v]));
                        self.emit(Inst::Free(slots[0]));
                        self.emit(Inst::Copy(slots[0], r));
                    }
                    Some(op) => {
                        let v = self.expr(value);
                        let loc = self.loc(target.span());
                        let r = self.operation(kir_op(*op), slots[0], v[0], self.is_float(value), loc);
                        self.emit(Inst::Copy(slots[0], r));
                    }
                }
            }
            Stmt::Return { value, .. } => {
                let v = value.as_ref().map_or(vec![], |e| self.owned(e));
                self.exit(v);
            }
            Stmt::Check(c) => {
                if self.contracts {
                    self.check(c, None);
                }
            }
            Stmt::Break(_) => {
                let (_, exit, depth) = *self.loops.last().unwrap();
                self.free_scopes(depth);
                self.finish(End::Jump(exit));
            }
            Stmt::Continue(_) => {
                let (next, _, depth) = *self.loops.last().unwrap();
                self.free_scopes(depth);
                self.finish(End::Jump(next));
            }
            Stmt::If { cond, then, otherwise } => {
                // `if mode = build`: only the branch of this mode is translated.
                if let Some(yes) = self.known(cond) {
                    match (yes, otherwise) {
                        (true, _) => self.block(then),
                        (false, Some(o)) => self.block(o),
                        (false, None) => {}
                    }
                    return;
                }
                let c = self.cond1(cond);
                let (b_then, b_else, b_after) = (self.new_block(), self.new_block(), self.new_block());
                self.go(End::Branch(c, b_then, b_else), b_then);
                self.block(then);
                self.go(End::Jump(b_after), b_else);
                if let Some(o) = otherwise {
                    self.block(o);
                }
                self.go(End::Jump(b_after), b_after);
            }
            Stmt::While { cond, body } => {
                let (head, b_body, exit) = (self.new_block(), self.new_block(), self.new_block());
                self.go(End::Jump(head), head);
                let c = self.cond1(cond);
                self.go(End::Branch(c, b_body, exit), b_body);
                self.loops.push((head, exit, self.names.len()));
                self.block(body);
                self.loops.pop();
                self.go(End::Jump(head), exit);
            }
            Stmt::Loop { counter, start, end, inclusive, body } => {
                // The end bound is computed only once, before the loop.
                let s = self.expr1(start);
                let i = self.int_local();
                self.emit(Inst::Copy(i, s));
                let e = self.expr1(end);
                let bound = self.int_local();
                self.emit(Inst::Copy(bound, e));

                let (head, b_body, next, increment, exit) =
                    (self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block());
                let loc = self.loc(counter.span);
                self.go(End::Jump(head), head);
                let c = self.int_local();
                let test = if *inclusive { Op::Le } else { Op::Lt };
                self.emit(Inst::Binary(c, test, i, bound, loc));
                self.go(End::Branch(c, b_body, exit), b_body);

                self.loops.push((next, exit, self.names.len()));
                self.names.push(HashMap::from([(counter.text.clone(), (vec![i], Type::Int))]));
                self.block(body);
                self.names.pop();
                self.loops.pop();
                self.go(End::Jump(next), next);

                // `continue` lands here. For `..=`, exit BEFORE adding 1 when on
                // the bound: `from i := 1 to 9223372036854775807` doesn't overflow.
                if *inclusive {
                    let last = self.int_local();
                    self.emit(Inst::Binary(last, Op::Eq, i, bound, loc));
                    self.go(End::Branch(last, exit, increment), increment);
                } else {
                    self.go(End::Jump(increment), increment);
                }
                let one = self.constant(1);
                self.emit(Inst::Binary(i, Op::Plus, i, one, loc));
                self.go(End::Jump(head), exit);
            }
            Stmt::Expr(e) => {
                self.expr(e);
            }
        }
    }

    /// `return v`: frees what the function still owns, checks the `ensure`s with
    /// `result` = v, then exits. The parameters are borrowed: the caller frees them.
    fn exit(&mut self, v: Slots) {
        // The temporaries of the enclosing statements (a loop bound…) are freed on this path
        // only: they stay listed, the normal path frees them at the end of their statement.
        for l in self.temps.clone().into_iter().rev() {
            self.emit(Inst::Free(l));
        }
        // The `ensure`s may read a `move` parameter: checked before it is freed.
        for c in self.ensures.clone() {
            self.result = Some((v.clone(), self.ret.clone()));
            self.check(c, self.site);
        }
        self.result = None;
        self.free_scopes(1);
        self.finish(End::Return([v, self.outs.clone()].concat()));
    }

    /// Tests a contract's condition; if false: `Fail` with a message like
    ///
    ///   error: you cannot divide by zero
    ///     --> divide.kova:2:13
    ///     require b != 0
    ///     values: b = 0
    ///     called from divide.kova:8:11
    fn check(&mut self, c: &Contract, site: Option<(Local, Local)>) {
        let ok = self.cond1(&c.cond);
        let (fail, next) = (self.new_block(), self.new_block());
        self.go(End::Branch(ok, next, fail), fail);

        let l = self.loc(c.cond.span);
        let cond: Vec<&str> = self.src[c.cond.span.start..c.cond.span.end].split_whitespace().collect();
        // The `{values}` of the message are computed here, only when the contract fails;
        // the program stops right after, so its temporaries are not freed.
        let mark = self.temps.len();
        let mut m = vec![Piece::Label("error_prefix")];
        match &c.message {
            Some(parts) => {
                for p in parts {
                    self.part_pieces(p, &mut m);
                }
            }
            None => m.push(Piece::Label("contract_violated")),
        }
        self.temps.truncate(mark);
        m.extend([
            Piece::Text("\n  --> ".into()),
            Piece::File,
            Piece::Text(format!(":{}:{}\n  {} {}\n", l.line, l.col, c.kind.keyword(), cond.join(" "))),
        ]);
        let mut names = vec![];
        names_in(&c.cond, &mut names);
        let values: Vec<(String, (Slots, Type))> = names
            .into_iter()
            .filter_map(|n| {
                let v = if n == "result" { self.result.clone() } else { self.names.iter().rev().find_map(|t| t.get(&n)).cloned() };
                Some((n, v?))
            })
            .collect();
        for (k, (name, (v, ty))) in values.iter().enumerate() {
            if k == 0 {
                m.extend([Piece::Text("  ".into()), Piece::Label("contract_values")]);
            } else {
                m.push(Piece::Text(", ".into()));
            }
            m.push(Piece::Text(format!("{name} = ")));
            value_pieces(v, ty, &mut m);
        }
        if !values.is_empty() {
            m.push(Piece::Text("\n".into()));
        }
        if let Some((line, col)) = site {
            m.extend([
                Piece::Text("  ".into()),
                Piece::Label("contract_called_from"),
                Piece::File,
                Piece::Text(":".into()),
                Piece::Value(line, Show::Int),
                Piece::Text(":".into()),
                Piece::Value(col, Show::Int),
                Piece::Text("\n".into()),
            ]);
        }
        self.emit(Inst::Fail(m));
        self.go(End::Unreachable, next);
    }

    /// The pieces of a part of a text: a String or a number as `print` shows it, a struct
    /// as `Dog { name: "Rex", age: 3 }`, a tuple as `(1, "a")`.
    fn part_pieces(&mut self, p: &Part, m: &mut Vec<Piece>) {
        match p {
            Part::Text(t) => m.push(Piece::Text(t.clone())),
            Part::Code(e) => match &self.info.types[e.id] {
                ty @ (Type::Struct(_) | Type::Tuple(_)) => {
                    let ty = ty.clone();
                    let v = self.expr(e);
                    value_pieces(&v, &ty, m);
                }
                ty => {
                    let show = Show::of(ty);
                    m.push(Piece::Value(self.expr1(e), show));
                }
            },
        }
    }

    /// A new String made of these pieces: `"a{x}b"` = ("a" + text of x) + "b",
    /// each intermediate String freed at once.
    fn join(&mut self, pieces: Vec<Piece>) -> Local {
        let mut whole: Option<Local> = None;
        let mut pieces = pieces.into_iter().peekable();
        while let Some(p) = pieces.next() {
            let (piece, fresh) = match p {
                Piece::Text(mut t) => {
                    while let Some(Piece::Text(next)) = pieces.peek() {
                        t += next;
                        pieces.next();
                    }
                    let l = self.int_local();
                    self.emit(Inst::Text(l, t));
                    (l, true)
                }
                Piece::Value(v, Show::String) => (v, false),
                Piece::Value(v, show) => {
                    let l = self.int_local();
                    self.emit(Inst::Str(l, StrOp::Format(show), vec![v]));
                    (l, true)
                }
                Piece::Label(_) | Piece::File => unreachable!("only in contract messages"),
            };
            let l = match whole {
                None if fresh => piece,
                _ => self.int_local(),
            };
            match whole {
                None if fresh => {}
                None => self.emit(Inst::Str(l, StrOp::Clone, vec![piece])),
                Some(w) => {
                    self.emit(Inst::Str(l, StrOp::Concat, vec![w, piece]));
                    self.emit(Inst::Free(w));
                    if fresh {
                        self.emit(Inst::Free(piece));
                    }
                }
            }
            whole = Some(l);
        }
        whole.expect("a text has at least one part")
    }

    /// The values of `callee`'s arguments, added to `vals`. A `move` parameter takes its
    /// argument; a `var` one changes a place: (its slots, slots for what the call gives back)
    /// goes into `places`.
    fn arguments(&mut self, callee: &str, args: &[Expr], vals: &mut Slots, places: &mut Vec<(Slots, Slots)>) {
        let modes = self.signatures[callee].1.clone();
        for (a, mode) in args.iter().zip(modes) {
            let value = if let ExprKind::Mark(_, x) = &a.kind { x } else { a };
            match mode {
                Mode::Read => vals.extend(self.expr(value)),
                Mode::Move => vals.extend(self.owned(value)),
                Mode::Var => {
                    let v = self.expr(value);
                    let out = self.new_slots(&self.info.types[value.id].clone());
                    vals.extend(&v);
                    places.push((v, out));
                }
            }
        }
    }

    /// `results := f(vals)`, with the call location for a function that has contracts.
    /// `places`: what `var self` and the `var` parameters change, written back after the call
    /// (copy-in / copy-out, like Swift's `inout`).
    fn call(&mut self, name: &str, mut vals: Slots, places: Vec<(Slots, Slots)>, span: Span, ty: &Type) -> Slots {
        let num = self.numbers[name];
        if self.sites[num] {
            let l = self.loc(span);
            let (line, col) = (self.constant(l.line as i64), self.constant(l.col as i64));
            vals.extend([line, col]);
        }
        let result = self.new_slots(ty);
        let outs: Slots = places.iter().flat_map(|(_, out)| out.clone()).collect();
        self.emit(Inst::Call([result.clone(), outs].concat(), num, vals));
        for (place, out) in &places {
            self.copy(place, out);
        }
        self.track(&result, ty);
        result
    }

    /// `rex.sound`: the struct method called (its name, `Dog.sound`, and how it receives `rex`).
    fn method(&self, receiver: &Expr, name: &ast::Name) -> Option<(String, Mode)> {
        let Type::Struct(d) = &self.info.types[receiver.id] else { return None };
        let q = method_name(&d.name, &name.text);
        let r = self.signatures.get(&q)?.0.expect("static function called on a value: refused by the type checker");
        Some((q, r))
    }

    /// The value of a condition known before running: `mode = build`, `!(mode = run)`.
    fn known(&self, e: &Expr) -> Option<bool> {
        match &e.kind {
            ExprKind::InMode { build, negated } => Some((*build == self.built) != *negated),
            ExprKind::Unary(UnOp::Not, x) => self.known(x).map(|b| !b),
            _ => None,
        }
    }

    fn if_value(&mut self, cond: &Expr, then: &Block, otherwise: &Block, ty: &Type) -> Slots {
        if let Some(yes) = self.known(cond) {
            let v = self.block_value(if yes { then } else { otherwise }, ty);
            self.track(&v, ty);
            return v;
        }
        let dst = self.new_slots(ty);
        let c = self.cond1(cond);
        let (b_then, b_else, b_after) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Branch(c, b_then, b_else), b_then);
        let v = self.block_value(then, ty);
        self.copy(&dst, &v);
        self.go(End::Jump(b_after), b_else);
        let v = self.block_value(otherwise, ty);
        self.copy(&dst, &v);
        self.go(End::Jump(b_after), b_after);
        self.track(&dst, ty);
        dst
    }

    /// The value of a block: that of its last line (checked by the type checker).
    fn block_value(&mut self, b: &Block, ty: &Type) -> Slots {
        self.names.push(HashMap::new());
        let (last, before) = b.stmts.split_last().expect("block checked by the type checker");
        for s in before {
            self.stmt(s);
        }
        let v = match last {
            Stmt::Expr(e) => self.owned(e),
            Stmt::If { cond, then, otherwise: Some(otherwise) } => {
                let v = self.if_value(cond, then, otherwise, ty);
                self.untrack(&v);
                v
            }
            _ => unreachable!("block checked by the type checker"),
        };
        self.free_scopes(self.names.len() - 1);
        self.names.pop();
        v
    }

    /// An expression whose value fits in a single slot (Int, Bool…).
    fn expr1(&mut self, e: &Expr) -> Local {
        self.expr(e)[0]
    }

    fn expr(&mut self, e: &Expr) -> Slots {
        match &e.kind {
            ExprKind::Int(v) if self.info.types[e.id] == Type::Float => vec![self.float_constant(*v as f64)],
            ExprKind::Int(v) => vec![self.constant(*v as i64)],
            ExprKind::Float(v) => vec![self.float_constant(*v)],
            ExprKind::Char(c) => vec![self.constant(*c as i64)],
            ExprKind::Bool(b) => vec![self.constant(*b as i64)],
            ExprKind::Str(text) => {
                let l = self.int_local();
                self.emit(Inst::Text(l, text.clone()));
                self.temps.push(l);
                vec![l]
            }
            ExprKind::Interp(parts) => {
                let mut pieces = vec![];
                for p in parts {
                    self.part_pieces(p, &mut pieces);
                }
                let l = self.join(pieces);
                self.temps.push(l);
                vec![l]
            }
            ExprKind::StructLit { fields, .. } => {
                // Computed in the written order, placed in the declared order.
                let ty = self.info.types[e.id].clone();
                let mut parts: Vec<Slots> = vec![vec![]; members(&ty).len()];
                for (name, value) in fields {
                    parts[field_index(&ty, &name.text)] = self.owned(value);
                }
                let v: Slots = parts.concat();
                self.track(&v, &ty);
                v
            }
            ExprKind::InMode { .. } => vec![self.constant(self.known(e).unwrap() as i64)],
            ExprKind::Var(name) => self.slots_of(name),
            ExprKind::Mark(_, x) => self.expr(x),
            ExprKind::Result => self.result.clone().expect("`result` checked by the type checker").0,
            ExprKind::Tuple(elems) => {
                let v: Slots = elems.iter().flat_map(|x| self.owned(x)).collect();
                self.track(&v, &self.info.types[e.id].clone());
                v
            }
            ExprKind::Field(x, i) => {
                let v = self.expr(x);
                let (start, n) = slice_of(&self.info.types[x.id], *i);
                v[start..start + n].to_vec()
            }
            ExprKind::Call { name, args } => {
                if let n @ ("print" | "write") = name.text.as_str() {
                    let vals = self.expr(&args[0]);
                    let newline = n == "print";
                    match &self.info.types[args[0].id] {
                        ty @ (Type::Struct(_) | Type::Tuple(_)) => {
                            let mut pieces = vec![];
                            value_pieces(&vals, ty, &mut pieces);
                            let text = self.join(pieces);
                            self.emit(Inst::Print { value: text, show: Show::String, newline });
                            self.emit(Inst::Free(text));
                        }
                        ty => self.emit(Inst::Print { value: vals[0], show: Show::of(ty), newline }),
                    }
                    return vec![];
                }
                let (mut vals, mut places) = (vec![], vec![]);
                self.arguments(&name.text, args, &mut vals, &mut places);
                let ty = self.info.types[e.id].clone();
                self.call(&name.text, vals, places, name.span, &ty)
            }
            ExprKind::Static { ty, name, args } => {
                let q = method_name(&ty.text, &name.text);
                let (mut vals, mut places) = (vec![], vec![]);
                self.arguments(&q, args, &mut vals, &mut places);
                let result = self.info.types[e.id].clone();
                self.call(&q, vals, places, name.span, &result)
            }
            ExprKind::Unary(UnOp::Neg, x) => {
                // A number written with `-` in front is a constant (handles `-9223372036854775808`).
                match x.kind {
                    ExprKind::Int(v) if self.info.types[e.id] == Type::Float => return vec![self.float_constant(-(v as f64))],
                    ExprKind::Int(v) => return vec![self.constant((v as i64).wrapping_neg())],
                    ExprKind::Float(v) => return vec![self.float_constant(-v)],
                    _ => {}
                }
                let a = self.expr1(x);
                if self.is_float(x) {
                    let dst = self.new_local(Scalar::Float);
                    self.emit(Inst::FNeg(dst, a));
                    return vec![dst];
                }
                let dst = self.int_local();
                let loc = self.loc(e.span);
                self.emit(Inst::Neg(dst, a, loc));
                vec![dst]
            }
            ExprKind::Unary(UnOp::Not, x) => {
                let a = self.expr1(x);
                let dst = self.int_local();
                self.emit(Inst::Not(dst, a));
                vec![dst]
            }
            ExprKind::If { cond, then, otherwise } => {
                let ty = self.info.types[e.id].clone();
                self.if_value(cond, then, otherwise, &ty)
            }
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                // Short circuit: `b` is computed only if `a` is not enough.
                let dst = self.int_local();
                let va = self.expr1(a);
                self.emit(Inst::Copy(dst, va));
                let (compute_b, after) = (self.new_block(), self.new_block());
                let end = if *op == BinOp::And { End::Branch(va, compute_b, after) } else { End::Branch(va, after, compute_b) };
                self.go(end, compute_b);
                let vb = self.expr1(b);
                self.emit(Inst::Copy(dst, vb));
                self.go(End::Jump(after), after);
                vec![dst]
            }
            ExprKind::Binary(op @ (BinOp::Plus | BinOp::Eq | BinOp::Ne), a, b) if self.info.types[a.id] == Type::String => {
                let (va, vb) = (self.expr1(a), self.expr1(b));
                let dst = self.int_local();
                if *op == BinOp::Plus {
                    self.emit(Inst::Str(dst, StrOp::Concat, vec![va, vb]));
                    self.temps.push(dst);
                    return vec![dst];
                }
                self.emit(Inst::Str(dst, StrOp::Eq, vec![va, vb]));
                if *op == BinOp::Eq {
                    return vec![dst];
                }
                let ne = self.int_local();
                self.emit(Inst::Not(ne, dst));
                vec![ne]
            }
            ExprKind::Binary(op, a, b) => {
                let (va, vb) = (self.expr1(a), self.expr1(b));
                // The error points at the operator: the first non-blank character after `a`.
                let after_a = &self.src[a.span.end..];
                let pos = a.span.end + after_a.len() - after_a.trim_start().len();
                let loc = self.loc(Span::new(pos, pos));
                vec![self.operation(kir_op(*op), va, vb, self.is_float(a), loc)]
            }
            ExprKind::Method { receiver, name, .. } if self.info.types[receiver.id].is_field(&name.text) => {
                let ty = self.info.types[receiver.id].clone();
                let v = self.expr(receiver);
                let (start, n) = slice_of(&ty, field_index(&ty, &name.text));
                v[start..start + n].to_vec()
            }
            ExprKind::Method { receiver, name, args } if self.method(receiver, name).is_some() => {
                let (q, r) = self.method(receiver, name).unwrap();
                let ty = self.info.types[receiver.id].clone();
                // `move self`: the method takes the value; otherwise it borrows it.
                let v = if r == Mode::Move { self.owned(receiver) } else { self.expr(receiver) };
                // `var self`: the method gives back the changed value, written into the caller's place.
                let mut places = vec![];
                if r == Mode::Var {
                    places.push((v.clone(), self.new_slots(&ty)));
                }
                let mut vals = v;
                self.arguments(&q, args, &mut vals, &mut places);
                let result = self.info.types[e.id].clone();
                self.call(&q, vals, places, name.span, &result)
            }
            ExprKind::Method { receiver, name, .. }
                if name.text == "clone" && matches!(self.info.types[receiver.id], Type::Struct(_) | Type::Tuple(_)) =>
            {
                let ty = self.info.types[receiver.id].clone();
                let v = self.expr(receiver);
                let copy = self.new_slots(&ty);
                for ((&d, &l), s) in copy.iter().zip(&v).zip(strings(&ty)) {
                    self.emit(if s { Inst::Str(d, StrOp::Clone, vec![l]) } else { Inst::Copy(d, l) });
                }
                self.track(&copy, &ty);
                copy
            }
            ExprKind::Method { receiver, name, .. } => {
                let v = self.expr1(receiver);
                let string = |op| match op {
                    "length" => StrOp::Length,
                    _ => StrOp::Clone,
                };
                if self.info.types[receiver.id] == Type::String {
                    let (dst, op) = (self.int_local(), string(name.text.as_str()));
                    self.emit(Inst::Str(dst, op, vec![v]));
                    if op == StrOp::Clone {
                        self.temps.push(dst);
                    }
                    return vec![dst];
                }
                let conversion = match (&self.info.types[receiver.id], name.text.as_str()) {
                    (Type::Int, "to_float") => Conversion::IntToFloat,
                    (Type::Float, "to_int") => Conversion::FloatToInt,
                    (Type::Char, "to_int") => return vec![v], // a Char already is its code
                    other => unreachable!("method checked by the type checker: {other:?}"),
                };
                let dst = self.new_slots(&self.info.types[e.id].clone())[0];
                let loc = self.loc(name.span);
                self.emit(Inst::Convert(dst, conversion, v, loc));
                vec![dst]
            }
        }
    }
}

/// The variables of a condition, in order, without duplicates (`result` included).
fn names_in(e: &Expr, names: &mut Vec<String>) {
    let mut add = |n: &str| {
        if !names.iter().any(|m| m == n) {
            names.push(n.to_string());
        }
    };
    match &e.kind {
        ExprKind::Var(n) => add(n),
        ExprKind::Result => add("result"),
        ExprKind::Call { args, .. } | ExprKind::Static { args, .. } | ExprKind::Tuple(args) => {
            args.iter().for_each(|a| names_in(a, names))
        }
        ExprKind::Method { receiver, args, .. } => {
            names_in(receiver, names);
            args.iter().for_each(|a| names_in(a, names));
        }
        ExprKind::Unary(_, x) | ExprKind::Field(x, _) | ExprKind::Mark(_, x) => names_in(x, names),
        ExprKind::StructLit { fields, .. } => fields.iter().for_each(|(_, v)| names_in(v, names)),
        ExprKind::If { cond, .. } => names_in(cond, names),
        ExprKind::Interp(parts) => parts.iter().for_each(|p| {
            if let Part::Code(x) = p {
                names_in(x, names);
            }
        }),
        ExprKind::Binary(_, a, b) => {
            names_in(a, names);
            names_in(b, names);
        }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::InMode { .. } => {}
    }
}

/// A value the way it would be written in Kova: `3`, `2.5`, `'a'`, `(1, true)`,
/// `Dog { name: "Rex", age: 3 }`.
fn value_pieces(v: &[Local], ty: &Type, m: &mut Vec<Piece>) {
    match ty {
        Type::Struct(d) if d.fields.is_empty() => m.push(Piece::Text(format!("{} {{}}", d.name))),
        Type::Struct(d) => {
            m.push(Piece::Text(format!("{} {{ ", d.name)));
            for (i, (name, t)) in d.fields.iter().enumerate() {
                let sep = if i > 0 { ", " } else { "" };
                m.push(Piece::Text(format!("{sep}{name}: ")));
                let (start, n) = slice_of(ty, i);
                value_pieces(&v[start..start + n], t, m);
            }
            m.push(Piece::Text(" }".into()));
        }
        Type::Tuple(ts) => {
            m.push(Piece::Text("(".into()));
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    m.push(Piece::Text(", ".into()));
                }
                let (start, n) = slice_of(ty, i);
                value_pieces(&v[start..start + n], t, m);
            }
            m.push(Piece::Text(")".into()));
        }
        Type::Char | Type::String => {
            let quote = if *ty == Type::Char { "'" } else { "\"" };
            m.push(Piece::Text(quote.into()));
            m.push(Piece::Value(v[0], Show::of(ty)));
            m.push(Piece::Text(quote.into()));
        }
        _ => m.push(Piece::Value(v[0], Show::of(ty))),
    }
}

fn kir_op(op: BinOp) -> Op {
    match op {
        BinOp::Plus => Op::Plus,
        BinOp::Minus => Op::Minus,
        BinOp::Times => Op::Times,
        BinOp::Div => Op::Div,
        BinOp::Rem => Op::Rem,
        BinOp::Eq => Op::Eq,
        BinOp::Ne => Op::Ne,
        BinOp::Lt => Op::Lt,
        BinOp::Le => Op::Le,
        BinOp::Gt => Op::Gt,
        BinOp::Ge => Op::Ge,
        BinOp::And | BinOp::Or | BinOp::Range | BinOp::RangeInclusive => unreachable!("handled elsewhere"),
    }
}
