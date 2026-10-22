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

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ast::{self, BinOp, Block, Contract, ContractKind, Expr, ExprKind, Match, Part, Pattern, Mode, Step, Stmt, Target, UnOp};
use crate::diag::{Span, line_col};
use crate::locale;
use crate::types::{EnumDef, Info, Type, enum_view, is_copy, method_name, narrowed, place_root, substitute, type_of};

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

/// A Map's element: its key and its value, side by side (a `Map<K, V>` = a `List<(K, V)>`).
fn pair(k: &Type, v: &Type) -> Type {
    Type::Tuple(vec![k.clone(), v.clone()])
}

fn element(t: &Type, i: usize) -> Type {
    members(t)[i].clone()
}

/// The number of the field `name` in a struct of type `t`.
fn field_index(t: &Type, name: &str) -> usize {
    let Type::Struct(d) = t else { unreachable!("struct checked by the type checker") };
    d.field(name).expect("field checked by the type checker").0
}


/// Where the fields of variant `i` lie in the slots of an enum value: (start, length).
/// Slot 0 holds the variant's number; then come the fields of every variant, side by side.
fn variant_slice(d: &EnumDef, i: usize) -> (usize, usize) {
    let size = |fs: &[(String, Type)]| fs.iter().map(|(_, t)| scalars(t).len()).sum::<usize>();
    let start = 1 + d.variants[..i].iter().map(|(_, fs)| size(fs)).sum::<usize>();
    (start, size(&d.variants[i].1))
}

/// The fields of an enum's variants, all of them, in the order of their slots.
fn variant_fields(d: &EnumDef) -> impl Iterator<Item = &Type> {
    d.variants.iter().flat_map(|(_, fs)| fs.iter().map(|(_, t)| t))
}

/// The slots needed for a value of this type, in order.
pub fn scalars(t: &Type) -> Vec<Scalar> {
    match t {
        Type::Int | Type::Bool | Type::Char | Type::String | Type::BigInt | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_) => vec![Scalar::Int],
        // The function's number, then the box of what the block uses around it.
        Type::Block(..) => vec![Scalar::Int, Scalar::Int],
        Type::Float => vec![Scalar::Float],
        Type::Tuple(ts) => ts.iter().flat_map(scalars).collect(),
        Type::Struct(_) => members(t).iter().flat_map(scalars).collect(),
        // ponytail: no slot is shared between variants (a Float and a String never meet in one),
        // so a value is wider than its largest variant; slots of the other variants hold 0.
        Type::Enum(d) => std::iter::once(Scalar::Int).chain(variant_fields(d).flat_map(scalars)).collect(),
        // Slot 0: 1 if there is a value (0 = `none`); then the value's slots, 0 for `none`.
        Type::Optional(t) => std::iter::once(Scalar::Int).chain(scalars(t)).collect(),
        // Slot 0: 1 for `ok`; then the error's slots, then the value's.
        Type::Result(ok, err) => std::iter::once(Scalar::Int).chain(scalars(err)).chain(scalars(ok)).collect(),
        Type::Unit => vec![],
        Type::Error => unreachable!("erroneous type after type checking"),
        Type::Param(_) => unreachable!("`Self` replaced by the real type (`Translator::ty`)"),
    }
}

/// For each slot of a value of this type: what it owns (`String` or `List<T>`), to free or copy.
fn heap(t: &Type) -> Vec<Option<Type>> {
    match t {
        Type::String | Type::BigInt | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_) => vec![Some(t.clone())],
        Type::Tuple(_) | Type::Struct(_) => members(t).iter().flat_map(heap).collect(),
        // The other variants' slots hold 0: freeing or copying them does nothing.
        Type::Enum(d) => std::iter::once(None).chain(variant_fields(d).flat_map(heap)).collect(),
        Type::Optional(t) => std::iter::once(None).chain(heap(t)).collect(),
        Type::Result(ok, err) => std::iter::once(None).chain(heap(err)).chain(heap(ok)).collect(),
        Type::Int | Type::Float | Type::Bool | Type::Char | Type::Unit | Type::Error | Type::Param(_) | Type::Block(..) => {
            vec![None; scalars(t).len()]
        }
    }
}

/// Does a value of this type own nothing (no `String`, no `List`)?
fn plain(t: &Type) -> bool {
    heap(t).iter().all(Option::is_none)
}

/// Where a value lives.
#[derive(Clone)]
enum Place {
    /// In slots of the function: a variable, a field of one.
    Slots(Slots),
    /// In element `index` of a List: its slots `start..`, of these machine types.
    Element { list: Local, index: Local, start: usize, kinds: Vec<Scalar> },
}

impl Place {
    /// Member `(start, n)` of the value there (see `slice_of`).
    fn narrow(self, (start, n): (usize, usize)) -> Place {
        match self {
            Place::Slots(s) => Place::Slots(s[start..start + n].to_vec()),
            Place::Element { list, index, start: first, kinds } => {
                Place::Element { list, index, start: first + start, kinds: kinds[start..start + n].to_vec() }
            }
        }
    }
}

/// Functions made by the translation (not written in the program), numbered after the others.
/// Each one receives a value of the type given (a `List`; for `Show`, an enum too).
#[derive(Clone, PartialEq)]
enum Helper {
    /// Frees the List and what its elements own.
    Drop(Type),
    /// Returns a deep copy.
    Clone(Type),
    /// Returns its text, as `print` writes it: `[1, 2]`, `Shape.Circle(radius: 2.0)`.
    Show(Type),
    /// Takes two values (an enum, an optional or a result) and returns 1 if they are equal.
    Equal(Type),
    /// A generic function translated for these types (one per type parameter): `larger<Int>`.
    Instance(String, Vec<Type>),
    /// `a.sound` on an `any Animal` (interface, method): takes the box, the other arguments and
    /// the call location; calls the method of the real type, found by its number in the box.
    Dispatch(String, String),
    /// For the debugger: returns the text of a value as a contract's `values:` line shows it.
    Look(Type),
    /// The function of a block (`{ x: Int => … }`), number `k` in the blocks made (`Translator::made`).
    Block(usize),
    /// Takes a Map or a Set and a key (of this type): returns its position, or -1 (see `find`).
    Find(Type),
}

/// For the debugger (`kova debug`): a variable visible at a statement, its slots, and the
/// function (a `Helper::Look`) that gives its text.
pub struct Watch {
    pub name: String,
    pub ty: String,
    pub slots: Slots,
    pub look: usize,
}

pub struct Program {
    pub functions: Vec<Function>,
    /// `None` for a file without `main` (a module): only `kova test` runs it.
    pub main: Option<usize>,
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

/// Position in a source file (`file` = its number, 0 = the program's), for runtime error messages.
#[derive(Clone, Copy)]
pub struct Loc {
    pub file: usize,
    pub line: usize,
    pub col: usize,
}

impl Loc {
    /// The file and the line in one number, as the runtime receives them (`kova_fault`).
    pub fn packed(&self) -> i64 {
        ((self.file as i64) << 32) | self.line as i64
    }
}

/// The text `path:line` of a packed site (`Loc::packed`).
pub fn site_text(files: &[String], packed: i64) -> String {
    format!("{}:{}", files[(packed >> 32) as usize], packed & 0xffff_ffff)
}

/// For an operation that cannot fail (a helper's loop counter).
const NOWHERE: Loc = Loc { file: 0, line: 0, col: 0 };

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
    /// `x.abs`, `x.sqrt`, `x.floor`, `x.ceil`, `x.round` on a Float, giving a Float.
    FUnary(Local, FloatOp, Local),
    Convert(Local, Conversion, Local, Loc),
    /// `results := f(args)`.
    Call(Slots, usize, Slots),
    /// `dst := function f`, to call it later with `CallIndirect` (a block).
    FuncRef(Local, usize),
    /// `results := (the function in this local)(args)`: calls a block.
    CallIndirect(Slots, Local, Slots),
    /// `dst := "text"`: a new String.
    Text(Local, String),
    /// `dst := op(args)`, done by the runtime (`kova_str_*` in `runtime/runtime.c`).
    Str(Local, StrOp, Slots),
    /// Frees a String, then sets its slot to 0 (freeing 0 does nothing).
    Free(Local),
    /// `dst := a new empty List` whose elements take `width` slots (`kova_list_*` in `runtime/runtime.c`).
    ListNew(Local, usize),
    /// `dst := list.length`.
    ListLength(Local, Local),
    /// Stops with "index out of bounds" unless `0 <= index < list.length`.
    ListCheck(Local, Local, Loc),
    /// Stops the program with this fault if the local is not 0.
    FaultIf(Local, Fault, Loc),
    /// `dst := slot k of element index` (index checked before).
    ListLoad(Local, Local, Local, usize),
    /// `slot k of element index := value`.
    ListStore(Local, Local, usize, Local),
    /// Adds an element at the end (its slots are 0): `dst := its index`.
    ListGrow(Local, Local),
    /// Removes element `index` (read before); the next ones move back by one.
    ListRemove(Local, Local),
    /// Frees the List's block, not what its elements own (done before); then sets its slot to 0.
    ListFree(Local),
    /// `dst := the index of a Map or a Set` (a List of positions, see `find_in_index`; 0 = none).
    /// Freed with its List; `ListRemove` takes the removed element out of it.
    ListIndex(Local, Local),
    /// `list.index := value` (the List had none).
    ListSetIndex(Local, Local),
    /// `dst := a number made from a and b`, never negative, never an overflow: for `hash`.
    Mix(Local, Local, Local),
    /// `print` (`newline` = true) or `write`.
    Print { value: Local, show: Show, newline: bool },
    /// Broken contract: writes the message to stderr and stops (code 101).
    Fail(Vec<Piece>),
    /// `exit(code)`: stops the program with this code (0 to 255, checked before).
    Exit(Local),
    /// Only for the debugger (`kova debug`): a statement starts here, these variables are visible.
    Line(Loc, Vec<Watch>),
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
    /// `s.to_int`: 1 if `s` is an Int (`-12`), else 0 (see `parse_int`).
    IsInt,
    /// `s.to_int`: its value, 0 if it is not an Int.
    ToInt,
    /// `a < b` on Strings: 1 or 0, in the order of their bytes (= of their characters' codes).
    Less,
    /// `read_file(path)`: a new String, or minus the number of the failure (`types::io_error`).
    ReadFile,
    /// `write_file(path, text)`, `append_file(path, text)`, `make_dir(path)`, `remove_file(path)`:
    /// 0, or minus the number of the failure; `list_dir(path)`: a new `List<String>` or minus it.
    WriteFile,
    AppendFile,
    MakeDir,
    RemoveFile,
    ListDir,
    /// `exists(path)`: 1 or 0.
    Exists,
    /// `s.to_upper`, `s.to_lower`: a new String (`case_of`).
    ToUpper,
    ToLower,
    /// `s.find(part)`: the number of the character where `part` starts, -1 if nowhere.
    Find,
    /// `s[from..to]`: a new String (the bounds checked before, in characters).
    Slice,
    /// `s.repeat(n)`: a new String.
    Repeat,
    /// `words.join(sep)` on a `List<String>`: a new String.
    Join,
    /// `s.to_float`: 1 if `s` is a Float (`parse_float`), else 0; then its bits.
    IsFloat,
    ToFloat,
    /// `now`: the bits of a Float, the seconds since 1970.
    Now,
    /// `sleep(seconds)`, given the bits of the Float: gives 0.
    Sleep,
    /// `random(from..to)`, `to` excluded (`from < to`, checked before).
    Random,
    /// `read_line`: a new String, 0 at the end of the input, -1 if the line is not UTF-8.
    ReadLine,
    /// `args`: a new `List<String>`, the words given after the program's name.
    Args,
    /// `s.trim`: a new String without the spaces, tabs and line ends at both ends.
    Trim,
    /// `s.contains(part)`, `s.starts_with(part)`, `s.ends_with(part)`: 1 or 0.
    Contains,
    StartsWith,
    EndsWith,
    /// `s.replace(old, new)`: a new String (`old` is never empty, see `Fault::EmptyText`).
    Replace,
    /// `a + b`, `a - b`, `a * b`, `a / b`, `a % b` on `BigInt`s (decimal texts): a new one.
    BigAdd,
    BigSub,
    BigMul,
    BigDiv,
    BigRem,
    /// `a < b` on `BigInt`s: 1 or 0.
    BigLess,
    /// `s.split(separator)`: a new `List<String>` (the separator is never empty).
    Split,
    /// `s.words`: a new `List<String>`, the pieces between spaces, tabs and line ends.
    Words,
    /// `s.chars`: a new `List<Char>`.
    Chars,
    /// A number for the text (0 for no String): where a key is looked for (`hash`).
    Hash,
}

/// A String method with arguments or a new List (`types::string_method`): its operation.
pub fn string_op(name: &str) -> Option<StrOp> {
    Some(match name {
        "trim" => StrOp::Trim,
        "contains" => StrOp::Contains,
        "starts_with" => StrOp::StartsWith,
        "ends_with" => StrOp::EndsWith,
        "replace" => StrOp::Replace,
        "split" => StrOp::Split,
        "words" => StrOp::Words,
        "chars" => StrOp::Chars,
        "to_upper" => StrOp::ToUpper,
        "to_lower" => StrOp::ToLower,
        "repeat" => StrOp::Repeat,
        _ => return None,
    })
}

/// The capital (`upper`) or small letter of `c`, for the Latin, Greek and Cyrillic alphabets;
/// any other character stays. The same table as `case_of` in `runtime/runtime.c` (Rust's own
/// `to_uppercase` knows all of Unicode, the C runtime does not: run and build must agree).
pub fn case_of(c: char, upper: bool) -> char {
    let c = c as u32;
    let pairs = |c: u32, even_upper: bool| {
        let ranges: &[(u32, u32)] = if even_upper { &[(0x100, 0x12F), (0x132, 0x137), (0x14A, 0x177)] } else { &[(0x139, 0x148), (0x179, 0x17E)] };
        ranges.iter().any(|&(a, b)| (a..=b).contains(&c))
    };
    let code = if upper {
        match c {
            0x61..=0x7A | 0x3B1..=0x3C9 | 0x430..=0x44F if c != 0x3C2 => c - 32,
            0xE0..=0xFE if c != 0xF7 => c - 32,
            0xFF => 0x178,
            0x3C2 => 0x3A3,
            0x450..=0x45F => c - 80,
            _ if pairs(c, true) && c % 2 == 1 => c - 1,
            _ if pairs(c, false) && c % 2 == 0 => c - 1,
            _ => c,
        }
    } else {
        match c {
            0x41..=0x5A | 0x391..=0x3A9 | 0x410..=0x42F if c != 0x3A2 => c + 32,
            0xC0..=0xDE if c != 0xD7 => c + 32,
            0x178 => 0xFF,
            0x400..=0x40F => c + 80,
            _ if pairs(c, true) && c % 2 == 0 => c + 1,
            _ if pairs(c, false) && c % 2 == 1 => c + 1,
            _ => c,
        }
    };
    char::from_u32(code).expect("a letter of the table")
}

/// `s.to_float`: an optional `-`, digits, then maybe `.` and digits, then maybe `e`, a sign and
/// digits (same rule as `parse_float` in `runtime/runtime.c`); `None` if infinite.
pub fn parse_float(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let d = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > d
    };
    if b.first() == Some(&b'-') {
        i += 1;
    }
    if !digits(&mut i) {
        return None;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return None;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return None;
        }
    }
    let v: f64 = if i == b.len() { s.parse().ok()? } else { return None };
    v.is_finite().then_some(v)
}

/// `s.to_int`: an optional `-` then digits only, within the range of an `Int`
/// (same rule as `parse_int` in `runtime/runtime.c`).
pub fn parse_int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A piece of the message of a broken contract.
pub enum Piece {
    Text(String),
    /// A text of `locale::RUNTIME` (`contract_values`): translated when the message is written,
    /// so that an executable speaks the language of whoever runs it.
    Label(&'static str),
    Value(Local, Show),
    /// The name of a source file, by its number (the paths are given to `interp` and `codegen`).
    File(usize),
}

/// How to show a slot. Same order as in `runtime/runtime.c`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Show {
    Int = 0,
    Bool = 1,
    Float = 2,
    Char = 3,
    String = 4,
    /// A call site: `path:line` of a packed site (`Loc::packed`); only in error messages.
    Site = 5,
}

impl Show {
    /// A value of one slot; a tuple or a struct is shown by `value_pieces`.
    pub fn of(t: &Type) -> Show {
        match t {
            Type::Int => Show::Int,
            Type::Bool => Show::Bool,
            Type::Float => Show::Float,
            Type::Char => Show::Char,
            // A `BigInt` is its decimal text.
            Type::String | Type::BigInt => Show::String,
            other => unreachable!("`{}` is not shown in one slot", other.text()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FloatOp {
    Abs,
    Sqrt,
    Floor,
    Ceil,
    /// Half away from zero: `2.5` → `3.0`, `-2.5` → `-3.0` (like Rust's `round`, unlike Python's).
    Round,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Conversion {
    /// `n.to_float()`
    IntToFloat,
    /// The bits of a Float, given by the runtime in an integer slot (`StrOp::ToFloat`).
    FromBits,
    /// The other way: a Float given to the runtime (`StrOp::Sleep`).
    ToBits,
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
    Index,
    NotText,
    /// `split("")` or `replace("", …)`: an empty text cannot be looked for.
    EmptyText,
    /// `exit(300)`: a program's exit code is between 0 and 255.
    ExitCode,
    /// `n.pow(-1)` on an `Int`.
    NegativePower,
    /// `random(5..5)`: nothing to draw from.
    EmptyRange,
}

impl Fault {
    /// Its message, a key of `locale::RUNTIME`.
    pub fn key(self) -> &'static str {
        match self {
            Fault::Overflow => "fault_overflow",
            Fault::DivisionByZero => "fault_division_by_zero",
            Fault::BadConversion => "fault_bad_conversion",
            Fault::Index => "fault_index",
            Fault::NotText => "fault_not_text",
            Fault::EmptyText => "fault_empty_text",
            Fault::ExitCode => "fault_exit_code",
            Fault::NegativePower => "fault_negative_power",
            Fault::EmptyRange => "fault_empty_range",
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

pub fn fault_text(f: Fault, files: &[String], l: Loc) -> String {
    let text = |k| locale::text(k, &[]);
    format!("{}{}\n  --> {}:{}:{}\n", text("error_prefix"), text(f.key()), files[l.file], l.line, l.col)
}

// ---- Translation AST → KIR ----

/// `contracts` = false for `kova build --boost`: `require`, `ensure` and `check` disappear.
/// `tests` = true for `kova test`: each `test` block becomes one more function.
/// `built` = true for `kova build`: the value of `mode = build`.
/// `watching`: for the debugger, an `Inst::Line` before each statement.
pub fn translate<'f>(f: &'f ast::File, info: &Info, src: &str, contracts: bool, tests: bool, built: bool, watching: bool) -> Program {
    // Every function: the free ones, then the structs' methods (`Dog.sound`), then the tests
    // (a test = a function without parameters, result or contracts).
    // The last element: what the `Type::Param`s stand for (`Self` in a struct's methods and in the
    // default methods it takes; `T` in a copy of a generic function).
    type Source<'f> =
        (String, Option<(Mode, Type)>, &'f [ast::Param], Option<&'f ast::TypeAst>, &'f [Contract], &'f Block, Vec<(String, Type)>);
    let free = |g: &'f ast::Function| -> Source<'f> {
        (g.name.text.clone(), None, &g.params[..], g.ret.as_ref(), &g.contracts[..], &g.body, vec![])
    };
    let mut all: Vec<Source> = f.functions.iter().filter(|g| g.generics.is_empty()).map(free).collect();
    // A generic function is translated once for each list of types it is called with
    // (`Helper::Instance`): `larger<Int>`, `larger<Dog>`.
    let generic: HashMap<String, (Source, Vec<String>)> = (f.functions.iter())
        .filter(|g| !g.generics.is_empty())
        .map(|g| (g.name.text.clone(), (free(g), g.generics.iter().map(|p| p.name.text.clone()).collect())))
        .collect();
    let method = |ty: &Type, owner: &str, m: &'f ast::Function| -> Source<'f> {
        let name = method_name(owner, &m.name.text);
        let me = vec![("Self".to_string(), ty.clone())];
        (name, m.receiver.map(|r| (r, ty.clone())), &m.params, m.ret.as_ref(), &m.contracts, &m.body, me)
    };
    let mut generic = generic;
    for s in &f.structs {
        let ty = info.structs[&s.name.text].clone();
        if s.generics.is_empty() {
            all.extend(s.methods.iter().map(|m| method(&ty, &s.name.text, m)));
            continue;
        }
        // `struct Stack<T>`: one copy of each method per list of types (`Stack.push<Int>`).
        let names: Vec<String> = s.generics.iter().map(|g| g.name.text.clone()).collect();
        generic.extend(s.methods.iter().map(|m| (method_name(&s.name.text, &m.name.text), (method(&ty, &s.name.text, m), names.clone()))));
    }
    // One copy of a default method for each type taking it: `Dog.introduce`, `Cat.introduce`.
    for (owner, k, j) in &info.defaults {
        all.push(method(&info.structs[owner], owner, &f.interfaces[*k].methods[*j].0));
    }
    let signatures: HashMap<String, (Option<Mode>, Vec<Mode>)> = (all.iter().chain(generic.values().map(|(g, _)| g)))
        .map(|g| (g.0.clone(), (g.1.as_ref().map(|r| r.0), g.2.iter().map(|p| p.mode).collect())))
        .collect();
    // The interfaces' methods, called on an `any` through their dispatch (`Helper::Dispatch`).
    let mut signatures = signatures;
    for it in &f.interfaces {
        for (m, _) in &it.methods {
            signatures.insert(method_name(&it.name.text, &m.name.text), (m.receiver, m.params.iter().map(|p| p.mode).collect()));
        }
    }
    let n = all.len();
    all.extend(f.tests.iter().filter(|_| tests).map(|t| (t.name.clone(), None, &[][..], None, &[][..], &t.body, vec![])));
    let numbers: HashMap<&str, usize> = all[..n].iter().enumerate().map(|(i, g)| (g.0.as_str(), i)).collect();
    // The functions that receive the call location (`main` is called by the runtime).
    let site = |g: &Source| contracts && !g.4.is_empty() && g.0 != "main";
    let sites: Vec<bool> = all.iter().map(site).collect();
    let generic_sites: HashMap<&str, bool> = generic.iter().map(|(n, (g, _))| (n.as_str(), site(g))).collect();
    let helpers = RefCell::new(vec![]);
    let made = RefCell::new(vec![]);
    let translator = |ret: Type, ensures: Vec<&'f Contract>, subst: Vec<(String, Type)>| Translator {
        src,
        info,
        subst,
        debug: false,
        numbers: &numbers,
        signatures: &signatures,
        sites: &sites,
        generic_sites: &generic_sites,
        helpers: &helpers,
        made: &made,
        owner: String::new(),
        first_helper: all.len(),
        contracts,
        built,
        watching,
        ensures,
        ret,
        result: None,
        site: None,
        outs: vec![],
        // 0: the borrowed parameters; 1: the owned ones (`move`), freed at each exit.
        names: vec![Scope::borrowed(), Scope::default()],
        locals: vec![],
        blocks: vec![],
        current: 0,
        loops: vec![],
        temps: vec![],
        envs: vec![],
        env_marks: vec![],
    };
    let make = |(name, receiver, params, ret, written_contracts, body, subst): &Source<'f>, with_site: bool| {
            let mut structs = info.structs.clone();
            let subst = subst.clone();
            structs.extend(subst.iter().cloned());
            let ret = ret.map_or(Type::Unit, |t| type_of(t, &structs));
            let kept = |k: ContractKind| written_contracts.iter().filter(move |c: &&Contract| contracts && c.kind == k);
            let mut t = translator(ret.clone(), kept(ContractKind::Ensure).collect(), subst);
            t.owner = name.clone();
            let mut returns = scalars(&ret);
            if let Some((r, ty)) = receiver {
                let v = t.new_slots(ty);
                t.parameter("self", *r, v, ty.clone(), &mut returns);
            }
            for p in params.iter() {
                let ty = type_of(&p.ty, &structs);
                let v = t.new_slots(&ty);
                t.parameter(&p.name.text, p.mode, v, ty, &mut returns);
            }
            t.site = with_site.then(|| (t.int_local(), t.int_local()));
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
            t.function(name.clone(), nb_params, returns)
    };
    let mut functions: Vec<Function> = all.iter().enumerate().map(|(i, g)| make(g, sites[i])).collect();
    // The helpers, asked for while translating (and by one another: `List<List<String>>`).
    let mut k = 0;
    while k < helpers.borrow().len() {
        let h = helpers.borrow()[k].clone();
        functions.push(match &h {
            Helper::Instance(name, types) => {
                let (g, names) = &generic[name];
                let mut subst: Vec<(String, Type)> = names.iter().cloned().zip(types.iter().cloned()).collect();
                // A method's `Self` and `self` are `Stack<Int>`.
                subst.extend(g.6.iter().map(|(n, t)| (n.clone(), substitute(t, &subst))).collect::<Vec<_>>());
                let receiver = g.1.as_ref().map(|(r, t)| (*r, substitute(t, &subst)));
                let shown = format!("{name}<{}>", types.iter().map(Type::text).collect::<Vec<_>>().join(", "));
                make(&(shown, receiver, g.2, g.3, g.4, g.5, subst), generic_sites[name.as_str()])
            }
            Helper::Block(k) => made.borrow_mut()[*k].take().expect("each block is made once"),
            h => translator(Type::Unit, vec![], vec![]).helper_function(h),
        });
        k += 1;
    }
    let tests = all[n..].iter().enumerate().map(|(i, t)| (t.0.clone(), n + i)).collect();
    Program { functions, main: numbers.get("main").copied(), tests }
}

struct Translator<'a> {
    src: &'a str,
    info: &'a Info,
    /// What each `Type::Param` stands for in this copy of the function (`Self` → `Dog`).
    subst: Vec<(String, Type)>,
    /// Showing values for a contract's `values:` line: `to_string` is not used.
    debug: bool,
    numbers: &'a HashMap<&'a str, usize>,
    /// How each function receives its value (`None`: not a method) and its parameters,
    /// by name (`f`, `Dog.sound`).
    signatures: &'a HashMap<String, (Option<Mode>, Vec<Mode>)>,
    /// By function number: does it receive the call location?
    sites: &'a [bool],
    /// The same for the generic functions, by name.
    generic_sites: &'a HashMap<&'a str, bool>,
    /// The helpers asked for so far: helper `k` is function `first_helper + k`.
    helpers: &'a RefCell<Vec<Helper>>,
    /// The functions of the blocks met so far (`Helper::Block`), taken when the helpers are made.
    made: &'a RefCell<Vec<Option<Function>>>,
    /// The function being translated (`main`), to name its blocks: `block in main`.
    owner: String,
    first_helper: usize,
    /// Keep contracts (false with `--boost`).
    contracts: bool,
    /// `kova build` (true) or `kova run` / `kova test` (false).
    built: bool,
    /// For the debugger: an `Inst::Line` before each statement.
    watching: bool,
    ensures: Vec<&'a Contract>,
    ret: Type,
    /// The value of `result` while checking an `ensure`.
    result: Option<(Slots, Type)>,
    /// The 2 hidden parameters: line and column of the call (see `sites`).
    site: Option<(Local, Local)>,
    /// The slots of `var self` and of the `var` parameters, given back after the result at each exit.
    outs: Slots,
    /// Name → its slots and type, one table per open block.
    names: Vec<Scope>,
    locals: Vec<Scalar>,
    /// Blocks being built; `None` = end not set yet.
    blocks: Vec<(Vec<Inst>, Option<End>)>,
    current: BlockId,
    /// For each open loop: (target of `continue`, target of `break`, number of open blocks).
    loops: Vec<(BlockId, BlockId, usize)>,
    /// The Strings and Lists made by the current statement and not given to anyone yet
    /// (with what they are, see `heap`): freed at the end of the statement.
    temps: Vec<(Local, Type)>,
    /// The boxes of the blocks given to the call being prepared, with the slots of what each one
    /// holds: given back to those variables after the call (copy-in / copy-out, like `var`).
    envs: Vec<(Local, Vec<Slots>)>,
    /// Where the boxes of each call whose arguments are ready start in `envs` (see `call_to`).
    env_marks: Vec<usize>,
}

/// The names of one open block.
#[derive(Default)]
struct Scope {
    /// Names that borrow their slots (borrowed parameters, `for` elements…): never freed.
    borrowed: bool,
    vars: HashMap<String, (Slots, Type)>,
}

impl Scope {
    fn borrowed() -> Scope {
        Scope { borrowed: true, vars: HashMap::new() }
    }
}

impl<'a> Translator<'a> {
    /// The type of `e`'s value.
    fn ty(&self, e: &Expr) -> Type {
        substitute(&self.info.types[e.id], &self.subst)
    }

    /// The type of the value `e` gives where it goes (`types::Info::given`).
    fn given(&self, e: &Expr) -> Type {
        substitute(&self.info.given(e), &self.subst)
    }

    fn function(self, name: String, nb_params: usize, returns: Vec<Scalar>) -> Function {
        Function {
            name,
            nb_params,
            locals: self.locals,
            returns,
            blocks: self.blocks.into_iter().map(|(insts, end)| KBlock { insts, end: end.unwrap_or(End::Unreachable) }).collect(),
        }
    }

    /// The number of the function doing `h` (made later by `helper_function`).
    fn helper(&self, h: Helper) -> usize {
        let mut hs = self.helpers.borrow_mut();
        let k = hs.iter().position(|x| *x == h).unwrap_or_else(|| {
            hs.push(h);
            hs.len() - 1
        });
        self.first_helper + k
    }

    fn helper_function(mut self, h: &Helper) -> Function {
        if let Helper::Equal(ty) = h {
            let (a, b) = (self.new_slots(ty), self.new_slots(ty));
            self.current = self.new_block();
            let same = self.equal_variants(&a, &b, ty);
            self.finish(End::Return(vec![same]));
            return self.function(format!("equal {}", ty.text()), a.len() * 2, vec![Scalar::Int]);
        }
        if let Helper::Dispatch(interface, method) = h {
            return self.dispatch(interface, method);
        }
        if let Helper::Find(k) = h {
            let (map, key) = (self.int_local(), self.new_slots(k));
            self.current = self.new_block();
            let found = self.find_in_index(map, &key, k);
            self.finish(End::Return(vec![found]));
            return self.function(format!("find {}", k.text()), 1 + key.len(), vec![Scalar::Int]);
        }
        if let Helper::Look(ty) = h {
            let v = self.new_slots(ty);
            self.current = self.new_block();
            let mut pieces = vec![];
            self.debug = true;
            self.value_pieces(&v, ty, &mut pieces);
            let text = self.join(pieces);
            self.free_temps(0);
            self.finish(End::Return(vec![text]));
            return self.function(format!("look {}", ty.text()), v.len(), vec![Scalar::Int]);
        }
        let (Helper::Drop(ty) | Helper::Clone(ty) | Helper::Show(ty)) = h else { unreachable!("made by `translate`") };
        let v = self.new_slots(ty);
        self.current = self.new_block();
        if let Type::Any(interface) = ty {
            return self.any_helper(h, interface, v[0]);
        }
        if let (Helper::Show(_), Some(d)) = (h, enum_view(ty)) {
            let text = self.show_enum(&v, &d, ty);
            self.finish(End::Return(vec![text]));
            return self.function(format!("show {}", ty.text()), v.len(), vec![Scalar::Int]);
        }
        let list = v[0];
        let (elem, map) = match ty {
            Type::List(elem) | Type::Set(elem) => ((**elem).clone(), false),
            Type::Map(k, v) => (pair(k, v), true),
            _ => unreachable!("helpers work on Lists, Maps and enums"),
        };
        let elem = &elem;
        let (name, result) = match h {
            Helper::Drop(_) => {
                self.each_element(list, elem, |t, _, v| t.free_value(&v, elem));
                self.emit(Inst::ListFree(list));
                ("drop", vec![])
            }
            Helper::Clone(_) => {
                // 0 (no List: a field of another variant of an enum) is copied as 0.
                let copy = self.constant(0);
                let (make, done) = (self.new_block(), self.new_block());
                self.go(End::Branch(list, make, done), make);
                self.emit(Inst::ListNew(copy, scalars(elem).len()));
                self.each_element(list, elem, |t, _, v| {
                    let c = t.clone_value(&v, elem);
                    t.push(copy, &c);
                });
                self.go(End::Jump(done), done);
                ("clone", vec![copy])
            }
            Helper::Show(_) => {
                let text = self.text("[");
                self.each_element(list, elem, |t, i, v| {
                    // `, ` before each element but the first.
                    let (zero, comma, next) = (t.constant(0), t.new_block(), t.new_block());
                    let first = t.int_local();
                    t.emit(Inst::Binary(first, Op::Eq, i, zero, NOWHERE));
                    t.go(End::Branch(first, next, comma), comma);
                    let sep = t.text(", ");
                    t.append(text, sep);
                    t.go(End::Jump(next), next);
                    let mark = t.temps.len();
                    let mut pieces = vec![];
                    if map {
                        // `"Rex" := 3`, as in the literal.
                        let (key, value) = (element(elem, 0), element(elem, 1));
                        let n = scalars(&key).len();
                        t.value_pieces(&v[..n], &key, &mut pieces);
                        pieces.push(Piece::Text(" := ".into()));
                        t.value_pieces(&v[n..], &value, &mut pieces);
                    } else {
                        t.value_pieces(&v, elem, &mut pieces);
                    }
                    let piece = t.join(pieces);
                    t.append(text, piece);
                    t.free_temps(mark);
                });
                let close = self.text("]");
                self.append(text, close);
                ("show", vec![text])
            }
            Helper::Equal(_) | Helper::Instance(..) | Helper::Dispatch(..) | Helper::Look(_) | Helper::Block(_) | Helper::Find(_) => unreachable!("made above"),
        };
        self.finish(End::Return(result.clone()));
        let returns = result.iter().map(|_| Scalar::Int).collect();
        self.function(format!("{name} {}", ty.text()), 1, returns)
    }

    /// Runs `body` with the real type and the value inside the box of an `any` (borrowed from it);
    /// nothing for 0 (a box given away).
    fn on_boxed(&mut self, b: Local, interface: &str, mut body: impl FnMut(&mut Self, &Type, Slots)) {
        let after = self.new_block();
        let full = self.new_block();
        self.go(End::Branch(b, full, after), full);
        let (zero, tag) = (self.constant(0), self.int_local());
        self.emit(Inst::ListLoad(tag, b, zero, 0));
        let types = self.info.impls.get(interface).cloned().unwrap_or_default();
        for (k, ty) in types.iter().enumerate() {
            let (yes, no) = (self.new_block(), self.new_block());
            let (number, test) = (self.constant(k as i64), self.int_local());
            self.emit(Inst::Binary(test, Op::Eq, tag, number, NOWHERE));
            self.go(End::Branch(test, yes, no), yes);
            let v = self.new_slots(ty);
            for (j, &d) in v.iter().enumerate() {
                self.emit(Inst::ListLoad(d, b, zero, j + 1));
            }
            body(self, ty, v);
            self.go(End::Jump(after), no);
        }
        self.go(End::Unreachable, after);
    }

    /// A new box for an `any interface`: the number of the real type `ty`, then the slots `v`.
    fn new_box(&mut self, interface: &str, ty: &Type, v: &[Local]) -> Local {
        let k = self.info.impls[interface].iter().position(|t| t == ty).expect("a type following the interface");
        let b = self.int_local();
        self.emit(Inst::ListNew(b, 1 + v.len()));
        let tag = self.constant(k as i64);
        self.push(b, &[[tag].as_slice(), v].concat());
        b
    }

    /// Frees, copies or shows the value of an `any` through its real type.
    fn any_helper(mut self, h: &Helper, interface: &str, b: Local) -> Function {
        let ty = Type::Any(interface.to_string());
        let (name, result) = match h {
            Helper::Drop(_) => {
                self.on_boxed(b, interface, |t, ty, v| t.free_value(&v, ty));
                self.emit(Inst::ListFree(b));
                ("drop", vec![])
            }
            Helper::Clone(_) => {
                let copy = self.constant(0);
                self.on_boxed(b, interface, |t, ty, v| {
                    let c = t.clone_value(&v, ty);
                    let nb = t.new_box(interface, ty, &c);
                    t.emit(Inst::Copy(copy, nb));
                });
                ("clone", vec![copy])
            }
            Helper::Show(_) => {
                let text = self.int_local();
                self.on_boxed(b, interface, |t, ty, v| {
                    let mark = t.temps.len();
                    let mut pieces = vec![];
                    t.value_pieces(&v, ty, &mut pieces);
                    let piece = t.join(pieces);
                    t.emit(Inst::Copy(text, piece));
                    t.free_temps(mark);
                });
                ("show", vec![text])
            }
            _ => unreachable!("an `any` is freed, copied or shown"),
        };
        self.finish(End::Return(result.clone()));
        let returns = result.iter().map(|_| Scalar::Int).collect();
        self.function(format!("{name} {}", ty.text()), 1, returns)
    }

    /// `Animal.sound` for an `any Animal`: calls `Dog.sound` or `Cat.sound`, by the box's number.
    /// A `var self` method changes the value inside the box, which stays where it is.
    fn dispatch(mut self, interface: &str, method: &str) -> Function {
        let q = method_name(interface, method);
        let (params, ret) = self.info.methods[&q].clone();
        let receiver = self.signatures[&q].0.expect("a method");
        let b = self.int_local();
        let args: Vec<Slots> = params.iter().map(|t| self.new_slots(t)).collect();
        let (line, col) = (self.int_local(), self.int_local());
        let nb_params = self.locals.len();
        self.current = self.new_block();
        let result = self.new_slots(&ret);
        self.on_boxed(b, interface, |t, ty, v| {
            let num = t.numbers[method_name(&ty.owner(), method).as_str()];
            let mut vals: Slots = [v.clone(), args.concat()].concat();
            if t.sites[num] {
                vals.extend([line, col]);
            }
            let out = if receiver == Mode::Var { t.new_slots(ty) } else { vec![] };
            t.emit(Inst::Call([result.clone(), out.clone()].concat(), num, vals));
            let zero = t.constant(0);
            for (j, &s) in out.iter().enumerate() {
                t.emit(Inst::ListStore(b, zero, j + 1, s));
            }
        });
        self.finish(End::Return(result.clone()));
        self.function(format!("dispatch {q}"), nb_params, scalars(&ret))
    }

    /// The text of an enum value `v`: `Shape.Circle(radius: 2.0)`, `Light.Red`; of an
    /// `optional`: `none` or its value as is.
    fn show_enum(&mut self, v: &[Local], d: &EnumDef, ty: &Type) -> Local {
        let text = self.int_local();
        let after = self.new_block();
        for (i, (name, fields)) in d.variants.iter().enumerate() {
            let (yes, no) = (self.new_block(), self.new_block());
            let (number, test) = (self.constant(i as i64), self.int_local());
            self.emit(Inst::Binary(test, Op::Eq, v[0], number, NOWHERE));
            self.go(End::Branch(test, yes, no), yes);
            // Before the pieces: showing a List or an enum field makes a temporary text.
            let mark = self.temps.len();
            let mut pieces = vec![];
            let mut at = variant_slice(d, i).0;
            if let Type::Optional(_) = ty {
                match &fields[..] {
                    [] => pieces.push(Piece::Text("none".into())),
                    [(_, t)] => self.value_pieces(&v[at..], t, &mut pieces),
                    _ => unreachable!("an optional has one value"),
                }
            } else if let (Type::Result(..), [(_, t)]) = (ty, &fields[..]) {
                // `ok(5)`, `err(IoError.NotFound(path: "x"))`.
                pieces.push(Piece::Text(format!("{name}(")));
                self.value_pieces(&v[at..at + scalars(t).len()], t, &mut pieces);
                pieces.push(Piece::Text(")".into()));
            } else if let Type::Result(..) = ty {
                // `ok` of a `Nothing or E`.
                pieces.push(Piece::Text(name.clone()));
            } else {
                pieces.push(Piece::Text(format!("{}.{name}", d.name)));
                for (j, (field, t)) in fields.iter().enumerate() {
                    pieces.push(Piece::Text(format!("{}{field}: ", if j == 0 { "(" } else { ", " })));
                    let n = scalars(t).len();
                    self.value_pieces(&v[at..at + n], t, &mut pieces);
                    at += n;
                }
                if !fields.is_empty() {
                    pieces.push(Piece::Text(")".into()));
                }
            }
            let piece = self.join(pieces);
            self.emit(Inst::Copy(text, piece));
            self.free_temps(mark);
            self.go(End::Jump(after), no);
        }
        self.go(End::Unreachable, after);
        text
    }

    /// `read_file(path)`, `write_file(path, text)`, `list_dir(path)`…: `ok(value)` (or `ok` for a
    /// `Nothing or IoError`), or `err(IoError.X(path: a copy of path))`. `op` gives the value
    /// (or 0) when it works, minus the failure's number otherwise.
    fn io_call(&mut self, op: StrOp, args: &[Expr], ty: &Type) -> Slots {
        let Type::Result(value, error) = ty else { unreachable!("a result checked by the type checker") };
        let Type::Enum(d) = &**error else { unreachable!("IoError is an enum") };
        let d = d.clone();
        let vals: Vec<Local> = args.iter().map(|a| self.expr1(a)).collect();
        let p = vals[0];
        let r = self.int_local();
        self.emit(Inst::Str(r, op, vals));
        // Slots: tag, IoError's tag, one path per variant, then the value (if any).
        let v = self.new_slots(ty);
        v.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
        let zero = self.constant(0);
        let good = self.int_local();
        self.emit(Inst::Binary(good, Op::Ge, r, zero, NOWHERE));
        let (ok, err, after) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Branch(good, ok, err), ok);
        self.emit(Inst::Const(v[0], 1));
        if **value != Type::Unit {
            self.emit(Inst::Copy(*v.last().unwrap(), r));
        }
        self.go(End::Jump(after), err);
        // Variant = failure − 1 = −r − 1.
        let minus_one = self.constant(-1);
        self.emit(Inst::Binary(v[1], Op::Minus, minus_one, r, NOWHERE));
        for i in 0..d.variants.len() {
            let (yes, no) = (self.new_block(), self.new_block());
            let (number, test) = (self.constant(i as i64), self.int_local());
            self.emit(Inst::Binary(test, Op::Eq, v[1], number, NOWHERE));
            self.go(End::Branch(test, yes, no), yes);
            let (start, _) = variant_slice(&d, i);
            self.emit(Inst::Str(v[1 + start], StrOp::Clone, vec![p]));
            self.go(End::Jump(after), no);
        }
        self.go(End::Jump(after), after);
        self.track(&v, &ty);
        v
    }

    /// `a = b` (PLAN Part 4, "J5"): 1 or 0. A struct writing `equals` is asked; otherwise
    /// the fields (or the variant, then its fields) are compared one by one.
    fn equal(&mut self, a: &[Local], b: &[Local], ty: &Type, span: Span) -> Local {
        match ty {
            // A `BigInt` is written one way only: equal texts.
            Type::String | Type::BigInt => {
                let d = self.int_local();
                self.emit(Inst::Str(d, StrOp::Eq, vec![a[0], b[0]]));
                d
            }
            // Only for a type following `Equal`: an `equals` alone changes nothing (nothing implicit).
            Type::Struct(_) | Type::Enum(_)
                if self.info.follows(ty, "Equal") && self.numbers.contains_key(method_name(&ty.text(), "equals").as_str()) =>
            {
                self.call(&method_name(&ty.text(), "equals"), [a, b].concat(), vec![], span, &Type::Bool)[0]
            }
            Type::Struct(_) | Type::Tuple(_) => {
                let same = self.constant(1);
                for (i, t) in members(ty).iter().enumerate() {
                    let (start, n) = slice_of(ty, i);
                    let eq = self.equal(&a[start..start + n], &b[start..start + n], t, span);
                    self.emit(Inst::Binary(same, Op::Times, same, eq, NOWHERE));
                }
                same
            }
            Type::Enum(_) | Type::Optional(_) | Type::Result(..) => {
                let f = self.helper(Helper::Equal(ty.clone()));
                let d = self.int_local();
                self.emit(Inst::Call(vec![d], f, [a, b].concat()));
                d
            }
            _ => self.operation(Op::Eq, a[0], b[0], *ty == Type::Float, NOWHERE),
        }
    }

    /// A number for `v`, the same for two values `equal` finds equal (same cases, same order):
    /// where a key is put in the index of a Map or a Set (`find_in_index`).
    fn hash(&mut self, v: &[Local], ty: &Type) -> Local {
        match ty {
            Type::Int | Type::Bool | Type::Char => v[0],
            Type::String | Type::BigInt => {
                let d = self.int_local();
                self.emit(Inst::Str(d, StrOp::Hash, vec![v[0]]));
                d
            }
            // Its own `equals` may look at some fields only: all its values get the same number.
            Type::Struct(_) | Type::Enum(_)
                if self.info.follows(ty, "Equal") && self.numbers.contains_key(method_name(&ty.text(), "equals").as_str()) =>
            {
                self.constant(0)
            }
            Type::Struct(_) | Type::Tuple(_) => {
                let h = self.constant(0);
                for (i, t) in members(ty).iter().enumerate() {
                    let (start, n) = slice_of(ty, i);
                    let x = self.hash(&v[start..start + n], t);
                    self.emit(Inst::Mix(h, h, x));
                }
                h
            }
            // The variant, then every variant's fields: those of the others hold 0.
            Type::Enum(_) | Type::Optional(_) | Type::Result(..) => {
                let d = enum_view(ty).expect("an enum, an optional or a result");
                let h = self.int_local();
                self.emit(Inst::Copy(h, v[0]));
                let mut at = 1;
                for t in variant_fields(&d) {
                    let n = scalars(t).len();
                    let x = self.hash(&v[at..at + n], t);
                    self.emit(Inst::Mix(h, h, x));
                    at += n;
                }
                h
            }
            // A `Float` (`0.0 = -0.0`, written differently), an `any`.
            _ => self.constant(0),
        }
    }

    /// Inside the `Equal` helper: same variant, then the same fields for that variant.
    fn equal_variants(&mut self, a: &[Local], b: &[Local], ty: &Type) -> Local {
        let d = enum_view(ty).expect("an enum, an optional or a result");
        let same = self.int_local();
        self.emit(Inst::Binary(same, Op::Eq, a[0], b[0], NOWHERE));
        let after = self.new_block();
        let compare = self.new_block();
        self.go(End::Branch(same, compare, after), compare);
        for (i, (_, fields)) in d.variants.iter().enumerate() {
            let (yes, no) = (self.new_block(), self.new_block());
            let (number, test) = (self.constant(i as i64), self.int_local());
            self.emit(Inst::Binary(test, Op::Eq, a[0], number, NOWHERE));
            self.go(End::Branch(test, yes, no), yes);
            let mut at = variant_slice(&d, i).0;
            for (_, t) in fields {
                let n = scalars(t).len();
                let eq = self.equal(&a[at..at + n], &b[at..at + n], t, Span::new(0, 0));
                self.emit(Inst::Binary(same, Op::Times, same, eq, NOWHERE));
                at += n;
            }
            self.go(End::Jump(after), no);
        }
        self.go(End::Jump(after), after);
        same
    }

    /// `a < b` on a String or a type following `Comparable` (its `less`).
    fn less(&mut self, a: &[Local], b: &[Local], ty: &Type, span: Span) -> Local {
        if matches!(ty, Type::Int | Type::Char | Type::Float) {
            return self.operation(Op::Lt, a[0], b[0], *ty == Type::Float, NOWHERE);
        }
        if matches!(ty, Type::String | Type::BigInt) {
            let (d, op) = (self.int_local(), if *ty == Type::BigInt { StrOp::BigLess } else { StrOp::Less });
            self.emit(Inst::Str(d, op, vec![a[0], b[0]]));
            return d;
        }
        self.call(&method_name(&ty.text(), "less"), [a, b].concat(), vec![], span, &Type::Bool)[0]
    }

    /// Runs `body` on each element of `list` (with its index), in order.
    fn each_element(&mut self, list: Local, elem: &Type, mut body: impl FnMut(&mut Self, Local, Slots)) {
        let n = self.int_local();
        self.emit(Inst::ListLength(n, list));
        let i = self.constant(0);
        let (head, b_body, exit) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(head), head);
        let more = self.int_local();
        self.emit(Inst::Binary(more, Op::Lt, i, n, NOWHERE));
        self.go(End::Branch(more, b_body, exit), b_body);
        let v = self.load(list, i, elem);
        body(self, i, v);
        let one = self.constant(1);
        self.emit(Inst::Binary(i, Op::Plus, i, one, NOWHERE));
        self.go(End::Jump(head), exit);
    }

    /// The slots of element `i` (checked before): borrowed from the List.
    fn load(&mut self, list: Local, i: Local, elem: &Type) -> Slots {
        let v = self.new_slots(elem);
        for (k, &d) in v.iter().enumerate() {
            self.emit(Inst::ListLoad(d, list, i, k));
        }
        v
    }

    /// Adds an element at the end of `list`; it now owns what `v` owns.
    fn push(&mut self, list: Local, v: &[Local]) {
        let i = self.int_local();
        self.emit(Inst::ListGrow(i, list));
        for (k, &s) in v.iter().enumerate() {
            self.emit(Inst::ListStore(list, i, k, s));
        }
    }

    /// Where an error of `a op b` points: the operator, the first non-blank character after `a`.
    fn operator(&self, a: &Expr) -> Loc {
        let after_a = &self.src[a.span.end..];
        let pos = a.span.end + after_a.len() - after_a.trim_start().len();
        self.loc(Span::new(pos, pos))
    }

    /// A `BigInt` of these digits (`-` included), as a temporary.
    fn big_text(&mut self, digits: &str) -> Local {
        let l = self.text(if digits.is_empty() || digits == "-" || digits == "-0" { "0" } else { digits });
        self.temps.push((l, Type::BigInt));
        l
    }

    /// `a op b` on `BigInt`s: a new one (not tracked); `/` and `%` by 0 stop the program.
    fn big(&mut self, op: BinOp, a: Local, b: Local, loc: Loc) -> Local {
        if matches!(op, BinOp::Div | BinOp::Rem) {
            let (zero, is_zero) = (self.big_text("0"), self.int_local());
            self.emit(Inst::Str(is_zero, StrOp::Eq, vec![b, zero]));
            self.emit(Inst::FaultIf(is_zero, Fault::DivisionByZero, loc));
        }
        let op = match op {
            BinOp::Plus => StrOp::BigAdd,
            BinOp::Minus => StrOp::BigSub,
            BinOp::Times => StrOp::BigMul,
            BinOp::Div => StrOp::BigDiv,
            _ => StrOp::BigRem,
        };
        let dst = self.int_local();
        self.emit(Inst::Str(dst, op, vec![a, b]));
        dst
    }

    /// A new String holding `s`.
    fn text(&mut self, s: &str) -> Local {
        let l = self.int_local();
        self.emit(Inst::Text(l, s.into()));
        l
    }

    /// `whole := whole + piece`; both old Strings are freed.
    fn append(&mut self, whole: Local, piece: Local) {
        let joined = self.int_local();
        self.emit(Inst::Str(joined, StrOp::Concat, vec![whole, piece]));
        self.emit(Inst::Free(whole));
        self.emit(Inst::Free(piece));
        self.emit(Inst::Copy(whole, joined));
    }

    /// Frees what slot `l` owns (`kind`, see `heap`), then empties it.
    fn free_slot(&mut self, l: Local, kind: &Type) {
        match kind {
            Type::String | Type::BigInt => self.emit(Inst::Free(l)),
            Type::List(elem) | Type::Set(elem) if plain(elem) => self.emit(Inst::ListFree(l)),
            Type::Map(k, v) if plain(k) && plain(v) => self.emit(Inst::ListFree(l)),
            list => {
                let f = self.helper(Helper::Drop(list.clone()));
                self.emit(Inst::Call(vec![], f, vec![l]));
                self.emit(Inst::Const(l, 0));
            }
        }
    }

    /// A deep copy of a value: what it owns is copied too.
    fn clone_value(&mut self, v: &[Local], ty: &Type) -> Slots {
        let copy = self.new_slots(ty);
        for ((&d, &l), kind) in copy.iter().zip(v).zip(heap(ty)) {
            match kind {
                None => self.emit(Inst::Copy(d, l)),
                Some(Type::String | Type::BigInt) => self.emit(Inst::Str(d, StrOp::Clone, vec![l])),
                Some(list) => {
                    let f = self.helper(Helper::Clone(list));
                    self.emit(Inst::Call(vec![d], f, vec![l]));
                }
            }
        }
        copy
    }

    /// A parameter (or `self`) received in these slots.
    fn parameter(&mut self, name: &str, mode: Mode, v: Slots, ty: Type, returns: &mut Vec<Scalar>) {
        match mode {
            // Taken: freed by the function (scope 1 is freed at each exit).
            Mode::Move => {
                self.names[1].vars.insert(name.to_string(), (v, ty));
            }
            // Changed: given back to the caller at each exit, after the result.
            Mode::Var => {
                self.outs.extend(&v);
                returns.extend(scalars(&ty));
                self.names[0].vars.insert(name.to_string(), (v, ty));
            }
            Mode::Read => {
                self.names[0].vars.insert(name.to_string(), (v, ty));
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
    /// For the debugger: the statement `s` starts here, with the variables visible (in the order
    /// they were made).
    fn line(&mut self, s: &Stmt) {
        let at = match s {
            Stmt::Declare { value, .. } => value.span,
            Stmt::Assign { target, .. } => target.span(),
            Stmt::Return { span, .. } | Stmt::Break(span) | Stmt::Continue(span) => *span,
            Stmt::Check(c) => c.cond.span,
            Stmt::If { cond, .. } | Stmt::While { cond, .. } => cond.span,
            Stmt::Loop { counter, .. } => counter.span,
            Stmt::Each { list, .. } => list.span,
            Stmt::Match(m) => m.subject.span,
            Stmt::Expr(e) => e.span,
        };
        let mut seen: HashMap<&String, (&Slots, &Type)> = HashMap::new();
        for scope in &self.names {
            seen.extend(scope.vars.iter().map(|(n, (v, t))| (n, (v, t))));
        }
        let mut vars: Vec<(String, Slots, Type)> = (seen.into_iter())
            .filter(|(n, (v, _))| n.starts_with(char::is_alphabetic) && !v.is_empty())
            .map(|(n, (v, t))| (n.clone(), v.clone(), substitute(t, &self.subst)))
            .collect();
        vars.sort_by_key(|(_, v, _)| v[0]);
        let watches = vars.into_iter().map(|(name, slots, ty)| Watch { name, ty: ty.text(), look: self.helper(Helper::Look(ty)), slots }).collect();
        let loc = self.loc(at);
        self.emit(Inst::Line(loc, watches));
    }

    fn loc(&self, s: Span) -> Loc {
        let (line, col) = line_col(self.src, s.start);
        Loc { file: crate::diag::file_number(self.src, s.start), line, col }
    }
    fn variable(&self, name: &str) -> (Slots, Type) {
        self.names.iter().rev().find_map(|t| t.vars.get(name)).expect("variable checked by the type checker").clone()
    }
    fn slots_of(&self, name: &str) -> Slots {
        self.variable(name).0
    }

    /// A new value (call result, `a + b`…): its Strings are temporaries until someone takes them.
    fn track(&mut self, v: &[Local], ty: &Type) {
        self.temps.extend(v.iter().zip(heap(ty)).filter_map(|(&l, k)| Some((l, k?))));
    }
    fn untrack(&mut self, v: &[Local]) {
        self.temps.retain(|(l, _)| !v.contains(l));
    }
    /// Frees the temporaries made since `mark`.
    fn free_temps(&mut self, mark: usize) {
        let temps: Vec<(Local, Type)> = self.temps.drain(mark..).collect();
        for (l, kind) in temps.into_iter().rev() {
            self.free_slot(l, &kind);
        }
    }
    fn free_value(&mut self, v: &[Local], ty: &Type) {
        for (&l, kind) in v.iter().zip(heap(ty)).rev() {
            if let Some(kind) = kind {
                self.free_slot(l, &kind);
            }
        }
    }
    /// Frees the variables of the blocks `names[from..]` (from the innermost), borrowed ones excepted.
    fn free_scopes(&mut self, from: usize) {
        let vars: Vec<(Slots, Type)> =
            self.names[from..].iter().rev().filter(|t| !t.borrowed).flat_map(|t| t.vars.values().cloned()).collect();
        for (v, ty) in vars {
            self.free_value(&v, &ty);
        }
    }

    /// `e` is given (`let`, `:=`, `return`…): the value now belongs to the receiver.
    /// A moved variable (marked by `ownership.rs`) is emptied, so that its block frees nothing.
    fn owned(&mut self, e: &Expr) -> Slots {
        if !self.info.moves[e.id] {
            let v = self.value(e);
            self.untrack(&v);
            let v = self.boxed(e, v, true);
            return self.wrapped(e, v);
        }
        let ExprKind::Var(name) = &e.kind else { unreachable!("only a variable is moved") };
        let (src, ty) = self.variable(name);
        let v = self.new_slots(&ty);
        self.copy(&v, &src);
        for (&l, kind) in src.iter().zip(heap(&ty)) {
            if kind.is_some() {
                self.emit(Inst::Const(l, 0));
            }
        }
        let v = self.boxed(e, v, true);
        self.wrapped(e, v)
    }

    /// A value going where an `any Animal` is expected (`types::Info::boxes`) is put in a box.
    /// `owns`: the box takes the value; otherwise it only lends it (a borrowed argument), so only
    /// the box itself is freed at the end of the statement.
    fn boxed(&mut self, e: &Expr, v: Slots, owns: bool) -> Slots {
        let Some(interface) = self.info.boxes[e.id].clone() else { return v };
        let b = self.new_box(&interface, &self.ty(e), &v);
        if !owns {
            // ponytail: freed like a List of Ints, which frees the box and not what it lends.
            self.temps.push((b, Type::List(Box::new(Type::Int))));
        }
        vec![b]
    }

    /// A `T` going where a `T?` is expected (`types::Info::wraps`) gets its slot "there is a value".
    fn wrapped(&mut self, e: &Expr, v: Slots) -> Slots {
        if !self.info.wraps[e.id] {
            return v;
        }
        let present = self.constant(1);
        std::iter::once(present).chain(v).collect()
    }

    /// Opens a scope where each of `names` is the value of its optional, borrowing its slots:
    /// inside `if x != none { … }`, `x` is a `T`.
    fn narrow(&mut self, names: &[&str]) {
        let mut vars = HashMap::new();
        for n in names {
            let (v, ty) = self.variable(n);
            let Type::Optional(inner) = ty else { unreachable!("optional checked by the type checker") };
            vars.insert(n.to_string(), (v[1..].to_vec(), *inner));
        }
        self.names.push(Scope { borrowed: true, vars });
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
                self.names.last_mut().unwrap().vars.insert(n.text.clone(), (slots.to_vec(), ty.clone()));
            }
            // Nobody receives this part: freed at once (unless it is only borrowed).
            Pattern::Ignore(_) if !self.names.last().unwrap().borrowed => self.free_value(slots, ty),
            Pattern::Ignore(_) => {}
            Pattern::Tuple(ps, _) => {
                for (i, p) in ps.iter().enumerate() {
                    let (start, n) = slice_of(ty, i);
                    self.bind(p, &slots[start..start + n], &element(ty, i));
                }
            }
        }
    }

    /// The places designated by a `:=` target, in order, with their types.
    fn target_places(&mut self, t: &Target) -> Vec<(Place, Type)> {
        match t {
            Target::Tuple(ts, _) => ts.iter().flat_map(|t| self.target_places(t)).collect(),
            Target::Place { name, fields, .. } => {
                let (slots, mut ty) = self.variable(&name.text);
                let mut place = Place::Slots(slots);
                for step in fields {
                    (place, ty) = match step {
                        Step::Index(i) => (place.narrow(slice_of(&ty, *i)), element(&ty, *i)),
                        Step::Name(n) => {
                            let i = field_index(&ty, &n.text);
                            (place.narrow(slice_of(&ty, i)), element(&ty, i))
                        }
                        // `ages["Rex"] := 3`: the place of the key's value, added if missing.
                        Step::At(i) if matches!(ty, Type::Map(..)) => {
                            let Type::Map(k, v) = ty.clone() else { unreachable!() };
                            let map = self.read(&place)[0];
                            let key = self.expr(i);
                            (self.entry(map, &key, &k, &v), *v)
                        }
                        Step::At(i) => self.element_place(place, &ty, i),
                    };
                }
                vec![(place, ty)]
            }
        }
    }

    /// Where the value of `e` lives (`rex.age`, `l[i].name`), and its type. A computed value
    /// (`make_dog.age`) lives in fresh slots.
    fn place(&mut self, e: &Expr) -> (Place, Type) {
        let ty = self.ty(e);
        match &e.kind {
            ExprKind::Var(name) => (Place::Slots(self.slots_of(name)), ty),
            ExprKind::Mark(_, x) => self.place(x),
            ExprKind::Field(x, i) => {
                let (p, t) = self.place(x);
                (p.narrow(slice_of(&t, *i)), ty)
            }
            ExprKind::Method { receiver, name, .. } if self.ty(receiver).is_field(&name.text) => {
                let (p, t) = self.place(receiver);
                (p.narrow(slice_of(&t, field_index(&t, &name.text))), ty)
            }
            ExprKind::Index(x, i) if !matches!(self.ty(x), Type::Map(..)) && i.range().is_none() => {
                let (p, t) = self.place(x);
                self.element_place(p, &t, i)
            }
            _ => (Place::Slots(self.expr(e)), ty),
        }
    }

    /// The position of `key` in the Map `map` (keys of type `k`), or -1; also a value in a Set.
    fn find(&mut self, map: Local, key: &[Local], k: &Type) -> Local {
        let f = self.helper(Helper::Find(k.clone()));
        let d = self.int_local();
        self.emit(Inst::Call(vec![d], f, [&[map], key].concat()));
        d
    }

    /// Inside the `Find` helper. The index (`Inst::ListIndex`) is a `List<(Int, Int)>`: element 0
    /// = (how many of the Map's elements it holds, 0), then a table of `cap` places, each (0, 0)
    /// when empty or (mixed hash, position + 1) of a key, at `1 + hash % cap` or after, at the
    /// first empty place. Made again when it would be half full (it then has 4 places per
    /// element), added to for the elements added since. `ListRemove` takes the element out of
    /// it (`unindex` in runtime.c and interp.rs).
    fn find_in_index(&mut self, map: Local, key: &[Local], k: &Type) -> Local {
        let (zero, one, two) = (self.constant(0), self.constant(1), self.constant(2));
        let (n, index, cap, test) = (self.int_local(), self.int_local(), self.int_local(), self.int_local());
        self.emit(Inst::ListLength(n, map));
        self.emit(Inst::ListIndex(index, map));
        // Made again when there is none, or when it is half full.
        let (check, remake, ready) = (self.new_block(), self.new_block(), self.new_block());
        self.emit(Inst::Binary(test, Op::Eq, index, zero, NOWHERE));
        self.go(End::Branch(test, remake, check), check);
        self.emit(Inst::ListLength(cap, index));
        self.emit(Inst::Binary(cap, Op::Minus, cap, one, NOWHERE));
        let twice = self.int_local();
        self.emit(Inst::Binary(twice, Op::Times, n, two, NOWHERE));
        self.emit(Inst::Binary(test, Op::Ge, twice, cap, NOWHERE));
        self.go(End::Branch(test, remake, ready), remake);
        self.emit(Inst::ListFree(index));
        let wanted = self.int_local();
        self.emit(Inst::Binary(wanted, Op::Times, twice, two, NOWHERE));
        let sixteen = self.constant(16);
        self.emit(Inst::Copy(cap, sixteen));
        let (double, made) = (self.new_block(), self.new_block());
        let grow = self.new_block();
        self.go(End::Jump(grow), grow);
        self.emit(Inst::Binary(test, Op::Lt, cap, wanted, NOWHERE));
        self.go(End::Branch(test, double, made), double);
        self.emit(Inst::Binary(cap, Op::Times, cap, two, NOWHERE));
        self.go(End::Jump(grow), made);
        self.emit(Inst::ListNew(index, 2));
        let (i, place, mixed) = (self.constant(0), self.int_local(), self.int_local());
        let (fill, more, filled) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(fill), fill);
        self.emit(Inst::Binary(test, Op::Le, i, cap, NOWHERE));
        self.go(End::Branch(test, more, filled), more);
        self.emit(Inst::ListGrow(place, index));
        self.emit(Inst::Binary(i, Op::Plus, i, one, NOWHERE));
        self.go(End::Jump(fill), filled);
        self.emit(Inst::ListSetIndex(map, index));
        self.go(End::Jump(ready), ready);
        // The elements added since: their position + 1 at the first empty place from their hash.
        let done = self.int_local();
        self.emit(Inst::ListLoad(done, index, zero, 0));
        let (head, add, caught_up) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(head), head);
        self.emit(Inst::Binary(test, Op::Lt, done, n, NOWHERE));
        self.go(End::Branch(test, add, caught_up), add);
        let keys = self.load(map, done, k);
        let h = self.hash(&keys, k);
        self.first_place(h, cap, place, mixed);
        let (probe, taken, free) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(probe), probe);
        let there = self.int_local();
        self.emit(Inst::ListLoad(there, index, place, 1));
        self.go(End::Branch(there, taken, free), taken);
        self.next_place(cap, place);
        self.go(End::Jump(probe), free);
        self.emit(Inst::Binary(done, Op::Plus, done, one, NOWHERE));
        self.emit(Inst::ListStore(index, place, 0, mixed));
        self.emit(Inst::ListStore(index, place, 1, done));
        self.go(End::Jump(head), caught_up);
        self.emit(Inst::ListStore(index, zero, 0, done));
        // The key: from its place, until an empty one.
        let h = self.hash(key, k);
        self.first_place(h, cap, place, mixed);
        let found = self.constant(-1);
        let (probe, look, missing, same, other) = (self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(probe), probe);
        let at = self.int_local();
        self.emit(Inst::ListLoad(at, index, place, 1));
        self.go(End::Branch(at, look, missing), look);
        self.emit(Inst::Binary(at, Op::Minus, at, one, NOWHERE));
        let keys = self.load(map, at, k);
        let eq = self.equal(&keys, key, k, Span::new(0, 0));
        self.go(End::Branch(eq, same, other), same);
        self.emit(Inst::Copy(found, at));
        self.go(End::Jump(missing), other);
        self.next_place(cap, place);
        self.go(End::Jump(probe), missing);
        found
    }

    /// `place := 1 + mixed % cap` (place 0 of the index is its count), `mixed` = the hash mixed
    /// once more, so that it is never negative and that keys following each other spread out.
    fn first_place(&mut self, h: Local, cap: Local, place: Local, mixed: Local) {
        let (zero, one) = (self.constant(0), self.constant(1));
        self.emit(Inst::Mix(mixed, h, zero));
        self.emit(Inst::Binary(place, Op::Rem, mixed, cap, NOWHERE));
        self.emit(Inst::Binary(place, Op::Plus, place, one, NOWHERE));
    }

    /// The place after `place`, back to 1 after the last one.
    fn next_place(&mut self, cap: Local, place: Local) {
        let (one, past, wrap, go_on) = (self.constant(1), self.int_local(), self.new_block(), self.new_block());
        self.emit(Inst::Binary(place, Op::Plus, place, one, NOWHERE));
        self.emit(Inst::Binary(past, Op::Gt, place, cap, NOWHERE));
        self.go(End::Branch(past, wrap, go_on), wrap);
        self.emit(Inst::Copy(place, one));
        self.go(End::Jump(go_on), go_on);
    }

    /// Adds `v` (owned) to the Set `set`, or frees it if an equal value is already there.
    fn set_add(&mut self, set: Local, v: &[Local], elem: &Type) {
        let i = self.find(set, v, elem);
        let (zero, missing) = (self.constant(0), self.int_local());
        self.emit(Inst::Binary(missing, Op::Lt, i, zero, NOWHERE));
        let (add, there, done) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Branch(missing, add, there), add);
        self.push(set, v);
        self.go(End::Jump(done), there);
        self.free_value(v, elem);
        self.go(End::Jump(done), done);
    }

    /// Where the value of `key` lives in the Map `map`: added at the end if missing
    /// (with a copy of the key; its value's slots hold 0, so freeing them does nothing).
    fn entry(&mut self, map: Local, key: &[Local], k: &Type, v: &Type) -> Place {
        let i = self.find(map, key, k);
        let (zero, missing) = (self.constant(0), self.int_local());
        self.emit(Inst::Binary(missing, Op::Lt, i, zero, NOWHERE));
        let (add, done) = (self.new_block(), self.new_block());
        self.go(End::Branch(missing, add, done), add);
        let copy = self.clone_value(key, k);
        self.emit(Inst::ListGrow(i, map));
        for (s, &l) in copy.iter().enumerate() {
            self.emit(Inst::ListStore(map, i, s, l));
        }
        self.go(End::Jump(done), done);
        Place::Element { list: map, index: i, start: scalars(k).len(), kinds: scalars(v) }
    }

    /// `ages["Rex"]` (`remove`: and takes it out): an `Int?`, a copy of the value or `none`.
    fn lookup(&mut self, map: Local, key: &[Local], k: &Type, v: &Type, remove: bool) -> Slots {
        let i = self.find(map, key, k);
        let ty = Type::Optional(Box::new(v.clone()));
        let result = self.new_slots(&ty);
        let zero = self.constant(0);
        self.emit(Inst::Binary(result[0], Op::Ge, i, zero, NOWHERE));
        let (yes, no, done) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Branch(result[0], yes, no), yes);
        let n = scalars(k).len();
        let value = self.new_slots(v);
        for (s, &d) in value.iter().enumerate() {
            self.emit(Inst::ListLoad(d, map, i, n + s));
        }
        if remove {
            // The key goes, the value is given to the caller.
            let keys = self.load(map, i, k);
            self.free_value(&keys, k);
            self.copy(&result[1..], &value);
            self.emit(Inst::ListRemove(map, i));
        } else {
            let copy = self.clone_value(&value, v);
            self.copy(&result[1..], &copy);
        }
        self.go(End::Jump(done), no);
        for &l in &result[1..] {
            self.emit(Inst::Const(l, 0));
        }
        self.go(End::Jump(done), done);
        self.track(&result, &ty);
        result
    }

    /// Element `index` of the List at `list` (of type `ty`): the index is checked here.
    fn element_place(&mut self, list: Place, ty: &Type, index: &Expr) -> (Place, Type) {
        let Type::List(elem) = ty else { unreachable!("List checked by the type checker") };
        // The address of the List's block: it does not move when the List grows.
        let list = self.read(&list)[0];
        let i = self.expr1(index);
        let loc = self.loc(index.span);
        self.emit(Inst::ListCheck(list, i, loc));
        (Place::Element { list, index: i, start: 0, kinds: scalars(elem) }, (**elem).clone())
    }

    /// The slots of a place: its own ones, or a copy of an element's (borrowed from the List).
    fn read(&mut self, p: &Place) -> Slots {
        match p {
            Place::Slots(s) => s.clone(),
            Place::Element { list, index, start, kinds } => {
                let v: Slots = kinds.iter().map(|&k| self.new_local(k)).collect();
                for (k, &d) in v.iter().enumerate() {
                    self.emit(Inst::ListLoad(d, *list, *index, start + k));
                }
                v
            }
        }
    }

    fn write(&mut self, p: &Place, v: &[Local]) {
        match p {
            Place::Slots(s) => self.copy(s, v),
            Place::Element { list, index, start, .. } => {
                for (k, &l) in v.iter().enumerate() {
                    self.emit(Inst::ListStore(*list, *index, start + k, l));
                }
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
        self.ty(e) == Type::Float
    }
    /// `dst := a op b`, as integers or floats depending on the type of `a`.
    fn operation(&mut self, op: Op, a: Local, b: Local, float: bool, loc: Loc) -> Local {
        let float_result = float && matches!(op, Op::Plus | Op::Minus | Op::Times | Op::Div);
        let dst = self.new_local(if float_result { Scalar::Float } else { Scalar::Int });
        self.emit(if float { Inst::FBinary(dst, op, a, b, loc) } else { Inst::Binary(dst, op, a, b, loc) });
        dst
    }

    /// Runs `body` with `name` bound to slots it borrows: no block frees them.
    fn borrowing(&mut self, name: &str, v: Slots, ty: Type, body: &Block) {
        self.names.push(Scope { borrowed: true, vars: HashMap::from([(name.to_string(), (v, ty))]) });
        self.block(body);
        self.names.pop();
    }

    fn block(&mut self, b: &Block) {
        self.names.push(Scope::default());
        for s in &b.stmts {
            self.stmt(s);
        }
        self.free_scopes(self.names.len() - 1);
        self.names.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        if self.watching {
            self.line(s);
        }
        let mark = self.temps.len();
        self.statement(s);
        self.free_temps(mark);
    }

    fn statement(&mut self, s: &Stmt) {
        match s {
            Stmt::Declare { pattern, value, .. } => {
                let v = self.owned(value);
                let ty = self.given(value);
                let slots = self.new_slots(&ty);
                self.copy(&slots, &v);
                self.bind(pattern, &slots, &ty);
            }
            // The value first, then the places (a call in the value may change a List: `l[0] := f(var l)`).
            Stmt::Assign { target, op, value } => match op {
                // `(a, b) := (b, a)`: read everything before writing, otherwise `a` would be overwritten too early.
                None => {
                    let v = self.owned(value);
                    let ty = self.given(value);
                    let new = self.new_slots(&ty);
                    self.copy(&new, &v);
                    let mut at = 0;
                    for (place, ty) in self.target_places(target) {
                        let n = scalars(&ty).len();
                        let old = self.read(&place);
                        self.free_value(&old, &ty);
                        self.write(&place, &new[at..at + n]);
                        at += n;
                    }
                }
                Some(op) => {
                    let v = self.expr1(value);
                    let (place, ty) = self.target_places(target).remove(0);
                    let old = self.read(&place)[0];
                    let r = if ty == Type::String {
                        let r = self.int_local();
                        self.emit(Inst::Str(r, StrOp::Concat, vec![old, v]));
                        self.emit(Inst::Free(old));
                        r
                    } else if ty == Type::BigInt {
                        let loc = self.loc(value.span);
                        let r = self.big(*op, old, v, loc);
                        self.emit(Inst::Free(old));
                        r
                    } else {
                        let loc = self.loc(target.span());
                        self.operation(kir_op(*op), old, v, self.is_float(value), loc)
                    };
                    self.write(&place, &[r]);
                }
            },
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
                self.narrow(&narrowed(cond, true));
                self.block(then);
                self.names.pop();
                self.go(End::Jump(b_after), b_else);
                if let Some(o) = otherwise {
                    self.narrow(&narrowed(cond, false));
                    self.block(o);
                    self.names.pop();
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
                self.borrowing(&counter.text, vec![i], Type::Int, body);
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
            Stmt::Each { name, value, list, body } => {
                let elem = match self.ty(list) {
                    Type::List(elem) | Type::Set(elem) => *elem,
                    Type::Map(k, v) => pair(&k, &v),
                    _ => unreachable!("List or Map checked by the type checker"),
                };
                // The List cannot change during the loop (`ownership.rs`): its length is read once.
                let l = self.expr1(list);
                let n = self.int_local();
                self.emit(Inst::ListLength(n, l));
                let i = self.constant(0);
                let (head, b_body, next, exit) = (self.new_block(), self.new_block(), self.new_block(), self.new_block());
                self.go(End::Jump(head), head);
                let more = self.int_local();
                self.emit(Inst::Binary(more, Op::Lt, i, n, NOWHERE));
                self.go(End::Branch(more, b_body, exit), b_body);
                let v = self.load(l, i, &elem);
                self.loops.push((next, exit, self.names.len()));
                // Borrowed from the List: nobody frees it.
                match value {
                    None => self.borrowing(&name.text, v, elem, body),
                    // `for name, age in ages`: the key's slots, then the value's.
                    Some(value) => {
                        let (k, t) = (element(&elem, 0), element(&elem, 1));
                        let n = scalars(&k).len();
                        let vars = HashMap::from([(name.text.clone(), (v[..n].to_vec(), k)), (value.text.clone(), (v[n..].to_vec(), t))]);
                        self.names.push(Scope { borrowed: true, vars });
                        self.block(body);
                        self.names.pop();
                    }
                }
                self.loops.pop();
                self.go(End::Jump(next), next);
                let one = self.constant(1);
                self.emit(Inst::Binary(i, Op::Plus, i, one, NOWHERE));
                self.go(End::Jump(head), exit);
            }
            Stmt::Match(m) => self.match_(m, None),
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
        for (l, kind) in self.temps.clone().into_iter().rev() {
            self.free_slot(l, &kind);
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
        m.extend([
            Piece::Text("\n  --> ".into()),
            Piece::File(l.file),
            Piece::Text(format!(":{}:{}\n  {} {}\n", l.line, l.col, c.kind.keyword(), cond.join(" "))),
        ]);
        let mut names = vec![];
        names_in(&c.cond, &mut names);
        let values: Vec<(String, (Slots, Type))> = names
            .into_iter()
            .filter_map(|n| {
                let v = if n == "result" { self.result.clone() } else { self.names.iter().rev().find_map(|t| t.vars.get(&n)).cloned() };
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
            // For debugging: the fields themselves, even for a type following `Display`.
            // ponytail: an enum's fields are shown by a shared helper, which still uses `to_string`.
            self.debug = true;
            self.value_pieces(v, ty, &mut m);
            self.debug = false;
        }
        if !values.is_empty() {
            m.push(Piece::Text("\n".into()));
        }
        if let Some((line, col)) = site {
            m.extend([
                Piece::Text("  ".into()),
                Piece::Label("contract_called_from"),
                Piece::Value(line, Show::Site),
                Piece::Text(":".into()),
                Piece::Value(col, Show::Int),
                Piece::Text("\n".into()),
            ]);
        }
        // The program stops: the temporaries of the message are never freed.
        self.temps.truncate(mark);
        self.emit(Inst::Fail(m));
        self.go(End::Unreachable, next);
    }

    /// A value the way it would be written in Kova: `3`, `2.5`, `'a'`, `(1, true)`,
    /// `Dog { name: "Rex", age: 3 }`, `[1, 2]`.
    fn value_pieces(&mut self, v: &[Local], ty: &Type, m: &mut Vec<Piece>) {
        match ty {
            // A type following `Display` is always shown by its `to_string`.
            Type::Struct(_) | Type::Enum(_) if self.info.follows(ty, "Display") && !self.debug => {
                let text = self.call(&method_name(&ty.text(), "to_string"), v.to_vec(), vec![], Span::new(0, 0), &Type::String);
                m.push(Piece::Value(text[0], Show::String));
            }
            Type::Struct(d) if d.fields.is_empty() => m.push(Piece::Text(format!("{} {{}}", ty.text()))),
            Type::Struct(d) => {
                m.push(Piece::Text(format!("{} {{ ", ty.text())));
                for (i, (name, t)) in d.fields.iter().enumerate() {
                    let sep = if i > 0 { ", " } else { "" };
                    m.push(Piece::Text(format!("{sep}{name}: ")));
                    let (start, n) = slice_of(ty, i);
                    self.value_pieces(&v[start..start + n], t, m);
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
                    self.value_pieces(&v[start..start + n], t, m);
                }
                m.push(Piece::Text(")".into()));
            }
            // Its text is made by a helper, and freed at the end of the statement.
            Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Any(_) => {
                let f = self.helper(Helper::Show(ty.clone()));
                let text = self.int_local();
                self.emit(Inst::Call(vec![text], f, v.to_vec()));
                self.temps.push((text, Type::String));
                m.push(Piece::Value(text, Show::String));
            }
            Type::Block(..) => m.push(Piece::Text(ty.text())),
            Type::Char | Type::String => {
                let quote = if *ty == Type::Char { "'" } else { "\"" };
                m.push(Piece::Text(quote.into()));
                m.push(Piece::Value(v[0], Show::of(ty)));
                m.push(Piece::Text(quote.into()));
            }
            _ => m.push(Piece::Value(v[0], Show::of(ty))),
        }
    }

    /// The pieces of a part of a text: a String or a number as `print` shows it, a struct
    /// as `Dog { name: "Rex", age: 3 }`, a tuple as `(1, "a")`.
    fn part_pieces(&mut self, p: &Part, m: &mut Vec<Piece>) {
        match p {
            Part::Text(t) => m.push(Piece::Text(t.clone())),
            Part::Code(e) => match &self.ty(e) {
                ty @ (Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_)) => {
                    let ty = ty.clone();
                    let v = self.expr(e);
                    self.value_pieces(&v, &ty, m);
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
                Piece::Label(_) | Piece::File(_) => unreachable!("only in contract messages"),
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
    fn arguments(&mut self, callee: &str, args: &[Expr], vals: &mut Slots, places: &mut Vec<(Place, Slots)>) {
        let mark = self.envs.len();
        self.arguments_from(callee, args, vals, places);
        self.env_marks.push(mark);
    }

    fn arguments_from(&mut self, callee: &str, args: &[Expr], vals: &mut Slots, places: &mut Vec<(Place, Slots)>) {
        let modes = self.signatures[callee].1.clone();
        for (a, mode) in args.iter().zip(modes) {
            let value = if let ExprKind::Mark(_, x) = &a.kind { x } else { a };
            match mode {
                Mode::Read => vals.extend(self.expr(value)),
                Mode::Move => vals.extend(self.owned(value)),
                Mode::Var => {
                    let (p, ty) = self.place(value);
                    let v = self.read(&p);
                    let out = self.new_slots(&ty);
                    vals.extend(&v);
                    places.push((p, out));
                }
            }
        }
    }

    /// `results := f(vals)`, with the call location for a function that has contracts.
    /// `places`: what `var self` and the `var` parameters change, written back after the call
    /// (copy-in / copy-out, like Swift's `inout`).
    fn call(&mut self, name: &str, vals: Slots, places: Vec<(Place, Slots)>, span: Span, ty: &Type) -> Slots {
        let num = self.numbers[name];
        self.call_to(num, self.sites[num], vals, places, span, ty)
    }

    /// The call `id` of `name`; a generic one calls its copy for the types of this call
    /// (`larger<Int>`, `Stack.push<Int>`), with the caller's own `T` replaced too.
    fn call_at(&mut self, id: usize, name: &str, vals: Slots, places: Vec<(Place, Slots)>, span: Span, ty: &Type) -> Slots {
        let Some(types) = self.info.type_args.get(&id) else {
            return self.call(name, vals, places, span, ty);
        };
        let types = types.iter().map(|t| substitute(t, &self.subst)).collect();
        let num = self.helper(Helper::Instance(name.to_string(), types));
        self.call_to(num, self.generic_sites[name], vals, places, span, ty)
    }

    /// Calls function `num` (`site`: it receives the call location).
    fn call_to(&mut self, num: usize, site: bool, mut vals: Slots, places: Vec<(Place, Slots)>, span: Span, ty: &Type) -> Slots {
        if site {
            let l = self.loc(span);
            let (line, col) = (self.constant(l.packed()), self.constant(l.col as i64));
            vals.extend([line, col]);
        }
        let result = self.new_slots(ty);
        let outs: Slots = places.iter().flat_map(|(_, out)| out.clone()).collect();
        self.emit(Inst::Call([result.clone(), outs].concat(), num, vals));
        for (place, out) in &places {
            self.write(place, out);
        }
        if let Some(mark) = self.env_marks.pop() {
            self.give_back(mark);
        }
        self.track(&result, ty);
        result
    }

    /// After a call: what its blocks changed goes back into the variables, and their boxes are freed.
    fn give_back(&mut self, mark: usize) {
        for (env, held) in self.envs.split_off(mark) {
            if held.is_empty() {
                continue;
            }
            let zero = self.constant(0);
            for (k, &l) in held.iter().flatten().enumerate() {
                self.emit(Inst::ListLoad(l, env, zero, k));
            }
            self.emit(Inst::ListFree(env));
        }
    }

    /// A block given to a function: its function, and a box holding a copy of the variables it
    /// uses around it (`Info::captures`). The block's function reads them from the box and writes
    /// them back before returning; `give_back` copies them into the variables after the call.
    fn block_value_of(&mut self, e: &Expr, params: &[ast::Param], body: &Block) -> Slots {
        let Type::Block(ps, r) = self.ty(e) else { unreachable!("block checked by the type checker") };
        let held: Vec<(String, Slots, Type)> = (self.info.captures.get(&e.id).into_iter().flatten())
            .map(|n| {
                let (v, t) = self.variable(n);
                (n.clone(), v, t)
            })
            .collect();
        let slots: Slots = held.iter().flat_map(|(_, v, _)| v.clone()).collect();
        let env = if slots.is_empty() { self.constant(0) } else {
            let env = self.int_local();
            self.emit(Inst::ListNew(env, slots.len()));
            self.push(env, &slots);
            env
        };
        self.envs.push((env, held.iter().map(|(_, v, _)| v.clone()).collect()));
        let mut t = Translator {
            src: self.src,
            info: self.info,
            subst: self.subst.clone(),
            debug: false,
            numbers: self.numbers,
            signatures: self.signatures,
            sites: self.sites,
            generic_sites: self.generic_sites,
            helpers: self.helpers,
            made: self.made,
            owner: format!("block in {}", self.owner.trim_start_matches("block in ")),
            first_helper: self.first_helper,
            contracts: self.contracts,
            built: self.built,
            watching: self.watching,
            ensures: vec![],
            ret: (*r).clone(),
            result: None,
            site: None,
            outs: vec![],
            names: vec![Scope::borrowed(), Scope::default()],
            locals: vec![],
            blocks: vec![],
            current: 0,
            loops: vec![],
            temps: vec![],
            envs: vec![],
            env_marks: vec![],
        };
        let inner_env = t.int_local();
        for (p, ty) in params.iter().zip(&ps) {
            let v = t.new_slots(ty);
            t.names[0].vars.insert(p.name.text.clone(), (v, ty.clone()));
        }
        let nb_params = t.locals.len();
        t.current = t.new_block();
        let zero = t.constant(0);
        let mut inside = vec![];
        for (name, _, ty) in &held {
            let v = t.new_slots(ty);
            t.names[0].vars.insert(name.clone(), (v.clone(), ty.clone()));
            inside.extend(v);
        }
        for (k, &l) in inside.iter().enumerate() {
            t.emit(Inst::ListLoad(l, inner_env, zero, k));
        }
        let v = if *r == Type::Unit {
            t.block(body);
            vec![]
        } else {
            t.block_value(body, &r)
        };
        // The temporaries of its last line (its value is owned, not among them).
        t.free_temps(0);
        for (k, &l) in inside.iter().enumerate() {
            t.emit(Inst::ListStore(inner_env, zero, k, l));
        }
        t.finish(End::Return(v));
        let name = t.owner.clone();
        let f = t.function(name, nb_params, scalars(&r));
        let k = {
            let mut made = self.made.borrow_mut();
            made.push(Some(f));
            made.len() - 1
        };
        let num = self.helper(Helper::Block(k));
        let fun = self.int_local();
        self.emit(Inst::FuncRef(fun, num));
        vec![fun, env]
    }

    /// Calls the block `b` (its function and its box) with `vals`.
    fn call_block(&mut self, b: &[Local], vals: Slots, ty: &Type) -> Slots {
        let result = self.new_slots(ty);
        self.emit(Inst::CallIndirect(result.clone(), b[0], [vec![b[1]], vals].concat()));
        result
    }

    /// `l.each { x: Int => … }`, `filter`, `map`, `any`, `all`, `fold`: a loop over the List with the
    /// block's body copied inside (like Lisaac, which inlines its blocks). The List cannot change
    /// meanwhile (`ownership.rs`): its length is read once.
    fn block_method(&mut self, e: &Expr, list: Local, elem: &Type, name: &str, args: &[Expr]) -> Slots {
        let block = args.last().expect("block checked by the type checker");
        // `l.each(f)`: a block parameter, called at each turn.
        let given = (!matches!(block.kind, ExprKind::Block { .. })).then(|| self.expr(block));
        let ty = self.ty(e);
        let one = |v: Slots| vec![(v, elem.clone())];
        match name {
            "each" => {
                self.each_element(list, elem, |t, _, v| {
                    t.run(block, &given, one(v), None);
                });
                vec![]
            }
            "filter" | "map" => {
                let Type::List(out_elem) = &ty else { unreachable!("a List checked by the type checker") };
                let out = self.int_local();
                self.emit(Inst::ListNew(out, scalars(out_elem).len()));
                self.each_element(list, elem, |t, _, v| {
                    if name == "map" {
                        let x = t.run(block, &given, one(v), Some(out_elem));
                        return t.push(out, &x);
                    }
                    let keep = t.run(block, &given, one(v.clone()), Some(&Type::Bool))[0];
                    let (yes, after) = (t.new_block(), t.new_block());
                    t.go(End::Branch(keep, yes, after), yes);
                    let copy = t.clone_value(&v, elem);
                    t.push(out, &copy);
                    t.go(End::Jump(after), after);
                });
                self.track(&[out], &ty);
                vec![out]
            }
            // `any` stops testing at the first `true`, `all` at the first `false`.
            "any" | "all" => {
                let all = name == "all";
                let answer = self.constant(all as i64);
                self.each_element(list, elem, |t, _, v| {
                    let (test, after) = (t.new_block(), t.new_block());
                    let end = if all { End::Branch(answer, test, after) } else { End::Branch(answer, after, test) };
                    t.go(end, test);
                    let c = t.run(block, &given, one(v), Some(&Type::Bool))[0];
                    t.emit(Inst::Copy(answer, c));
                    t.go(End::Jump(after), after);
                });
                vec![answer]
            }
            // `fold(start)`: the block gets the value so far and an element, and gives the next value.
            _ => {
                let start = self.owned(&args[0]);
                let acc = self.new_slots(&ty);
                self.copy(&acc, &start);
                self.each_element(list, elem, |t, _, v| {
                    let next = t.run(block, &given, vec![(acc.clone(), ty.clone()), (v, elem.clone())], Some(&ty));
                    t.free_value(&acc, &ty);
                    t.copy(&acc, &next);
                });
                self.track(&acc, &ty);
                acc
            }
        }
    }

    /// `l[a..b]`: a new List of copies of the elements `a` to `b` (without `b`; with it after `..=`).
    /// Ends outside the List, or `b` before `a`, stop the program like `l[i]`.
    fn slice(&mut self, e: &Expr, x: &Expr, i: &Expr) -> Slots {
        let (inclusive, a, b) = i.range().unwrap();
        let ty = self.ty(e);
        let string = ty == Type::String;
        let list = self.expr1(x);
        let (i, end, zero, one) = (self.int_local(), self.int_local(), self.constant(0), self.constant(1));
        let first = self.expr1(a);
        self.emit(Inst::Copy(i, first));
        let last = self.expr1(b);
        self.emit(Inst::Binary(end, if inclusive { Op::Plus } else { Op::Minus }, last, if inclusive { one } else { zero }, NOWHERE));
        let (n, bad, loc) = (self.int_local(), self.int_local(), self.loc(e.span));
        // In characters for a String.
        self.emit(if string { Inst::Str(n, StrOp::Length, vec![list]) } else { Inst::ListLength(n, list) });
        for (op, l, r) in [(Op::Lt, i, zero), (Op::Lt, end, i), (Op::Gt, end, n)] {
            self.emit(Inst::Binary(bad, op, l, r, NOWHERE));
            self.emit(Inst::FaultIf(bad, Fault::Index, loc));
        }
        if string {
            let text = self.int_local();
            self.emit(Inst::Str(text, StrOp::Slice, vec![list, i, end]));
            self.temps.push((text, Type::String));
            return vec![text];
        }
        let Type::List(elem) = &ty else { unreachable!("a List checked by the type checker") };
        let out = self.int_local();
        self.emit(Inst::ListNew(out, scalars(elem).len()));
        let (test, body, done) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(test), test);
        let more = self.int_local();
        self.emit(Inst::Binary(more, Op::Lt, i, end, NOWHERE));
        self.go(End::Branch(more, body, done), body);
        let v = self.load(list, i, elem);
        let copy = self.clone_value(&v, elem);
        self.push(out, &copy);
        self.emit(Inst::Binary(i, Op::Plus, i, one, NOWHERE));
        self.go(End::Jump(test), done);
        self.track(&[out], &ty);
        vec![out]
    }

    /// `n.abs`, `a.min(b)`, `n.pow(k)`, `x.sqrt`, `x.round`… on the number `v` (Part 4, "Standard
    /// library v2"): the same instructions in run and build, so the same digits.
    fn number_method(&mut self, e: &Expr, v: Local, name: &str, args: &[Expr]) -> Slots {
        let ExprKind::Method { receiver, name: n, .. } = &e.kind else { unreachable!("a method call") };
        let ty = self.ty(receiver);
        let float = ty == Type::Float;
        let loc = self.loc(n.span);
        let out = self.new_slots(&ty)[0];
        let f = |op| match op {
            "abs" => FloatOp::Abs,
            "sqrt" => FloatOp::Sqrt,
            "floor" => FloatOp::Floor,
            "ceil" => FloatOp::Ceil,
            _ => FloatOp::Round,
        };
        match name {
            _ if float && matches!(name, "abs" | "sqrt" | "floor" | "ceil" | "round") => {
                self.emit(Inst::FUnary(out, f(name), v));
                if name == "abs" || name == "sqrt" {
                    return vec![out];
                }
                // Too big or NaN: the same error as `x.to_int`.
                let n = self.int_local();
                self.emit(Inst::Convert(n, Conversion::FloatToInt, out, loc));
                vec![n]
            }
            // `-n`, an overflow for the smallest `Int`.
            "abs" => {
                let (zero, negative) = (self.constant(0), self.int_local());
                self.emit(Inst::Copy(out, v));
                self.emit(Inst::Binary(negative, Op::Lt, v, zero, NOWHERE));
                let (flip, done) = (self.new_block(), self.new_block());
                self.go(End::Branch(negative, flip, done), flip);
                self.emit(Inst::Neg(out, v, loc));
                self.go(End::Jump(done), done);
                vec![out]
            }
            // `a.min(b)`: `b` if `b < a`, else `a`.
            "min" | "max" => {
                let b = self.expr1(&args[0]);
                self.emit(Inst::Copy(out, v));
                let (l, r) = if name == "min" { (b, v) } else { (v, b) };
                let other = self.operation(Op::Lt, l, r, float, NOWHERE);
                let (take, done) = (self.new_block(), self.new_block());
                self.go(End::Branch(other, take, done), take);
                self.emit(Inst::Copy(out, b));
                self.go(End::Jump(done), done);
                vec![out]
            }
            // `pow`: squared again and again (`x⁵ = x · x⁴`); a Float to a negative power is `1 / x^k`.
            _ => {
                let k = self.int_local();
                let given = self.expr1(&args[0]);
                self.emit(Inst::Copy(k, given));
                let (zero, two) = (self.constant(0), self.constant(2));
                let negative = self.int_local();
                self.emit(Inst::Binary(negative, Op::Lt, k, zero, NOWHERE));
                if !float {
                    self.emit(Inst::FaultIf(negative, Fault::NegativePower, loc));
                } else {
                    let (flip, go_on) = (self.new_block(), self.new_block());
                    self.go(End::Branch(negative, flip, go_on), flip);
                    self.emit(Inst::Neg(k, k, loc));
                    self.go(End::Jump(go_on), go_on);
                }
                let base = self.new_slots(&ty)[0];
                self.emit(Inst::Copy(base, v));
                self.emit(Inst::Const(out, if float { 1f64.to_bits() as i64 } else { 1 }));
                let (head, body, odd, after_odd, square, done) =
                    (self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block());
                self.go(End::Jump(head), head);
                let more = self.int_local();
                self.emit(Inst::Binary(more, Op::Gt, k, zero, NOWHERE));
                self.go(End::Branch(more, body, done), body);
                let bit = self.int_local();
                self.emit(Inst::Binary(bit, Op::Rem, k, two, NOWHERE));
                self.go(End::Branch(bit, odd, after_odd), odd);
                let times = self.operation(Op::Times, out, base, float, loc);
                self.emit(Inst::Copy(out, times));
                self.go(End::Jump(after_odd), after_odd);
                self.emit(Inst::Binary(k, Op::Div, k, two, NOWHERE));
                // Squared only if still needed: `x.pow(1)` must not overflow on `x * x`.
                let left = self.int_local();
                self.emit(Inst::Binary(left, Op::Gt, k, zero, NOWHERE));
                self.go(End::Branch(left, square, head), square);
                let squared = self.operation(Op::Times, base, base, float, loc);
                self.emit(Inst::Copy(base, squared));
                self.go(End::Jump(head), done);
                if float {
                    let (flip, after) = (self.new_block(), self.new_block());
                    self.go(End::Branch(negative, flip, after), flip);
                    let unit = self.new_slots(&ty)[0];
                    self.emit(Inst::Const(unit, 1f64.to_bits() as i64));
                    let inverse = self.operation(Op::Div, unit, out, true, loc);
                    self.emit(Inst::Copy(out, inverse));
                    self.go(End::Jump(after), after);
                }
                vec![out]
            }
        }
    }

    /// `l.sorted` or `l.sorted { a: T, b: T => … }`: a sorted copy. An element moves back while it
    /// comes before the previous one, so equal elements keep their order.
    /// ponytail: insertion sort, O(n²); a merge sort when Lists of thousands are sorted.
    fn sorted(&mut self, list: Local, elem: &Type, block: Option<&Expr>, span: Span) -> Slots {
        let ty = Type::List(Box::new(elem.clone()));
        let out = self.clone_value(&[list], &ty)[0];
        let given = block.filter(|b| !matches!(b.kind, ExprKind::Block { .. })).map(|b| self.expr(b));
        let n = self.int_local();
        self.emit(Inst::ListLength(n, out));
        let (i, j, one, zero) = (self.constant(1), self.int_local(), self.constant(1), self.constant(0));
        let (outer, start, inner, test, swap, next, done) =
            (self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(outer), outer);
        let more = self.int_local();
        self.emit(Inst::Binary(more, Op::Lt, i, n, NOWHERE));
        self.go(End::Branch(more, start, done), start);
        self.emit(Inst::Copy(j, i));
        self.go(End::Jump(inner), inner);
        let left = self.int_local();
        self.emit(Inst::Binary(left, Op::Gt, j, zero, NOWHERE));
        self.go(End::Branch(left, test, next), test);
        let before = self.int_local();
        self.emit(Inst::Binary(before, Op::Minus, j, one, NOWHERE));
        let (a, b) = (self.load(out, j, elem), self.load(out, before, elem));
        let first = match block {
            Some(block) => self.run(block, &given, vec![(a.clone(), elem.clone()), (b.clone(), elem.clone())], Some(&Type::Bool))[0],
            None => self.less(&a, &b, elem, span),
        };
        self.go(End::Branch(first, swap, next), swap);
        for (k, (&x, &y)) in a.iter().zip(&b).enumerate() {
            self.emit(Inst::ListStore(out, j, k, y));
            self.emit(Inst::ListStore(out, before, k, x));
        }
        self.emit(Inst::Copy(j, before));
        self.go(End::Jump(inner), next);
        self.emit(Inst::Binary(i, Op::Plus, i, one, NOWHERE));
        self.go(End::Jump(outer), done);
        self.track(&[out], &ty);
        vec![out]
    }

    /// Runs `block` on `vals`: its body copied here, or a call of the block `given`.
    fn run(&mut self, block: &Expr, given: &Option<Slots>, vals: Vec<(Slots, Type)>, result: Option<&Type>) -> Slots {
        match (given, &block.kind) {
            (Some(b), _) => self.call_block(b, vals.into_iter().flat_map(|(v, _)| v).collect(), result.unwrap_or(&Type::Unit)),
            (None, ExprKind::Block { params, body }) => self.inline(params, body, vals, result),
            (None, _) => unreachable!("block checked by the type checker"),
        }
    }

    /// The body of a block, here, with its parameters borrowing `vals`; with `result`, its value
    /// (owned). Its temporaries are freed at once: it runs at each turn of a loop.
    fn inline(&mut self, params: &[ast::Param], body: &Block, vals: Vec<(Slots, Type)>, result: Option<&Type>) -> Slots {
        let mark = self.temps.len();
        let vars = params.iter().zip(vals).map(|(p, v)| (p.name.text.clone(), v)).collect();
        self.names.push(Scope { borrowed: true, vars });
        let v = match result {
            Some(ty) => self.block_value(body, ty),
            None => {
                self.block(body);
                vec![]
            }
        };
        self.names.pop();
        self.free_temps(mark);
        v
    }

    /// `x.clone` on an `any`: the copy of its box, not a method of the interface.
    fn is_clone(&self, receiver: &Expr, name: &ast::Name) -> bool {
        let Type::Any(i) = self.ty(receiver) else { return false };
        name.text == "clone" && !self.signatures.contains_key(&method_name(&i, "clone"))
    }

    /// `rex.sound`: the struct method called (its name, `Dog.sound`, and how it receives `rex`).
    fn method(&self, receiver: &Expr, name: &ast::Name) -> Option<(String, Mode)> {
        let t = &self.ty(receiver);
        if !matches!(t, Type::Struct(_) | Type::Enum(_)) {
            return None;
        }
        let q = method_name(&t.owner(), &name.text);
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
        self.narrow(&narrowed(cond, true));
        let v = self.block_value(then, ty);
        self.names.pop();
        self.copy(&dst, &v);
        self.go(End::Jump(b_after), b_else);
        self.narrow(&narrowed(cond, false));
        let v = self.block_value(otherwise, ty);
        self.names.pop();
        self.copy(&dst, &v);
        self.go(End::Jump(b_after), b_after);
        self.track(&dst, ty);
        dst
    }

    /// The value of a block: that of its last line (checked by the type checker).
    fn block_value(&mut self, b: &Block, ty: &Type) -> Slots {
        self.names.push(Scope::default());
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
            Stmt::Match(m) => {
                let v = self.new_slots(ty);
                self.match_(m, Some((&v, ty)));
                v
            }
            _ => unreachable!("block checked by the type checker"),
        };
        self.free_scopes(self.names.len() - 1);
        self.names.pop();
        v
    }

    /// `match`: jumps to the arm of the value's variant; with `dst`, each arm's value goes there.
    /// On a place (`match shape`), the arms borrow its fields; a copied value is copied first, so
    /// that the arms may change the place. A computed value belongs to the `match`: each arm frees
    /// what it does not bind.
    fn match_(&mut self, m: &Match, dst: Option<(&[Local], &Type)>) {
        let ty = self.ty(&m.subject);
        let d = &enum_view(&ty).expect("enum checked by the type checker");
        let borrowed = place_root(&m.subject, &self.info.types).is_some();
        let v = if borrowed {
            let (p, _) = self.place(&m.subject);
            let v = self.read(&p);
            if is_copy(&ty) {
                let c = self.new_slots(&ty);
                self.copy(&c, &v);
                c
            } else {
                v
            }
        } else {
            let v = self.expr(&m.subject);
            self.untrack(&v);
            v
        };
        let after = self.new_block();
        for (k, arm) in m.arms.iter().enumerate() {
            let (body, next) = (self.new_block(), self.new_block());
            match arm.case.variant() {
                // The last arm takes what is left: the type checker made sure nothing else is.
                Some((name, _)) if k + 1 < m.arms.len() => {
                    let (number, test) = (self.constant(d.variant(name).unwrap().0 as i64), self.int_local());
                    self.emit(Inst::Binary(test, Op::Eq, v[0], number, NOWHERE));
                    self.go(End::Branch(test, body, next), body);
                }
                _ => self.go(End::Jump(body), body),
            }
            self.names.push(Scope { borrowed, vars: HashMap::new() });
            match arm.case.variant() {
                Some((name, binds)) if !binds.is_empty() => {
                    let (i, fields) = d.variant(name).unwrap();
                    let mut at = variant_slice(d, i).0;
                    for (p, (_, t)) in binds.into_iter().zip(fields) {
                        let n = scalars(t).len();
                        self.bind(p, &v[at..at + n], t);
                        at += n;
                    }
                }
                _ if !borrowed => self.free_value(&v, &ty),
                _ => {}
            }
            match dst {
                Some((dst, t)) => {
                    let value = self.block_value(&arm.body, t);
                    self.copy(dst, &value);
                }
                None => self.block(&arm.body),
            }
            self.free_scopes(self.names.len() - 1);
            self.names.pop();
            self.go(End::Jump(after), next);
        }
        self.go(End::Unreachable, after);
    }

    /// An expression whose value fits in a single slot (Int, Bool…).
    fn expr1(&mut self, e: &Expr) -> Local {
        self.expr(e)[0]
    }

    fn expr(&mut self, e: &Expr) -> Slots {
        let v = self.value(e);
        let v = self.boxed(e, v, false);
        self.wrapped(e, v)
    }

    fn value(&mut self, e: &Expr) -> Slots {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::BigInt(_) | ExprKind::Unary(UnOp::Neg, _) if self.ty(e) == Type::BigInt && big_literal(e).is_some() => {
                vec![self.big_text(&big_literal(e).unwrap())]
            }
            ExprKind::Int(v) if self.ty(e) == Type::Float => vec![self.float_constant(*v as f64)],
            ExprKind::Int(v) => vec![self.constant(*v as i64)],
            ExprKind::Float(v) => vec![self.float_constant(*v)],
            ExprKind::Char(c) => vec![self.constant(*c as i64)],
            ExprKind::Bool(b) => vec![self.constant(*b as i64)],
            ExprKind::Str(text) => {
                let l = self.int_local();
                self.emit(Inst::Text(l, text.clone()));
                self.temps.push((l, Type::String));
                vec![l]
            }
            ExprKind::BigInt(digits) => vec![self.big_text(digits)],
            ExprKind::Interp(parts) => {
                let mut pieces = vec![];
                for p in parts {
                    self.part_pieces(p, &mut pieces);
                }
                let l = self.join(pieces);
                self.temps.push((l, Type::String));
                vec![l]
            }
            ExprKind::StructLit { fields, .. } => {
                // Computed in the written order, placed in the declared order.
                let ty = self.ty(e);
                let mut parts: Vec<Slots> = vec![vec![]; members(&ty).len()];
                for (name, value) in fields {
                    parts[field_index(&ty, &name.text)] = self.owned(value);
                }
                let v: Slots = parts.concat();
                self.track(&v, &ty);
                v
            }
            ExprKind::InMode { .. } => vec![self.constant(self.known(e).unwrap() as i64)],
            // `body` alone: calls the block without parameters.
            ExprKind::Var(name) if self.info.invoked.contains(&e.id) => {
                let (b, ty) = (self.slots_of(name), self.ty(e));
                let r = self.call_block(&b, vec![], &ty);
                self.track(&r, &ty);
                r
            }
            ExprKind::Var(name) => self.slots_of(name),
            // `test(x)`: calls the block `test`.
            ExprKind::Call { name, args } if self.info.invoked.contains(&e.id) => {
                let b = self.slots_of(&name.text);
                let vals = args.iter().flat_map(|a| self.expr(a)).collect();
                let ty = self.ty(e);
                let r = self.call_block(&b, vals, &ty);
                self.track(&r, &ty);
                r
            }
            ExprKind::Mark(_, x) => self.expr(x),
            // `[]` or `Map<String, Int>()` where a Map is expected.
            ExprKind::List(_) | ExprKind::New(_) if matches!(self.ty(e), Type::Map(..)) => {
                let ty = self.ty(e);
                let Type::Map(k, v) = &ty else { unreachable!() };
                let l = self.int_local();
                self.emit(Inst::ListNew(l, scalars(&pair(k, v)).len()));
                self.track(&[l], &ty);
                vec![l]
            }
            // `["a", "b"]` or `Set<String>()`: added one after the other, a value given twice is kept once.
            ExprKind::List(_) | ExprKind::New(_) if matches!(self.ty(e), Type::Set(_)) => {
                let ty = self.ty(e);
                let Type::Set(elem) = &ty else { unreachable!() };
                let l = self.int_local();
                self.emit(Inst::ListNew(l, scalars(elem).len()));
                if let ExprKind::List(elems) = &e.kind {
                    for x in elems {
                        let v = self.owned(x);
                        self.set_add(l, &v, elem);
                    }
                }
                self.track(&[l], &ty);
                vec![l]
            }
            // Written one after the other: a key given twice keeps its last value.
            ExprKind::MapLit(pairs) => {
                let ty = self.ty(e);
                let Type::Map(k, v) = &ty else { unreachable!("Map checked by the type checker") };
                let l = self.int_local();
                self.emit(Inst::ListNew(l, scalars(&pair(k, v)).len()));
                for (key, value) in pairs {
                    let (key, value) = (self.owned(key), self.owned(value));
                    let place = self.entry(l, &key, k, v);
                    let old = self.read(&place);
                    self.free_value(&old, v);
                    self.write(&place, &value);
                    self.free_value(&key, k);
                }
                self.track(&[l], &ty);
                vec![l]
            }
            ExprKind::List(_) | ExprKind::New(_) => {
                let ty = self.ty(e);
                let Type::List(elem) = &ty else { unreachable!("List checked by the type checker") };
                let l = self.int_local();
                self.emit(Inst::ListNew(l, scalars(elem).len()));
                if let ExprKind::List(elems) = &e.kind {
                    for x in elems {
                        let v = self.owned(x);
                        self.push(l, &v);
                    }
                }
                self.track(&[l], &ty);
                vec![l]
            }
            ExprKind::Index(x, i) if matches!(self.ty(x), Type::Map(..)) => {
                let Type::Map(k, v) = self.ty(x) else { unreachable!() };
                let map = self.expr1(x);
                let key = self.expr(i);
                self.lookup(map, &key, &k, &v, false)
            }
            ExprKind::Index(x, i) if i.range().is_some() => self.slice(e, x, i),
            ExprKind::Index(..) => {
                let (p, _) = self.place(e);
                self.read(&p)
            }
            ExprKind::Result => self.result.clone().expect("`result` checked by the type checker").0,
            ExprKind::Tuple(elems) => {
                let v: Slots = elems.iter().flat_map(|x| self.owned(x)).collect();
                self.track(&v, &self.ty(e));
                v
            }
            ExprKind::Field(..) => {
                let (p, _) = self.place(e);
                self.read(&p)
            }
            ExprKind::Call { name, args } => {
                if let n @ ("print" | "write") = name.text.as_str() {
                    let vals = self.expr(&args[0]);
                    let newline = n == "print";
                    match &self.given(&args[0]) {
                        ty @ (Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_)) => {
                            let ty = ty.clone();
                            let mut pieces = vec![];
                            self.value_pieces(&vals, &ty, &mut pieces);
                            let text = self.join(pieces);
                            self.emit(Inst::Print { value: text, show: Show::String, newline });
                            self.emit(Inst::Free(text));
                        }
                        ty => self.emit(Inst::Print { value: vals[0], show: Show::of(ty), newline }),
                    }
                    return vec![];
                }
                let io = match name.text.as_str() {
                    "read_file" => Some(StrOp::ReadFile),
                    "write_file" => Some(StrOp::WriteFile),
                    "append_file" => Some(StrOp::AppendFile),
                    "list_dir" => Some(StrOp::ListDir),
                    "make_dir" => Some(StrOp::MakeDir),
                    "remove_file" => Some(StrOp::RemoveFile),
                    _ => None,
                };
                if let Some(op) = io {
                    let ty = self.ty(e);
                    return self.io_call(op, args, &ty);
                }
                if name.text == "now" {
                    let (bits, x) = (self.int_local(), self.new_slots(&Type::Float)[0]);
                    self.emit(Inst::Str(bits, StrOp::Now, vec![]));
                    self.emit(Inst::Convert(x, Conversion::FromBits, bits, NOWHERE));
                    return vec![x];
                }
                if name.text == "sleep" {
                    let (s, bits, done) = (self.expr1(&args[0]), self.int_local(), self.int_local());
                    self.emit(Inst::Convert(bits, Conversion::ToBits, s, NOWHERE));
                    self.emit(Inst::Str(done, StrOp::Sleep, vec![bits]));
                    return vec![];
                }
                // `random(1..=6)`: the end made exclusive, then the range must hold a number.
                if name.text == "random" {
                    let (inclusive, a, b) = args[0].range().expect("a range checked by the type checker");
                    let (from, last) = (self.expr1(a), self.expr1(b));
                    let loc = self.loc(args[0].span);
                    let to = self.int_local();
                    if inclusive {
                        let one = self.constant(1);
                        self.emit(Inst::Binary(to, Op::Plus, last, one, loc));
                    } else {
                        self.emit(Inst::Copy(to, last));
                    }
                    let empty = self.int_local();
                    self.emit(Inst::Binary(empty, Op::Ge, from, to, NOWHERE));
                    self.emit(Inst::FaultIf(empty, Fault::EmptyRange, loc));
                    let n = self.int_local();
                    self.emit(Inst::Str(n, StrOp::Random, vec![from, to]));
                    return vec![n];
                }
                if name.text == "exists" {
                    let (p, d) = (self.expr1(&args[0]), self.int_local());
                    self.emit(Inst::Str(d, StrOp::Exists, vec![p]));
                    return vec![d];
                }
                if name.text == "args" {
                    let l = self.int_local();
                    self.emit(Inst::Str(l, StrOp::Args, vec![]));
                    self.temps.push((l, self.ty(e)));
                    return vec![l];
                }
                if name.text == "exit" {
                    let code = self.expr1(&args[0]);
                    let (low, high, (zero, max)) = (self.int_local(), self.int_local(), (self.constant(0), self.constant(255)));
                    self.emit(Inst::Binary(low, Op::Lt, code, zero, NOWHERE));
                    self.emit(Inst::Binary(high, Op::Gt, code, max, NOWHERE));
                    let loc = self.loc(args[0].span);
                    self.emit(Inst::FaultIf(low, Fault::ExitCode, loc));
                    self.emit(Inst::FaultIf(high, Fault::ExitCode, loc));
                    // The program stops: what is still alive is not freed (like a runtime error).
                    self.emit(Inst::Exit(code));
                    let next = self.new_block();
                    self.go(End::Unreachable, next);
                    return vec![];
                }
                if name.text == "read_line" {
                    let line = self.int_local();
                    self.emit(Inst::Str(line, StrOp::ReadLine, vec![]));
                    let zero = self.constant(0);
                    // Not `none`: a loop reading until `none` would stop in the middle without a word.
                    let broken = self.int_local();
                    self.emit(Inst::Binary(broken, Op::Lt, line, zero, NOWHERE));
                    let loc = self.loc(name.span);
                    self.emit(Inst::FaultIf(broken, Fault::NotText, loc));
                    let present = self.int_local();
                    self.emit(Inst::Binary(present, Op::Ne, line, zero, NOWHERE));
                    let v = vec![present, line];
                    self.track(&v, &self.ty(e));
                    return v;
                }
                let (mut vals, mut places) = (vec![], vec![]);
                self.arguments(&name.text, args, &mut vals, &mut places);
                let ty = self.ty(e);
                self.call_at(e.id, &name.text, vals, places, name.span, &ty)
            }
            ExprKind::Static { ty, name, .. } if ty.text == "Float" && name.text == "pi" => {
                let x = self.new_slots(&Type::Float)[0];
                self.emit(Inst::Const(x, std::f64::consts::PI.to_bits() as i64));
                vec![x]
            }
            ExprKind::Static { ty, name, args, .. } => {
                let q = method_name(&ty.text, &name.text);
                let (mut vals, mut places) = (vec![], vec![]);
                self.arguments(&q, args, &mut vals, &mut places);
                let result = self.ty(e);
                self.call_at(e.id, &q, vals, places, name.span, &result)
            }
            ExprKind::Unary(UnOp::Neg, x) => {
                // A number written with `-` in front is a constant (handles `-9223372036854775808`).
                match x.kind {
                    ExprKind::Int(v) if self.ty(e) == Type::Float => return vec![self.float_constant(-(v as f64))],
                    ExprKind::Int(v) => return vec![self.constant((v as i64).wrapping_neg())],
                    ExprKind::Float(v) => return vec![self.float_constant(-v)],
                    _ => {}
                }
                let a = self.expr1(x);
                if self.ty(x) == Type::BigInt {
                    let zero = self.big_text("0");
                    let dst = self.big(BinOp::Minus, zero, a, NOWHERE);
                    self.temps.push((dst, Type::BigInt));
                    return vec![dst];
                }
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
                let ty = self.ty(e);
                self.if_value(cond, then, otherwise, &ty)
            }
            ExprKind::None => {
                let v = self.new_slots(&self.ty(e));
                v.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
                v
            }
            // On `none` (or `err`), leaves the function with `none` (or the same `err`);
            // otherwise the value, owned by the expression.
            ExprKind::Try(x) => {
                let v = self.owned(x);
                let (empty, present) = (self.new_block(), self.new_block());
                self.go(End::Branch(v[0], present, empty), empty);
                let nothing = self.new_slots(&self.ret.clone());
                nothing.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
                // The error's slots come right after the tag in both results.
                let skip = match &self.ty(x) {
                    Type::Result(_, err) => {
                        let n = scalars(err).len();
                        for k in 1..=n {
                            self.emit(Inst::Copy(nothing[k], v[k]));
                        }
                        n
                    }
                    _ => 0,
                };
                self.exit(nothing);
                self.current = present;
                let value = v[1 + skip..].to_vec();
                self.track(&value, &self.ty(e));
                value
            }
            ExprKind::Block { params, body } => self.block_value_of(e, params, body),
            ExprKind::Match(m) => {
                let ty = self.ty(e);
                let v = self.new_slots(&ty);
                self.match_(m, Some((&v, &ty)));
                self.track(&v, &ty);
                v
            }
            // `ok(v)`: tag 1, the error's slots 0, then v; `err(e)`: tag 0, e, then the value's slots 0.
            ExprKind::Outcome { ok, value } => {
                let ty = self.ty(e);
                let Type::Result(t, err) = &ty else { unreachable!("result checked by the type checker") };
                let given = self.owned(value);
                let zeros = |me: &mut Self, t: &Type| -> Slots {
                    let z = me.new_slots(t);
                    z.iter().for_each(|&l| me.emit(Inst::Const(l, 0)));
                    z
                };
                let tag = self.constant(*ok as i64);
                let v: Slots = if *ok {
                    std::iter::once(tag).chain(zeros(self, err)).chain(given).collect()
                } else {
                    let z = zeros(self, t);
                    std::iter::once(tag).chain(given).chain(z).collect()
                };
                self.track(&v, &ty);
                v
            }
            ExprKind::Variant { name, args, .. } => {
                let ty = self.ty(e);
                let Type::Enum(d) = &ty else { unreachable!("enum checked by the type checker") };
                let i = d.variant(&name.text).unwrap().0;
                let fields: Slots = args.iter().flat_map(|a| self.owned(a)).collect();
                let (start, n) = variant_slice(d, i);
                let v = self.new_slots(&ty);
                // Every slot is written: in a loop, these locals still hold the previous turn's value.
                self.emit(Inst::Const(v[0], i as i64));
                for (k, &l) in v.iter().enumerate().skip(1) {
                    if (start..start + n).contains(&k) {
                        self.emit(Inst::Copy(l, fields[k - start]));
                    } else {
                        self.emit(Inst::Const(l, 0));
                    }
                }
                self.track(&v, &ty);
                v
            }
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), a, b)
                if !matches!(self.ty(a), Type::Int | Type::Float | Type::Bool | Type::Char) =>
            {
                let ty = self.ty(a);
                let (va, vb) = (self.expr(a), self.expr(b));
                let same = self.equal(&va, &vb, &ty, e.span);
                if *op == BinOp::Eq {
                    return vec![same];
                }
                let ne = self.int_local();
                self.emit(Inst::Not(ne, same));
                vec![ne]
            }
            // `a > b` = `b < a`, `a <= b` = `!(b < a)`, `a >= b` = `!(a < b)`, as in Eiffel's `COMPARABLE`.
            ExprKind::Binary(op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge), a, b)
                if matches!(self.ty(a), Type::String | Type::Struct(_) | Type::Enum(_)) =>
            {
                let ty = self.ty(a);
                let (va, vb) = (self.expr(a), self.expr(b));
                let (x, y) = if matches!(op, BinOp::Lt | BinOp::Ge) { (&va, &vb) } else { (&vb, &va) };
                let less = self.less(x, y, &ty, e.span);
                if matches!(op, BinOp::Lt | BinOp::Gt) {
                    return vec![less];
                }
                let not = self.int_local();
                self.emit(Inst::Not(not, less));
                vec![not]
            }
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                // Short circuit: `b` is computed only if `a` is not enough.
                let dst = self.int_local();
                let va = self.expr1(a);
                self.emit(Inst::Copy(dst, va));
                let (compute_b, after) = (self.new_block(), self.new_block());
                let end = if *op == BinOp::And { End::Branch(va, compute_b, after) } else { End::Branch(va, after, compute_b) };
                self.go(end, compute_b);
                self.narrow(&if *op == BinOp::And { narrowed(a, true) } else { vec![] });
                let vb = self.expr1(b);
                self.names.pop();
                self.emit(Inst::Copy(dst, vb));
                self.go(End::Jump(after), after);
                vec![dst]
            }
            ExprKind::Binary(op @ (BinOp::Plus | BinOp::Minus | BinOp::Times | BinOp::Div | BinOp::Rem), a, b) if self.ty(a) == Type::BigInt => {
                let (va, vb) = (self.expr1(a), self.expr1(b));
                let loc = self.operator(a);
                let dst = self.big(*op, va, vb, loc);
                self.temps.push((dst, Type::BigInt));
                vec![dst]
            }
            ExprKind::Binary(BinOp::Plus, a, b) if self.ty(a) == Type::String => {
                let (va, vb) = (self.expr1(a), self.expr1(b));
                let dst = self.int_local();
                self.emit(Inst::Str(dst, StrOp::Concat, vec![va, vb]));
                self.temps.push((dst, Type::String));
                vec![dst]
            }
            ExprKind::Binary(op, a, b) => {
                let (va, vb) = (self.expr1(a), self.expr1(b));
                let loc = self.operator(a);
                vec![self.operation(kir_op(*op), va, vb, self.is_float(a), loc)]
            }
            ExprKind::Method { receiver, name, .. } if self.ty(receiver).is_field(&name.text) => {
                let (p, _) = self.place(e);
                self.read(&p)
            }
            // `a.equals(b)` / `a.less(b)` on a `T: Equal` / `T: Comparable` whose real type writes
            // no such method (`Int`, a struct compared field by field): as `a = b` / `a < b`.
            ExprKind::Method { receiver, name, args }
                if matches!(self.info.types[receiver.id], Type::Param(_))
                    && matches!(name.text.as_str(), "equals" | "less")
                    && self.method(receiver, name).is_none() =>
            {
                let ty = self.ty(receiver);
                let (va, vb) = (self.expr(receiver), self.expr(&args[0]));
                if name.text == "equals" {
                    return vec![self.equal(&va, &vb, &ty, e.span)];
                }
                vec![self.less(&va, &vb, &ty, e.span)]
            }
            // `a.sound` on an `any Animal`: its dispatch picks the real type's method.
            ExprKind::Method { receiver, name, args } if matches!(self.ty(receiver), Type::Any(_)) && !self.is_clone(receiver, name) => {
                let Type::Any(interface) = self.ty(receiver) else { unreachable!() };
                let q = method_name(&interface, &name.text);
                // The box stays where it is: a `var self` method changes what is inside.
                let mut vals = self.expr(receiver);
                let mut places = vec![];
                self.arguments(&q, args, &mut vals, &mut places);
                let num = self.helper(Helper::Dispatch(interface, name.text.clone()));
                let result = self.ty(e);
                self.call_to(num, true, vals, places, name.span, &result)
            }
            ExprKind::Method { receiver, name, args } if self.method(receiver, name).is_some() => {
                let (q, r) = self.method(receiver, name).unwrap();
                let ty = self.ty(receiver);
                // `move self`: the method takes the value; otherwise it borrows it.
                // `var self`: the method gives back the changed value, written into the caller's place.
                let mut places = vec![];
                let vals = match r {
                    Mode::Move => self.owned(receiver),
                    Mode::Read => self.expr(receiver),
                    Mode::Var => {
                        let (p, _) = self.place(receiver);
                        let v = self.read(&p);
                        places.push((p, self.new_slots(&ty)));
                        v
                    }
                };
                let mut vals = vals;
                self.arguments(&q, args, &mut vals, &mut places);
                let result = self.ty(e);
                self.call_at(e.id, &q, vals, places, name.span, &result)
            }
            ExprKind::Method { receiver, name, .. }
                if name.text == "clone"
                    && matches!(self.ty(receiver), Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_)) =>
            {
                let ty = self.ty(receiver);
                let v = self.expr(receiver);
                let copy = self.clone_value(&v, &ty);
                self.track(&copy, &ty);
                copy
            }
            // `remove` changes the Map, but its slot (the address of its block) stays the same.
            ExprKind::Method { receiver, name, args } if matches!(self.ty(receiver), Type::Map(..)) => {
                let Type::Map(k, v) = self.ty(receiver) else { unreachable!() };
                let map = self.expr1(receiver);
                match name.text.as_str() {
                    "length" => {
                        let n = self.int_local();
                        self.emit(Inst::ListLength(n, map));
                        vec![n]
                    }
                    "contains" => {
                        let key = self.expr(&args[0]);
                        let i = self.find(map, &key, &k);
                        let (zero, d) = (self.constant(0), self.int_local());
                        self.emit(Inst::Binary(d, Op::Ge, i, zero, NOWHERE));
                        vec![d]
                    }
                    "remove" => {
                        let key = self.expr(&args[0]);
                        self.lookup(map, &key, &k, &v, true)
                    }
                    // A copy of each key, or of each value.
                    "keys" | "values" => {
                        let (ty, out_elem) = (self.ty(e), if name.text == "keys" { &k } else { &v });
                        let out = self.int_local();
                        self.emit(Inst::ListNew(out, scalars(out_elem).len()));
                        let nk = scalars(&k).len();
                        self.each_element(map, &pair(&k, &v), |t, _, kv| {
                            let part = if name.text == "keys" { &kv[..nk] } else { &kv[nk..] };
                            let copy = t.clone_value(part, out_elem);
                            t.push(out, &copy);
                        });
                        self.track(&[out], &ty);
                        vec![out]
                    }
                    other => unreachable!("Map method `{other}` checked by the type checker"),
                }
            }
            ExprKind::Method { receiver, name, args } if matches!(self.ty(receiver), Type::Set(_)) => {
                let Type::Set(elem) = self.ty(receiver) else { unreachable!() };
                let set = self.expr1(receiver);
                match name.text.as_str() {
                    "length" => {
                        let n = self.int_local();
                        self.emit(Inst::ListLength(n, set));
                        vec![n]
                    }
                    "add" => {
                        let x = if let ExprKind::Mark(_, x) = &args[0].kind { x } else { &args[0] };
                        let v = self.owned(x);
                        self.set_add(set, &v, &elem);
                        vec![]
                    }
                    "contains" | "remove" => {
                        let x = self.expr(&args[0]);
                        let i = self.find(set, &x, &elem);
                        let (zero, d) = (self.constant(0), self.int_local());
                        self.emit(Inst::Binary(d, Op::Ge, i, zero, NOWHERE));
                        if name.text == "remove" {
                            let (yes, done) = (self.new_block(), self.new_block());
                            self.go(End::Branch(d, yes, done), yes);
                            let old = self.load(set, i, &elem);
                            self.free_value(&old, &elem);
                            self.emit(Inst::ListRemove(set, i));
                            self.go(End::Jump(done), done);
                        }
                        vec![d]
                    }
                    other => unreachable!("Set method `{other}` checked by the type checker"),
                }
            }
            // `add` and `remove` change the List, but its slot (the address of its block) stays the same.
            ExprKind::Method { receiver, name, args } if matches!(self.ty(receiver), Type::List(_)) => {
                let Type::List(elem) = self.ty(receiver) else { unreachable!() };
                let list = self.expr1(receiver);
                match name.text.as_str() {
                    "each" | "filter" | "map" | "any" | "all" | "fold" => self.block_method(e, list, &elem, &name.text, args),
                    "sorted" => self.sorted(list, &elem, args.last(), e.span),
                    "join" => {
                        let (sep, text) = (self.expr1(&args[0]), self.int_local());
                        self.emit(Inst::Str(text, StrOp::Join, vec![list, sep]));
                        self.temps.push((text, Type::String));
                        vec![text]
                    }
                    "length" => {
                        let n = self.int_local();
                        self.emit(Inst::ListLength(n, list));
                        vec![n]
                    }
                    "add" => {
                        let x = if let ExprKind::Mark(_, x) = &args[0].kind { x } else { &args[0] };
                        let v = self.owned(x);
                        self.push(list, &v);
                        vec![]
                    }
                    "remove" => {
                        let index = self.expr1(&args[0]);
                        let loc = self.loc(args[0].span);
                        self.emit(Inst::ListCheck(list, index, loc));
                        let v = self.load(list, index, &elem);
                        self.emit(Inst::ListRemove(list, index));
                        self.track(&v, &elem);
                        v
                    }
                    other => unreachable!("List method `{other}` checked by the type checker"),
                }
            }
            ExprKind::Method { receiver, name, args } => {
                let v = self.expr1(receiver);
                let string = |op| match op {
                    "length" => StrOp::Length,
                    _ => StrOp::Clone,
                };
                if self.ty(receiver) == Type::String
                    && let Some(op) = string_op(&name.text)
                {
                    let mut vals = vec![v];
                    vals.extend(args.iter().map(|a| self.expr1(a)));
                    if matches!(op, StrOp::Split | StrOp::Replace) {
                        let (n, zero, empty) = (self.int_local(), self.constant(0), self.int_local());
                        self.emit(Inst::Str(n, StrOp::Length, vec![vals[1]]));
                        self.emit(Inst::Binary(empty, Op::Eq, n, zero, NOWHERE));
                        let loc = self.loc(args[0].span);
                        self.emit(Inst::FaultIf(empty, Fault::EmptyText, loc));
                    }
                    let dst = self.int_local();
                    self.emit(Inst::Str(dst, op, vals));
                    match self.ty(e) {
                        Type::Bool => {}
                        ty => self.temps.push((dst, ty)),
                    }
                    return vec![dst];
                }
                // `s.find(part)`: `none` if it is not there.
                if self.ty(receiver) == Type::String && name.text == "find" {
                    let part = self.expr1(&args[0]);
                    let (at, zero, present, value) = (self.int_local(), self.constant(0), self.int_local(), self.int_local());
                    self.emit(Inst::Str(at, StrOp::Find, vec![v, part]));
                    self.emit(Inst::Binary(present, Op::Ge, at, zero, NOWHERE));
                    self.emit(Inst::Binary(value, Op::Times, at, present, NOWHERE));
                    return vec![present, value];
                }
                if self.ty(receiver) == Type::String && name.text == "to_float" {
                    let (present, bits) = (self.int_local(), self.int_local());
                    self.emit(Inst::Str(present, StrOp::IsFloat, vec![v]));
                    self.emit(Inst::Str(bits, StrOp::ToFloat, vec![v]));
                    let x = self.new_slots(&Type::Float)[0];
                    self.emit(Inst::Convert(x, Conversion::FromBits, bits, NOWHERE));
                    return vec![present, x];
                }
                // A `BigInt` is its decimal text: read like a String's, `none` if too big.
                if matches!(self.ty(receiver), Type::String | Type::BigInt) && name.text == "to_int" {
                    let (present, n) = (self.int_local(), self.int_local());
                    self.emit(Inst::Str(present, StrOp::IsInt, vec![v]));
                    self.emit(Inst::Str(n, StrOp::ToInt, vec![v]));
                    return vec![present, n];
                }
                // A Char is a Unicode code: 0 to 0x10FFFF, without the surrogates 0xD800 to 0xDFFF.
                if self.ty(receiver) == Type::Int && name.text == "to_char" {
                    let test = |t: &mut Self, op, bound| {
                        let (b, d) = (t.constant(bound), t.int_local());
                        t.emit(Inst::Binary(d, op, v, b, NOWHERE));
                        d
                    };
                    let (low, high, s1, s2) = (test(self, Op::Ge, 0), test(self, Op::Le, 0x10FFFF), test(self, Op::Ge, 0xD800), test(self, Op::Le, 0xDFFF));
                    let (surrogate, not_surrogate, present, code) = (self.int_local(), self.int_local(), self.int_local(), self.int_local());
                    self.emit(Inst::Binary(surrogate, Op::Times, s1, s2, NOWHERE));
                    self.emit(Inst::Not(not_surrogate, surrogate));
                    self.emit(Inst::Binary(present, Op::Times, low, high, NOWHERE));
                    self.emit(Inst::Binary(present, Op::Times, present, not_surrogate, NOWHERE));
                    self.emit(Inst::Binary(code, Op::Times, v, present, NOWHERE));
                    return vec![present, code];
                }
                if self.ty(receiver) == Type::String {
                    let (dst, op) = (self.int_local(), string(name.text.as_str()));
                    self.emit(Inst::Str(dst, op, vec![v]));
                    if op == StrOp::Clone {
                        self.temps.push((dst, Type::String));
                    }
                    return vec![dst];
                }
                if self.ty(receiver) == Type::BigInt && name.text == "clone" {
                    let copy = self.int_local();
                    self.emit(Inst::Str(copy, StrOp::Clone, vec![v]));
                    self.temps.push((copy, Type::BigInt));
                    return vec![copy];
                }
                if self.ty(receiver) == Type::Int && name.text == "to_big" {
                    let digits = self.int_local();
                    self.emit(Inst::Str(digits, StrOp::Format(Show::Int), vec![v]));
                    self.temps.push((digits, Type::BigInt));
                    return vec![digits];
                }
                if name.text == "clone" {
                    let copy = self.new_slots(&self.ty(receiver))[0];
                    self.emit(Inst::Copy(copy, v));
                    return vec![copy];
                }
                if name.text == "to_string" {
                    let text = self.int_local();
                    self.emit(Inst::Str(text, StrOp::Format(Show::of(&self.ty(receiver))), vec![v]));
                    self.temps.push((text, Type::String));
                    return vec![text];
                }
                if crate::types::number_method(&self.ty(receiver), &name.text).is_some() {
                    return self.number_method(e, v, &name.text, args);
                }
                let conversion = match (&self.ty(receiver), name.text.as_str()) {
                    (Type::Int, "to_float") => Conversion::IntToFloat,
                    (Type::Float, "to_int") => Conversion::FloatToInt,
                    (Type::Char, "to_int") => return vec![v], // a Char already is its code
                    other => unreachable!("method checked by the type checker: {other:?}"),
                };
                let dst = self.new_slots(&self.ty(e))[0];
                let loc = self.loc(name.span);
                self.emit(Inst::Convert(dst, conversion, v, loc));
                vec![dst]
            }
        }
    }
}

/// The digits of a number written directly (`12`, `-3`, `123456789012345678901`), for a `BigInt`.
fn big_literal(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Int(v) => Some(v.to_string()),
        ExprKind::BigInt(d) => Some(d.clone()),
        ExprKind::Unary(UnOp::Neg, x) => big_literal(x).map(|d| format!("-{d}")),
        _ => None,
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
        ExprKind::Call { args, .. } | ExprKind::Static { args, .. } | ExprKind::Variant { args, .. } | ExprKind::Tuple(args) => {
            args.iter().for_each(|a| names_in(a, names))
        }
        ExprKind::Match(m) => names_in(&m.subject, names),
        ExprKind::Try(x) | ExprKind::Outcome { value: x, .. } => names_in(x, names),
        ExprKind::Method { receiver, args, .. } => {
            names_in(receiver, names);
            args.iter().for_each(|a| names_in(a, names));
        }
        ExprKind::Unary(_, x) | ExprKind::Field(x, _) | ExprKind::Mark(_, x) => names_in(x, names),
        ExprKind::List(elems) => elems.iter().for_each(|a| names_in(a, names)),
        ExprKind::MapLit(pairs) => pairs.iter().for_each(|(k, v)| {
            names_in(k, names);
            names_in(v, names);
        }),
        ExprKind::Index(x, i) => {
            names_in(x, names);
            names_in(i, names);
        }
        ExprKind::New(_) => {}
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
        ExprKind::Int(_) | ExprKind::BigInt(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::InMode { .. } | ExprKind::None => {}
        // ponytail: what a block uses is not shown in a contract's `values:` line.
        ExprKind::Block { .. } => {}
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
