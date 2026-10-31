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

use crate::ast::{self, BinOp, Block, Case, Contract, ContractKind, Expr, ExprKind, Match, OnlyIn, Otherwise, Part, Pattern, Mode, Step, Stmt, Target, UnOp};
use crate::diag::{Span, line_col};
use crate::locale;
pub use crate::shared::{case_of, float_text, parse_float, parse_int};
use crate::types::{CType, c_layout, changes, changes_in, EnumDef, Info, Type, enum_view, is_copy, method_name, narrowed, place_root, substitute, type_of};

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
        Type::Struct(d) => d.fields().iter().map(|(_, t)| laid_out(&d.name, t)).collect(),
        _ => unreachable!("tuple or struct checked by the type checker"),
    }
}

/// A field holding its owner again (`next: Cell?` in `Cell`) is laid out in a box (`Type::Box`):
/// otherwise a `Cell` would need the slots of a `Cell` plus more.
fn laid_out(owner: &str, t: &Type) -> Type {
    if crate::types::holds(t, owner, &mut vec![]) { Type::Box(Box::new(t.clone())) } else { t.clone() }
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
    let size = |fs: &[(String, Type)]| fs.iter().map(|(_, t)| scalars(&laid_out(&d.name, t)).len()).sum::<usize>();
    let start = 1 + d.variants()[..i].iter().map(|(_, fs)| size(fs)).sum::<usize>();
    (start, size(&d.variants()[i].1))
}

/// The fields of an enum's variants, all of them, in the order of their slots.
fn variant_fields(d: &EnumDef) -> impl Iterator<Item = Type> + '_ {
    d.variants().iter().flat_map(|(_, fs)| fs.iter().map(|(_, t)| laid_out(&d.name, t)))
}

/// The slots needed for a value of this type, in order.
pub fn scalars(t: &Type) -> Vec<Scalar> {
    match t {
        Type::Int | Type::Bool | Type::Char | Type::Date | Type::DateTime | Type::Pointer | Type::String | Type::BigInt | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_) | Type::Box(_) => vec![Scalar::Int],
        // The function's number, then the box of what the block uses around it.
        Type::Block(..) => vec![Scalar::Int, Scalar::Int],
        Type::Float => vec![Scalar::Float],
        Type::Tuple(ts) => ts.iter().flat_map(scalars).collect(),
        Type::Struct(_) => members(t).iter().flat_map(scalars).collect(),
        // ponytail: no slot is shared between variants (a Float and a String never meet in one),
        // so a value is wider than its largest variant; slots of the other variants hold 0.
        Type::Enum(d) => std::iter::once(Scalar::Int).chain(variant_fields(d).flat_map(|t| scalars(&t))).collect(),
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
        Type::Enum(d) => std::iter::once(None).chain(variant_fields(d).flat_map(|t| heap(&t))).collect(),
        // Its one element is the value: freed and copied like a List of one.
        Type::Box(t) => vec![Some(Type::List(t.clone()))],
        Type::Optional(t) => std::iter::once(None).chain(heap(t)).collect(),
        Type::Result(ok, err) => std::iter::once(None).chain(heap(err)).chain(heap(ok)).collect(),
        Type::Int | Type::Float | Type::Bool | Type::Char | Type::Date | Type::DateTime | Type::Pointer | Type::Unit | Type::Error | Type::Param(_) | Type::Block(..) => {
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
    /// Takes a value: returns its JSON text (`json_writer`).
    ToJson(Type),
    /// Takes a JSON text's nodes and the number of one: returns a status, a path and the value
    /// read from it (`json_reader`).
    FromJson(Type),
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
    /// The functions of the `extern` blocks, in the order of `extern_order` (`Inst::Extern`).
    pub externs: Vec<crate::types::ExternFn>,
    /// The fields' C types of each C struct (`Info::c_structs`).
    pub c_structs: Vec<Vec<CType>>,
    /// `header { icon := "…" }` (filled by `main`).
    pub icon: Option<crate::icon::Icon>,
}

/// The `extern` functions, numbered (`Inst::Extern`).
fn extern_order(info: &Info) -> Vec<&String> {
    let mut names: Vec<&String> = info.externs.keys().collect();
    names.sort();
    names
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
    /// `dst := op(x, y)` (`y` only for `pow`): `shared::math`, the same in run and build.
    Math(Local, Math, Local, Local),
    Convert(Local, Conversion, Local, Loc),
    /// `results := f(args)`.
    Call(Slots, usize, Slots),
    /// `results := extern function k (Program::externs)(args)`, one slot per C value; then one
    /// result per `var` parameter, what C left at its address. The location: for a library or a
    /// function not found.
    Extern(Slots, usize, Slots, Loc),
    /// `dst := function f`, to call it later with `CallIndirect` (a block).
    FuncRef(Local, usize),
    /// `dst := the address of a function with these C parameters and result`, made by codegen
    /// around function `f` (its trampoline), for C to call back (PLAN Part 4 "`extern`, step 2").
    Callback(Local, usize, Vec<CType>, CType),
    /// `results := (the function in this local)(args)`: calls a block.
    CallIndirect(Slots, Local, Slots),
    /// `dst := "text"`: a new String.
    Text(Local, String),
    /// `dst := op(args)`, done by the runtime (`kova_str_*` in `runtime/kova.rs`).
    Str(Local, StrOp, Slots),
    /// Frees a String, then sets its slot to 0 (freeing 0 does nothing).
    Free(Local),
    /// `dst := a new empty List` whose elements take `width` slots (`kova_list_*` in `runtime/kova.rs`).
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
    /// `files.copy(a) to(b)`, `files.move(a) to(b)`: 0, or minus the number of the failure, plus
    /// 16 when it is `b`'s (`io_result` then names `b`).
    CopyFile,
    MoveFile,
    /// `files.size(path)`: the bytes, or minus the number of the failure.
    FileSize,
    /// `files.modified(path)`: a List of one Int, the local time in seconds since 1970 (it may be
    /// below 0), or minus the number of the failure.
    Modified,
    /// `files.is_folder(path)`: 1 or 0.
    IsFolder,
    /// `files.matching(pattern)` (`shared::matching`): a new `List<String>` or minus the number of
    /// the failure.
    Matching,
    /// `system.env(name)`: a new String, or 0 if the variable is not there.
    Env,
    /// `system.run(program) args(words)` (the words: a `List<String>`, or 0 for none): a List of
    /// one `system.Finished` (output, errors, code), or minus the `RunError` variant + 1.
    Run,
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
    /// `DateTime.now`: the local time, in seconds since 1970-01-01 00:00 (`shared::local_offset`).
    LocalNow,
    /// `s.to_date` / `s.to_datetime` (`shared::parse_date`): the days / seconds, `i64::MIN` if not a date.
    ToDate,
    ToDateTime,
    /// `read_line`: a new String, 0 at the end of the input, -1 if the line is not UTF-8.
    ReadLine,
    /// `get` / `post`: (address, method, body, follow the redirections: 1 or 0) → a List of one
    /// `Response` (status, url, body, headers), or minus the `WebError` variant + 1.
    Web,
    /// `T.from_json(text)` (src/json.rs): a List of nodes, 4 slots each (`json::slots`), or minus
    /// the line where the text stops being JSON.
    JsonParse,
    /// A String as JSON writes it, between quotes: a new String.
    JsonQuote,
    /// `serve(port)` (PLAN Part 4 "The web server", src/http.rs): what listens on 127.0.0.1 (on
    /// every address if the second argument is 1: `serve_network`), or -1 if the port is taken or
    /// not allowed. The third is the folder of `serve_files`, or 0: what it writes when it starts
    /// names it. The fourth is what it writes (`types::log_mode`: 0 all, 1 the start, 2 nothing).
    Listen,
    /// The next question that can be read: a List of one element, a `Request`'s 6 slots then the
    /// connection.
    Accept,
    /// (connection, status, headers, body, file): sends the `Answer` and closes; gives 0.
    Reply,
    /// `q.path`, `q.query(name)` (a String or 0), `Answer.status(404)`'s text: new Strings.
    UrlPath,
    UrlQuery,
    StatusText,
    /// `Answer.file(path)`: the file to send (a new String), or 0 for a 404 (`http::file_to_serve`).
    FileToServe,
    /// `serve(…) workers(n)`, built only (PLAN Part 4 "Several questions at once"): (n, a
    /// function taking one box, the box) runs the function on `n` threads, this one included.
    /// Never ends.
    Workers,
    /// `l.map workers(n) { … }`, built only (PLAN Part 4 "Several things at once"): (n, the
    /// List's length, a function taking a box and a counter, the box) runs the function on up to
    /// `n` threads, this one included, then waits for them.
    Parallel,
    /// In that function: the next element to do (counted from 0), from the counter.
    Next,
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
    /// `s.wrapped(width)`: a new String (`shared::wrapped`).
    Wrap,
    /// `s.matches(pattern)`: 1 or 0 (`src/regex.rs`; the pattern was checked by the checker).
    Matches,
    /// `s.find_all(pattern)`: a new `List<String>`.
    FindAll,
    /// `s.replace_all(pattern) with(new)`: a new String.
    ReplaceAll,
    /// `s.groups(pattern)`: a new `List<String>`, or 0 if nothing matches.
    Groups,
    /// `List<Book>.from_csv(text)` (src/csv.rs): (text, the fields as `csv_spec` writes them) → a
    /// new List, or a `csv::Problem::code` below 0.
    CsvRead,
    /// `books.to_csv`: (the List, `csv_spec`, the separator) → a new String.
    CsvWrite,
    /// `l.mean`, `l.median` of a List that is not empty: (the List, 1 for Floats) → a Float's bits.
    Mean,
    Median,
    Lines,
    Fixed,
    RoundTo,
    /// `secure.random(from..to)`, `to` excluded and above `from` (checked before).
    SecureRandom,
    /// `secure.token(n)`, `n` ≥ 0 (checked before): a new String of `2n` hexadecimal digits.
    SecureToken,
    /// `secure.uuid`: a new String.
    Uuid,
    /// PLAN Part 4 "The library after Python, step 4" (src/crypto.rs). `s.sha256`,
    /// `s.hmac_sha256(key)`, `s.to_base64`, `s.to_base64url`: a new String.
    Sha256,
    Hmac,
    ToBase64,
    ToBase64Url,
    /// `String.from_base64(code)`, `String.from_base64url(code)`: a new String, or 0.
    FromBase64,
    FromBase64Url,
    /// `files.sha256(path)`: a new String, or minus the `IoError` variant + 1.
    FileSha256,
    /// `secure.key`: 32 bytes of chance in base64, a new String.
    SecureKey,
    /// `secure.encrypt(text) key(k)`: a new String, or 0 if `k` is no key.
    Encrypt,
    /// `secure.decrypt(code) key(k)`: a new String, or 0.
    Decrypt,
    /// `secure.password_hash(password) memory(mb) passes(n)`, checked before: a new String.
    PasswordHash,
    /// `secure.password_matches(password) hash(h)`: 1 or 0.
    PasswordMatches,
    /// `s.red`: a new String in the style numbered by the second slot (`shared::PAINTS`), plain
    /// when the output is not a terminal (`shared::colors`).
    Paint,
    /// `log.info(text)`: the level (`shared::LEVELS`), then the text.
    Log,
    /// `log.level(l)`: the lowest level shown, unless `KOVA_LOG` names one.
    LogLevel,
    /// For an `extern` call: a copy of a String ended by a 0 byte, given to C (freed by
    /// `CFree`); the second slot says whether there is one (else 0); -1 if it holds a 0 byte.
    CText,
    CFree,
    /// The String a C function gave (copied; NULL gives 0).
    FromCText,
    /// `CText` for a `WideString` (PLAN Part 4 "`extern`, step 4"): UTF-16, freed by `CWideFree`.
    CWideText,
    CWideFree,
    /// The String a C function gave as UTF-16 (copied; NULL gives 0).
    FromCWideText,
    /// A leaf of a C struct: (address, offset, 1 for a 4-byte `int` else 8 bytes); 0 at NULL.
    CRead,
    /// `ui.typed`: the text of the field that changed last (src/ui.rs).
    UiTyped,
    /// `ui.draw(op)`: one more thing on the canvas being drawn; `ui.drawn` takes all of it.
    UiDraw,
    UiDrawn,
    /// `ui.act(n)`: what the window must do at the next `ui.show`.
    UiAct,
    /// `ui.place(…) size(…)`: where the window must go at the next `ui.show`.
    UiPlace,
    /// `ui.room(0 | 1)`: the width or height of the `ui.fill` canvas being made; `ui.room(2)`:
    /// a new view starts, from the first canvas.
    UiRoom,
    /// `ui.show(title, view, width, height)`: the button clicked, -1 once the window is closed (src/ui.rs).
    UiShow,
    /// Only made by codegen.rs for an `Inst::Extern`: the address of the function (its number,
    /// library, C name, line, column), found at its first call.
    ExternAddress,
}

/// What src/csv.rs reads or writes for a List of `elem`: `*` for rows (`List<String>`), else
/// each field's kind and name (`s:title,?i:pages`).
fn csv_spec(elem: &Type) -> String {
    let Type::Struct(d) = elem else { return "*".into() };
    let fields: Vec<String> = d.fields().iter().map(|(name, t)| {
        let kind = crate::types::csv_kind(t).expect("checked by the type checker");
        format!("{}{}:{name}", if kind.len() > 1 { "?" } else { "" }, &kind[kind.len() - 1..])
    }).collect();
    fields.join(",")
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
        "lines" => StrOp::Lines,
        "chars" => StrOp::Chars,
        "to_upper" => StrOp::ToUpper,
        "to_lower" => StrOp::ToLower,
        "repeat" => StrOp::Repeat,
        "wrapped" => StrOp::Wrap,
        "matches" => StrOp::Matches,
        "find_all" => StrOp::FindAll,
        "replace_all(…) with(…)" => StrOp::ReplaceAll,
        "sha256" => StrOp::Sha256,
        "hmac_sha256" => StrOp::Hmac,
        "to_base64" => StrOp::ToBase64,
        "to_base64url" => StrOp::ToBase64Url,
        n if crate::shared::PAINTS.iter().any(|(p, _)| *p == n) => StrOp::Paint,
        _ => return None,
    })
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

/// How to show a slot. Same order as in `runtime/kova.rs`.
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

/// The maths Rust's `core` doesn't have, numbered as `shared::math` reads them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Math {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Ln,
    Log10,
    Exp,
    Pow,
}

/// The time limit of a `get`, a `post` or a new `Request` (PLAN Part 4 "The web, Request").
const DEFAULT_SECONDS: f64 = 30.0;

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
    /// `Date.year(2026) month(2) day(30)`.
    BadDate,
    /// `DateTime.year(2026) month(10) day(23) hour(25) minute(0)`.
    BadTime,
    /// `serve(8080)` on a port taken, or not allowed.
    CannotServe,
    /// `serve(8080) workers(0)`.
    BadWorkers,
    /// `[].mean`, `[].median`.
    NoElements,
    /// `secure.token(-1)`.
    NegativeSize,
    /// `secure.encrypt(text) key("abc")`: not a key made by `secure.key`.
    BadKey,
    /// `secure.password_hash(p) memory(2)`: 8 to 1024 MB, 1 to 16 passes.
    PasswordSettings,
    /// An `Int` given to C as an `Int32` that does not fit.
    Int32Range,
    /// A `String` given to C holding a 0 byte.
    TextWithZero,
    /// A C function declared `-> String` or `-> Pointer` gave NULL.
    NullText,
    NullPointer,
    /// An `extern` call under `kova test` or `kova debug`, which interpret.
    ExternInterpreted,
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
            Fault::BadDate => "fault_bad_date",
            Fault::BadTime => "fault_bad_time",
            Fault::CannotServe => "fault_cannot_serve",
            Fault::BadWorkers => "fault_workers",
            Fault::NoElements => "fault_no_elements",
            Fault::NegativeSize => "fault_negative_size",
            Fault::Int32Range => "fault_int32",
            Fault::TextWithZero => "fault_text_zero",
            Fault::NullText => "fault_null_text",
            Fault::NullPointer => "fault_null_pointer",
            Fault::ExternInterpreted => "fault_extern_interpreted",
            Fault::BadKey => "fault_bad_key",
            Fault::PasswordSettings => "fault_password_settings",
        }
    }
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
    // Then `mode build else …` (PLAN 2.14), then the contracts a method takes from its
    // interfaces, with the names of the interface's parameters → the method's.
    type Source<'f> = (
        String,
        Option<(Mode, Type)>,
        &'f [ast::Param],
        Option<&'f ast::TypeAst>,
        &'f [Contract],
        &'f Block,
        Vec<(String, Type)>,
        Option<&'f OnlyIn>,
        Vec<(&'f [Contract], Vec<(String, String)>)>,
    );
    let free = |g: &'f ast::Function| -> Source<'f> {
        (g.name.text.clone(), None, &g.params[..], g.ret.as_ref(), &g.contracts[..], &g.body, vec![], g.only.as_ref(), vec![])
    };
    // `struct Bank : Account` writing `take`: the `require`s and `ensure`s of `Account.take` too.
    let inherited = |owner: &str, m: &'f ast::Function| -> Vec<(&'f [Contract], Vec<(String, String)>)> {
        let Some(s) = f.structs.iter().find(|s| s.name.text == owner) else { return vec![] };
        (s.interfaces.iter())
            .filter_map(|n| f.interfaces.iter().find(|i| i.name.text == n.text))
            .flat_map(|i| i.methods.iter().map(|(w, _)| w))
            .filter(|w| w.name.text == m.name.text && !w.contracts.is_empty() && !std::ptr::eq(*w, m))
            .map(|w| (&w.contracts[..], w.params.iter().zip(&m.params).map(|(a, b)| (a.name.text.clone(), b.name.text.clone())).collect()))
            .collect()
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
        (name, m.receiver.map(|r| (r, ty.clone())), &m.params, m.ret.as_ref(), &m.contracts, &m.body, me, m.only.as_ref(), inherited(owner, m))
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
    // One copy of a default method for each type taking it: `Dog.introduce`, `Cat.introduce`
    // (for `Kennel<T>`, one per list of types, like its own methods).
    for (owner, k, j) in &info.defaults {
        let m = method(&info.structs[owner], owner, &f.interfaces[*k].methods[*j].0);
        match f.structs.iter().find(|s| s.name.text == *owner).filter(|s| !s.generics.is_empty()) {
            Some(s) => {
                generic.insert(m.0.clone(), (m, s.generics.iter().map(|g| g.name.text.clone()).collect()));
            }
            None => all.push(m),
        }
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
    for e in &f.externs {
        for (func, ..) in &e.functions {
            signatures.insert(func.name.text.clone(), (None, func.params.iter().map(|p| p.mode).collect()));
        }
    }
    let n = all.len();
    all.extend(f.tests.iter().filter(|_| tests).map(|t| (t.name.clone(), None, &[][..], None, &[][..], &t.body, vec![], None, vec![])));
    let numbers: HashMap<&str, usize> = all[..n].iter().enumerate().map(|(i, g)| (g.0.as_str(), i)).collect();
    // A `mode build` function under `kova run`: its `else` replaces its body and its contracts.
    fn away(only: Option<&OnlyIn>, built: bool) -> Option<&OnlyIn> {
        only.filter(|m| m.build != built)
    }
    // The functions that receive the call location (`main` is called by the runtime).
    let site = |g: &Source| {
        let stops = matches!(away(g.7, built), Some(OnlyIn { otherwise: Otherwise::Stop(_), .. }));
        let any = !g.4.is_empty() || g.8.iter().any(|(c, _)| !c.is_empty());
        g.0 != "main" && (stops || contracts && any && away(g.7, built).is_none())
    };
    let sites: Vec<bool> = all.iter().map(site).collect();
    let generic_sites: HashMap<&str, bool> = generic.iter().map(|(n, (g, _))| (n.as_str(), site(g))).collect();
    let helpers = RefCell::new(vec![]);
    let made = RefCell::new(vec![]);
    let translator = |ret: Type, ensures: Vec<(&'f Contract, Option<Vars>)>, subst: Vec<(String, Type)>| Translator {
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
        writes: vec![],
        temps: vec![],
        envs: vec![],
        env_marks: vec![],
        reopen: None,
    };
    let make = |(name, receiver, params, ret, written_contracts, body, subst, only, from): &Source<'f>, with_site: bool| {
            let mut structs = info.structs.clone();
            let subst = subst.clone();
            structs.extend(subst.iter().cloned());
            let ret = ret.map_or(Type::Unit, |t| type_of(t, &structs));
            let away = away(*only, built);
            let kept = move |k: ContractKind| written_contracts.iter().filter(move |c: &&Contract| contracts && away.is_none() && c.kind == k);
            let mut t = translator(ret.clone(), kept(ContractKind::Ensure).map(|c| (c, None)).collect(), subst);
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
            // An interface's contracts read its own parameter names: those names, on top of every
            // other, give the method's parameters.
            for (written, names) in from.iter().filter(|_| contracts && away.is_none()) {
                let mut vars: Vars = names.iter().map(|(i, m)| (i.clone(), t.variable(m))).collect();
                if receiver.is_some() {
                    vars.insert("self".into(), t.variable("self"));
                }
                for c in written.iter() {
                    match c.kind {
                        ContractKind::Require => {
                            t.names.push(Scope { borrowed: true, vars: vars.clone() });
                            t.check(c, t.site);
                            t.names.pop();
                        }
                        _ => t.ensures.push((c, Some(vars.clone()))),
                    }
                }
            }
            match away.map(|m| (m.span, &m.otherwise)) {
                Some((span, Otherwise::Stop(parts))) => t.stop(span, parts, t.site),
                Some((_, Otherwise::Skip)) => {}
                Some((_, Otherwise::Return(e))) => {
                    let v = t.owned(e);
                    t.exit(v);
                }
                None | Some((_, Otherwise::Discreet)) => t.block(body),
            }
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
                make(&(shown, receiver, g.2, g.3, g.4, g.5, subst, g.7, g.8.clone()), generic_sites[name.as_str()])
            }
            Helper::Block(k) => made.borrow_mut()[*k].take().expect("each block is made once"),
            h => translator(Type::Unit, vec![], vec![]).helper_function(h),
        });
        k += 1;
    }
    let tests = all[n..].iter().enumerate().map(|(i, t)| (t.0.clone(), n + i)).collect();
    let externs = extern_order(info).into_iter().map(|n| info.externs[n].clone()).collect();
    Program { functions, main: numbers.get("main").copied(), tests, externs, c_structs: info.c_structs.clone(), icon: None }
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
    /// With the names an interface's contract reads (`None`: the function's own).
    ensures: Vec<(&'a Contract, Option<Vars>)>,
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
    /// For each open `for x in var l` (and case of `match var`): the number of loops open around
    /// its body, where its value lives and the slots that hold it meanwhile, written back at the
    /// end of each turn (of the case), and before a `break` or a `return`.
    writes: Vec<(usize, Place, Slots)>,
    /// The Strings and Lists made by the current statement and not given to anyone yet
    /// (with what they are, see `heap`): freed at the end of the statement.
    temps: Vec<(Local, Type)>,
    /// The boxes of the blocks given to the call being prepared, with the slots of what each one
    /// holds: given back to those variables after the call (copy-in / copy-out, like `var`).
    envs: Vec<(Local, Vec<Slots>)>,
    /// Where the boxes of each call whose arguments are ready start in `envs` (see `call_to`).
    env_marks: Vec<usize>,
    /// A guarded `var` given its optional back after the value of `x := …` (`statements`).
    reopen: Option<Vars>,
}

/// The names of one open block.
#[derive(Default)]
struct Scope {
    /// Names that borrow their slots (borrowed parameters, `for` elements…): never freed.
    borrowed: bool,
    vars: Vars,
}

type Vars = HashMap<String, (Slots, Type)>;

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
            let same = if let Type::List(elem) = ty { self.equal_lists(a[0], b[0], elem) } else { self.equal_variants(&a, &b, ty) };
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
        if let Helper::ToJson(ty) = h {
            return self.json_writer(ty);
        }
        if let Helper::FromJson(ty) = h {
            return self.json_reader(ty);
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
                // Each element's text goes into a List joined once: appending to the text made so
                // far would copy it at each element.
                let parts = self.int_local();
                self.emit(Inst::ListNew(parts, 1));
                self.each_element(list, elem, |t, _, v| {
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
                    t.push(parts, &[piece]);
                    t.free_temps(mark);
                });
                let (sep, joined) = (self.text(", "), self.int_local());
                self.emit(Inst::Str(joined, StrOp::Join, vec![parts, sep]));
                self.emit(Inst::Free(sep));
                self.free_value(&[parts], &Type::List(Box::new(Type::String)));
                // A Set is shown as it is written: `Set["a", "b"]`.
                let text = self.text(if matches!(ty, Type::Set(_)) { "Set[" } else { "[" });
                self.append(text, joined);
                let close = self.text("]");
                self.append(text, close);
                ("show", vec![text])
            }
            Helper::Equal(_) | Helper::Instance(..) | Helper::Dispatch(..) | Helper::Look(_) | Helper::Block(_) | Helper::Find(_) | Helper::ToJson(_) | Helper::FromJson(_) => {
                unreachable!("made above")
            }
        };
        self.finish(End::Return(result.clone()));
        let returns = result.iter().map(|_| Scalar::Int).collect();
        self.function(format!("{name} {}", ty.text()), 1, returns)
    }

    /// An `Answer` (`types::answer`) owning `body` and `file`, with a `content-type` if given.
    fn answer_made(&mut self, status: Local, content_type: Option<&str>, body: Local, file: Local) -> Slots {
        let headers = self.int_local();
        self.emit(Inst::ListNew(headers, 2));
        if let Some(t) = content_type {
            let (name, value) = (self.text("content-type"), self.text(t));
            self.push(headers, &[name, value]);
        }
        vec![status, headers, body, file]
    }

    /// `Answer.file(path)`: the file (its `content-type` found when it is sent), or a 404.
    fn file_answer(&mut self, path: Local) -> Slots {
        let (file, zero, found) = (self.int_local(), self.constant(0), self.int_local());
        self.emit(Inst::Str(file, StrOp::FileToServe, vec![path]));
        self.emit(Inst::Binary(found, Op::Ne, file, zero, NOWHERE));
        let v = self.new_slots(&crate::types::answer());
        let (yes, no, done) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Branch(found, yes, no), yes);
        let (status, body) = (self.constant(200), self.text(""));
        let a = self.answer_made(status, None, body, file);
        self.copy(&v, &a);
        self.go(End::Jump(done), no);
        let (status, body, empty) = (self.constant(404), self.int_local(), self.text(""));
        self.emit(Inst::Str(body, StrOp::StatusText, vec![status]));
        let a = self.answer_made(status, Some("text/plain; charset=utf-8"), body, empty);
        self.copy(&v, &a);
        self.go(End::Jump(done), done);
        v
    }

    /// `serve(port) { q: Request => … }` (PLAN Part 4 "The web server"): listens, then answers each
    /// question with the block, one after the other, until the program stops (`exit`). Without a
    /// block, `serve_files(folder) at(port)`: each answer is `Answer.file(folder + q.path)`. With
    /// `everyone`, the other computers can ask too (`serve_network`). With `workers`, built, `n`
    /// threads answer (`workers`); `kova run` still answers one question at a time: the same
    /// answers, since the block cannot change what is around it (`ownership.rs` `workers`).
    #[allow(clippy::too_many_arguments)]
    fn serve(&mut self, port: Local, log: Local, everyone: bool, block: Option<&Expr>, folder: Option<Local>, workers: Option<Local>, loc: Loc) -> Slots {
        if let Some(n) = workers {
            let (one, bad) = (self.constant(1), self.int_local());
            self.emit(Inst::Binary(bad, Op::Lt, n, one, NOWHERE));
            self.emit(Inst::FaultIf(bad, Fault::BadWorkers, loc));
        }
        let given = block.filter(|b| !matches!(b.kind, ExprKind::Block { .. })).map(|b| self.expr(b));
        let (listener, zero, taken) = (self.int_local(), self.constant(0), self.int_local());
        let (open, named) = (self.constant(everyone as i64), folder.unwrap_or(zero));
        self.emit(Inst::Str(listener, StrOp::Listen, vec![port, open, named, log]));
        self.emit(Inst::Binary(taken, Op::Lt, listener, zero, NOWHERE));
        self.emit(Inst::FaultIf(taken, Fault::CannotServe, loc));
        match (workers, block) {
            (Some(n), Some(block)) if self.built => self.workers(listener, n, block),
            _ => self.answer_loop(listener, block, &given, folder),
        }
        vec![]
    }

    /// Answers each question that `listener` receives with the block (or the folder's file),
    /// forever.
    fn answer_loop(&mut self, listener: Local, block: Option<&Expr>, given: &Option<Slots>, folder: Option<Local>) {
        let zero = self.constant(0);
        let (request, answer) = (crate::types::request(), crate::types::answer());
        let head = self.new_block();
        self.go(End::Jump(head), head);
        let got = self.int_local();
        self.emit(Inst::Str(got, StrOp::Accept, vec![listener]));
        let q = self.load(got, zero, &request);
        let link = self.node(got, zero, 6);
        self.emit(Inst::ListFree(got));
        let a = match (block, folder) {
            (Some(block), _) => self.run(block, given, vec![(q.clone(), request.clone())], Some(&answer)),
            (None, folder) => {
                let (path, whole) = (self.int_local(), self.int_local());
                self.emit(Inst::Str(path, StrOp::UrlPath, vec![q[1]]));
                self.emit(Inst::Str(whole, StrOp::Concat, vec![folder.expect("a folder"), path]));
                self.emit(Inst::Free(path));
                let a = self.file_answer(whole);
                self.emit(Inst::Free(whole));
                a
            }
        };
        let done = self.int_local();
        self.emit(Inst::Str(done, StrOp::Reply, vec![link, a[0], a[1], a[2], a[3]]));
        self.free_value(&a, &answer);
        self.free_value(&q, &request);
        let after = self.new_block();
        self.go(End::Jump(head), after);
    }

    /// `serve(…) workers(n) { q: Request => … }`, built: `answer_loop` in a function of its own,
    /// run by `n` threads (`StrOp::Workers`). It receives a box holding the listener and the
    /// variables the block reads around it; each thread first copies them, so that none reads
    /// what another one changes (a Map makes its index the first time it is looked into).
    // ponytail: each thread has its own copy of what the block reads (n times its memory); shared
    // values that never change would avoid it.
    fn workers(&mut self, listener: Local, n: Local, block: &Expr) {
        let held = self.held(block);
        let slots: Slots = std::iter::once(listener).chain(held.iter().flat_map(|(_, v, _)| v.clone())).collect();
        let env = self.int_local();
        self.emit(Inst::ListNew(env, slots.len()));
        self.push(env, &slots);
        let mut t = self.inner(Type::Unit);
        let inner_env = t.int_local();
        t.current = t.new_block();
        let (zero, l) = (t.constant(0), t.int_local());
        t.emit(Inst::ListLoad(l, inner_env, zero, 0));
        let mut k = 1;
        for (name, _, ty) in &held {
            let v = t.new_slots(ty);
            for &x in &v {
                t.emit(Inst::ListLoad(x, inner_env, zero, k));
                k += 1;
            }
            let own = t.clone_value(&v, ty);
            t.names[0].vars.insert(name.clone(), (own, ty.clone()));
        }
        t.answer_loop(l, Some(block), &None, None);
        t.finish(End::Return(vec![]));
        let name = format!("workers in {}", t.owner.trim_start_matches("block in "));
        let f = t.function(name, 1, vec![]);
        let num = self.made_function(f);
        let (fun, done) = (self.int_local(), self.int_local());
        self.emit(Inst::FuncRef(fun, num));
        self.emit(Inst::Str(done, StrOp::Workers, vec![n, fun, env]));
    }

    /// The JSON text of a value (PLAN Part 4 "JSON"): a new String, not tracked.
    fn json_text(&mut self, v: &[Local], ty: &Type) -> Local {
        let text = self.int_local();
        match ty {
            Type::Int => self.emit(Inst::Str(text, StrOp::Format(Show::Int), vec![v[0]])),
            Type::Bool => self.emit(Inst::Str(text, StrOp::Format(Show::Bool), vec![v[0]])),
            Type::String => self.emit(Inst::Str(text, StrOp::JsonQuote, vec![v[0]])),
            _ => {
                let f = self.helper(Helper::ToJson(ty.clone()));
                self.emit(Inst::Call(vec![text], f, v.to_vec()));
            }
        }
        text
    }

    /// `to_json` of a Float, an optional, a List, a Map or a struct: compact, fields in their
    /// declared order. A NaN or an infinity has no JSON form: written `null`.
    fn json_writer(mut self, ty: &Type) -> Function {
        let v = self.new_slots(ty);
        self.current = self.new_block();
        let text = match ty {
            Type::Box(t) => {
                let inner = self.unbox(v[0], t);
                self.json_text(&inner, t)
            }
            Type::Float | Type::Optional(_) => {
                let (text, yes, no, done) = (self.int_local(), self.new_block(), self.new_block(), self.new_block());
                let present = if let Type::Float = ty {
                    // Finite: `x - x` is 0 (NaN for NaN and the infinities).
                    let (zero, d, finite) = (self.float_constant(0.0), self.new_local(Scalar::Float), self.int_local());
                    self.emit(Inst::FBinary(d, Op::Minus, v[0], v[0], NOWHERE));
                    self.emit(Inst::FBinary(finite, Op::Eq, d, zero, NOWHERE));
                    finite
                } else {
                    v[0]
                };
                self.go(End::Branch(present, yes, no), yes);
                match ty {
                    Type::Optional(t) => {
                        let inner = self.json_text(&v[1..], t);
                        self.emit(Inst::Copy(text, inner));
                    }
                    _ => self.emit(Inst::Str(text, StrOp::Format(Show::Float), vec![v[0]])),
                }
                self.go(End::Jump(done), no);
                self.emit(Inst::Text(text, "null".into()));
                self.go(End::Jump(done), done);
                text
            }
            Type::List(elem) | Type::Map(_, elem) => {
                // Each element's text in a List, joined once: appending one by one would copy the
                // text made so far at each element.
                let map = matches!(ty, Type::Map(..));
                let each = if map { pair(&Type::String, elem) } else { (**elem).clone() };
                let parts = self.int_local();
                self.emit(Inst::ListNew(parts, 1));
                self.each_element(v[0], &each, |t, _, x| {
                    let piece = if map {
                        let (key, colon) = (t.json_text(&x[..1], &Type::String), t.text(":"));
                        t.append(key, colon);
                        let value = t.json_text(&x[1..], elem);
                        t.append(key, value);
                        key
                    } else {
                        t.json_text(&x, elem)
                    };
                    t.push(parts, &[piece]);
                });
                let (sep, joined) = (self.text(","), self.int_local());
                self.emit(Inst::Str(joined, StrOp::Join, vec![parts, sep]));
                self.emit(Inst::Free(sep));
                self.free_value(&[parts], &Type::List(Box::new(Type::String)));
                let text = self.text(if map { "{" } else { "[" });
                self.append(text, joined);
                let close = self.text(if map { "}" } else { "]" });
                self.append(text, close);
                text
            }
            Type::Struct(d) => {
                let text = self.text("{");
                for (i, ((name, _), t)) in d.fields().iter().zip(members(ty)).enumerate() {
                    let label = self.text(&format!("{}{}:", if i == 0 { "" } else { "," }, crate::json::quote(name)));
                    self.append(text, label);
                    let (start, n) = slice_of(ty, i);
                    let piece = self.json_text(&v[start..start + n], &t);
                    self.append(text, piece);
                }
                let close = self.text("}");
                self.append(text, close);
                text
            }
            _ => unreachable!("a JSON form checked by the type checker (`no_json`)"),
        };
        self.finish(End::Return(vec![text]));
        self.function(format!("to_json {}", ty.text()), v.len(), vec![Scalar::Int])
    }

    /// Slot `k` of node `i` of a JSON text's nodes (`json::slots`).
    fn node(&mut self, nodes: Local, i: Local, k: usize) -> Local {
        let d = self.int_local();
        self.emit(Inst::ListLoad(d, nodes, i, k));
        d
    }

    /// Goes on if `ok` is true; otherwise runs `fail`, then jumps to `done`.
    fn or_else(&mut self, ok: Local, done: BlockId, fail: impl FnOnce(&mut Self)) {
        let (yes, no) = (self.new_block(), self.new_block());
        self.go(End::Branch(ok, yes, no), no);
        fail(self);
        self.go(End::Jump(done), yes);
    }

    /// `T.from_json(text)` on node `i` of a JSON text's nodes, for a `ty`: returns a status
    /// (0 = read, 1 = missing, 2 = wrong kind: `JsonError`'s numbers), the path where it failed
    /// seen from this value (`.author.name`, `[2]`; no String when read) and the value read
    /// (0s when it failed: what was read before is freed).
    fn json_reader(mut self, ty: &Type) -> Function {
        use crate::json::{ARRAY, EXACT_INT, FINITE, NUMBER, OBJECT, TEXT};
        let (nodes, i) = (self.int_local(), self.int_local());
        self.current = self.new_block();
        let (status, path) = (self.constant(0), self.constant(0));
        let v = self.new_slots(ty);
        v.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
        let done = self.new_block();
        let kind = self.node(nodes, i, 0);
        let wrong = |t: &mut Self| {
            t.emit(Inst::Const(status, 2));
            t.emit(Inst::Text(path, String::new()));
        };
        let is = |t: &mut Self, k: i64| {
            let (want, test) = (t.constant(k), t.int_local());
            t.emit(Inst::Binary(test, Op::Eq, kind, want, NOWHERE));
            test
        };
        // A child's failure: its status, and its path after `before` (`[2]`, `.name`).
        let failed = |t: &mut Self, st: Local, before: Vec<Piece>, p: Local| {
            t.emit(Inst::Copy(status, st));
            let mut pieces = before;
            pieces.push(Piece::Value(p, Show::String));
            let whole = t.join(pieces);
            t.emit(Inst::Free(p));
            t.emit(Inst::Copy(path, whole));
        };
        match ty {
            Type::Int | Type::Float => {
                let number = is(&mut self, NUMBER);
                self.or_else(number, done, wrong);
                // An Int is written without a fraction and fits; a Float is finite.
                let (flags, want, ok) = (self.node(nodes, i, 3), self.constant(if *ty == Type::Int { EXACT_INT | FINITE } else { FINITE }), self.int_local());
                self.emit(Inst::Binary(ok, if *ty == Type::Int { Op::Eq } else { Op::Ge }, flags, want, NOWHERE));
                self.or_else(ok, done, wrong);
                self.emit(Inst::ListLoad(v[0], nodes, i, if *ty == Type::Int { 2 } else { 1 }));
            }
            Type::Bool => {
                // `false` and `true` are kinds 1 and 2.
                let (no, yes, either) = (is(&mut self, crate::json::FALSE), is(&mut self, crate::json::TRUE), self.int_local());
                self.emit(Inst::Binary(either, Op::Plus, no, yes, NOWHERE));
                self.or_else(either, done, wrong);
                self.emit(Inst::Copy(v[0], yes));
            }
            Type::String => {
                let text = is(&mut self, TEXT);
                self.or_else(text, done, wrong);
                let s = self.node(nodes, i, 1);
                self.emit(Inst::Str(v[0], StrOp::Clone, vec![s]));
            }
            Type::Optional(t) | Type::Box(t) => {
                // `null` is `none` (a missing field reads the last node, a `null`).
                if let Type::Optional(_) = ty {
                    let (null, read) = (is(&mut self, crate::json::NULL), self.new_block());
                    self.go(End::Branch(null, done, read), read);
                }
                let f = self.helper(Helper::FromJson((**t).clone()));
                let (st, p, x) = (self.int_local(), self.int_local(), self.new_slots(t));
                self.emit(Inst::Call([vec![st, p], x.clone()].concat(), f, vec![nodes, i]));
                self.emit(Inst::Copy(status, st));
                self.emit(Inst::Copy(path, p));
                let (zero, ok) = (self.constant(0), self.int_local());
                self.emit(Inst::Binary(ok, Op::Eq, st, zero, NOWHERE));
                self.or_else(ok, done, |_| {});
                if let Type::Optional(_) = ty {
                    self.emit(Inst::Const(v[0], 1));
                    self.copy(&v[1..], &x);
                } else {
                    let b = self.boxing(x, ty);
                    self.copy(&v, &b);
                }
            }
            Type::List(_) | Type::Map(..) | Type::Struct(_) => {
                let list = matches!(ty, Type::List(_));
                let want = is(&mut self, if list { ARRAY } else { OBJECT });
                self.or_else(want, done, wrong);
                let (first, count) = (self.node(nodes, i, 1), self.node(nodes, i, 2));
                // Node of element `j`, of the name of member `j`, of its value.
                let at = |t: &mut Self, j: Local, by: i64, plus: i64| {
                    let (k, b, d) = (t.constant(by), t.constant(plus), t.int_local());
                    t.emit(Inst::Binary(d, Op::Times, j, k, NOWHERE));
                    t.emit(Inst::Binary(d, Op::Plus, d, b, NOWHERE));
                    t.emit(Inst::Binary(d, Op::Plus, d, first, NOWHERE));
                    d
                };
                let read = |t: &mut Self, elem: &Type, node: Local| {
                    let f = t.helper(Helper::FromJson(elem.clone()));
                    let (st, p, x) = (t.int_local(), t.int_local(), t.new_slots(elem));
                    t.emit(Inst::Call([vec![st, p], x.clone()].concat(), f, vec![nodes, node]));
                    let (zero, ok) = (t.constant(0), t.int_local());
                    t.emit(Inst::Binary(ok, Op::Eq, st, zero, NOWHERE));
                    (ok, st, p, x)
                };
                match ty {
                    Type::List(elem) => {
                        self.emit(Inst::ListNew(v[0], scalars(elem).len()));
                        self.times(count, |t, j| {
                            let node = at(t, j, 1, 0);
                            let (ok, st, p, x) = read(t, elem, node);
                            t.or_else(ok, done, |t| {
                                t.free_value(&v, ty);
                                failed(t, st, vec![Piece::Text("[".into()), Piece::Value(j, Show::Int), Piece::Text("]".into())], p);
                            });
                            t.push(v[0], &x);
                        });
                    }
                    Type::Map(_, value) => {
                        self.emit(Inst::ListNew(v[0], 1 + scalars(value).len()));
                        self.times(count, |t, j| {
                            let (name, node) = (at(t, j, 2, 0), at(t, j, 2, 1));
                            let key = t.node(nodes, name, 1);
                            let (ok, st, p, x) = read(t, value, node);
                            t.or_else(ok, done, |t| {
                                t.free_value(&v, ty);
                                failed(t, st, vec![Piece::Text(".".into()), Piece::Value(key, Show::String)], p);
                            });
                            // The same name twice: the last one is kept.
                            let place = t.entry(v[0], &[key], &Type::String, value);
                            let old = t.read(&place);
                            t.free_value(&old, value);
                            t.write(&place, &x);
                        });
                    }
                    _ => {
                        let Type::Struct(d) = ty else { unreachable!() };
                        let fields = members(ty);
                        for (f, (name, _)) in d.fields().iter().enumerate() {
                            // The node of its value: the last member with its name, or the last node (a `null`).
                            let (found, label) = (self.constant(-1), self.text(name));
                            self.times(count, |t, j| {
                                let (key, node) = (at(t, j, 2, 0), at(t, j, 2, 1));
                                let (k, same) = (t.node(nodes, key, 1), t.int_local());
                                t.emit(Inst::Str(same, StrOp::Eq, vec![k, label]));
                                let (yes, next) = (t.new_block(), t.new_block());
                                t.go(End::Branch(same, yes, next), yes);
                                t.emit(Inst::Copy(found, node));
                                t.go(End::Jump(next), next);
                            });
                            self.emit(Inst::Free(label));
                            let earlier = |t: &mut Self| {
                                for (k, ft) in fields[..f].iter().enumerate() {
                                    let (start, n) = slice_of(ty, k);
                                    t.free_value(&v[start..start + n], ft);
                                }
                            };
                            let (zero, there) = (self.constant(0), self.int_local());
                            self.emit(Inst::Binary(there, Op::Ge, found, zero, NOWHERE));
                            let optional = match &fields[f] {
                                Type::Box(t) => matches!(**t, Type::Optional(_)),
                                t => matches!(t, Type::Optional(_)),
                            };
                            if optional {
                                let (absent, after) = (self.new_block(), self.new_block());
                                self.go(End::Branch(there, after, absent), absent);
                                let (n, one) = (self.int_local(), self.constant(1));
                                self.emit(Inst::ListLength(n, nodes));
                                self.emit(Inst::Binary(found, Op::Minus, n, one, NOWHERE));
                                self.go(End::Jump(after), after);
                            } else {
                                self.or_else(there, done, |t| {
                                    earlier(t);
                                    t.emit(Inst::Const(status, 1));
                                    t.emit(Inst::Text(path, format!(".{name}")));
                                });
                            }
                            let (ok, st, p, x) = read(&mut self, &fields[f], found);
                            self.or_else(ok, done, |t| {
                                earlier(t);
                                failed(t, st, vec![Piece::Text(format!(".{name}"))], p);
                            });
                            let (start, n) = slice_of(ty, f);
                            self.copy(&v[start..start + n], &x);
                        }
                    }
                }
            }
            _ => unreachable!("a JSON form checked by the type checker (`no_json`)"),
        }
        self.go(End::Jump(done), done);
        self.finish(End::Return([vec![status, path], v.clone()].concat()));
        let returns = [vec![Scalar::Int, Scalar::Int], scalars(ty)].concat();
        self.function(format!("from_json {}", ty.text()), 2, returns)
    }

    /// `T.from_json(text)`: `ok(value)`, or `err(JsonError.X(…))` (see `json_reader`).
    fn from_json(&mut self, text: Local, ty: &Type) -> Slots {
        let Type::Result(value, error) = ty else { unreachable!("a result checked by the type checker") };
        let Type::Enum(d) = &**error else { unreachable!("JsonError is an enum") };
        let d = d.clone();
        let v = self.new_slots(ty);
        v.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
        let (nodes, zero, good) = (self.int_local(), self.constant(0), self.int_local());
        self.emit(Inst::Str(nodes, StrOp::JsonParse, vec![text]));
        self.emit(Inst::Binary(good, Op::Gt, nodes, zero, NOWHERE));
        let done = self.new_block();
        // `NotJson(line)`: variant 0.
        self.or_else(good, done, |t| {
            let line = 1 + variant_slice(&d, 0).0;
            t.emit(Inst::Binary(v[line], Op::Minus, zero, nodes, NOWHERE));
        });
        let f = self.helper(Helper::FromJson((**value).clone()));
        let (st, p, x) = (self.int_local(), self.int_local(), self.new_slots(value));
        self.emit(Inst::Call([vec![st, p], x.clone()].concat(), f, vec![nodes, zero]));
        // The texts of the nodes, then the nodes.
        let n = self.int_local();
        self.emit(Inst::ListLength(n, nodes));
        self.times(n, |t, j| {
            let (kind, text, s) = (t.node(nodes, j, 0), t.constant(crate::json::TEXT), t.node(nodes, j, 1));
            let (is_text, yes, next) = (t.int_local(), t.new_block(), t.new_block());
            t.emit(Inst::Binary(is_text, Op::Eq, kind, text, NOWHERE));
            t.go(End::Branch(is_text, yes, next), yes);
            t.emit(Inst::Free(s));
            t.go(End::Jump(next), next);
        });
        self.emit(Inst::ListFree(nodes));
        let ok = self.int_local();
        self.emit(Inst::Binary(ok, Op::Eq, st, zero, NOWHERE));
        let (yes, no) = (self.new_block(), self.new_block());
        self.go(End::Branch(ok, yes, no), yes);
        self.emit(Inst::Const(v[0], 1));
        let k = x.len();
        self.copy(&v[v.len() - k..], &x);
        self.go(End::Jump(done), no);
        // `Missing(field)` or `WrongKind(field)`: the path without its first `.` (`author.name`,
        // `[2].title`), `.` for the whole.
        self.emit(Inst::Copy(v[1], st));
        let (length, field, dot, dotted) = (self.int_local(), self.int_local(), self.text("."), self.int_local());
        self.emit(Inst::Str(length, StrOp::Length, vec![p]));
        self.emit(Inst::Str(dotted, StrOp::StartsWith, vec![p, dot]));
        self.emit(Inst::Free(dot));
        let (whole, part, cut, kept, named) = (self.new_block(), self.new_block(), self.new_block(), self.new_block(), self.new_block());
        self.go(End::Branch(length, part, whole), whole);
        self.emit(Inst::Text(field, ".".into()));
        self.go(End::Jump(named), part);
        self.go(End::Branch(dotted, cut, kept), cut);
        let one = self.constant(1);
        self.emit(Inst::Str(field, StrOp::Slice, vec![p, one, length]));
        self.go(End::Jump(named), kept);
        self.emit(Inst::Str(field, StrOp::Clone, vec![p]));
        self.go(End::Jump(named), named);
        self.emit(Inst::Free(p));
        let (missing, wrong, after) = (self.new_block(), self.new_block(), self.new_block());
        let is_missing = self.with(Op::Eq, st, 1);
        self.go(End::Branch(is_missing, missing, wrong), missing);
        self.emit(Inst::Copy(v[1 + variant_slice(&d, 1).0], field));
        self.go(End::Jump(after), wrong);
        self.emit(Inst::Copy(v[1 + variant_slice(&d, 2).0], field));
        self.go(End::Jump(after), after);
        self.go(End::Jump(done), done);
        self.track(&v, ty);
        v
    }

    /// `List<Book>.from_csv(text)`: `ok(list)`, or `err(CsvError.X(…))` decoded from
    /// `csv::Problem::code`.
    fn from_csv(&mut self, text: Local, ty: &Type) -> Slots {
        let Type::Result(value, error) = ty else { unreachable!("a result checked by the type checker") };
        let (Type::List(elem), Type::Enum(d)) = (&**value, &**error) else { unreachable!("a List, and CsvError") };
        let (elem, d) = ((**elem).clone(), d.clone());
        let v = self.new_slots(ty);
        v.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
        let (spec, r, zero) = (self.text(&csv_spec(&elem)), self.int_local(), self.constant(0));
        self.emit(Inst::Str(r, StrOp::CsvRead, vec![text, spec]));
        self.emit(Inst::Free(spec));
        let (good, done) = (self.int_local(), self.new_block());
        self.emit(Inst::Binary(good, Op::Gt, r, zero, NOWHERE));
        self.or_else(good, done, |t| {
            // -(1 + variant + 4 × (field + 4096 × line))
            let c = t.int_local();
            t.emit(Inst::Binary(c, Op::Minus, zero, r, NOWHERE));
            let c = t.with(Op::Minus, c, 1);
            let variant = t.with(Op::Rem, c, 4);
            let rest = t.with(Op::Div, c, 4);
            let (field, line) = (t.with(Op::Rem, rest, 4096), t.with(Op::Div, rest, 4096));
            t.emit(Inst::Copy(v[1], variant));
            let at = |k: usize, j: usize| v[1 + variant_slice(&d, k).0 + j];
            // `NotCsv(line)`, `Missing(column)`, `WrongKind(line, column)`.
            let (not_csv, missing, wrong, after) = (t.new_block(), t.new_block(), t.new_block(), t.new_block());
            let (is_not, is_missing) = (t.with(Op::Eq, variant, 0), t.with(Op::Eq, variant, 1));
            let other = t.new_block();
            t.go(End::Branch(is_not, not_csv, other), not_csv);
            t.emit(Inst::Copy(at(0, 0), line));
            t.go(End::Jump(after), other);
            let column = t.int_local();
            t.emit(Inst::Const(column, 0));
            // Rows (`List<List<String>>`) only fail with `NotCsv`.
            let names: Vec<String> = match &elem {
                Type::Struct(s) => s.fields().iter().map(|(n, _)| n.clone()).collect(),
                _ => vec![],
            };
            for (k, name) in names.into_iter().enumerate() {
                let (yes, no) = (t.new_block(), t.new_block());
                let is = t.with(Op::Eq, field, k as i64);
                t.go(End::Branch(is, yes, no), yes);
                t.emit(Inst::Text(column, name));
                t.go(End::Jump(no), no);
            }
            t.go(End::Branch(is_missing, missing, wrong), missing);
            t.emit(Inst::Copy(at(1, 0), column));
            t.go(End::Jump(after), wrong);
            t.emit(Inst::Copy(at(2, 0), line));
            t.emit(Inst::Copy(at(2, 1), column));
            t.go(End::Jump(after), after);
        });
        self.emit(Inst::Const(v[0], 1));
        self.emit(Inst::Copy(v[v.len() - 1], r));
        self.go(End::Jump(done), done);
        self.track(&v, ty);
        v
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
            let name = method_name(&ty.owner(), method);
            let (num, site) = match t.numbers.get(name.as_str()) {
                Some(&num) => (num, t.sites[num]),
                None => (t.helper(Helper::Instance(name.clone(), ty.args().to_vec())), t.generic_sites[name.as_str()]),
            };
            let mut vals: Slots = [v.clone(), args.concat()].concat();
            if site {
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
        for (i, (name, fields)) in d.variants().iter().enumerate() {
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
            } else if *ty == crate::types::weekday() {
                // Built in, for people: `Friday`.
                pieces.push(Piece::Text(name.clone()));
            } else if [crate::types::io_error(), crate::types::web_error(), crate::types::json_error(), crate::types::run_error(), crate::types::csv_error()].contains(ty) {
                // Built in, for people: `NotFound("notes.txt")`, `WrongKind(2, "pages")`.
                for (j, (_, t)) in fields.iter().enumerate() {
                    pieces.push(Piece::Text(if j == 0 { format!("{name}(") } else { ", ".into() }));
                    self.value_pieces(&v[at..at + 1], t, &mut pieces);
                    at += 1;
                }
                pieces.push(Piece::Text(")".into()));
            } else {
                pieces.push(Piece::Text(format!("{}.{name}", ty.text())));
                for (j, (field, t)) in fields.iter().enumerate() {
                    pieces.push(Piece::Text(format!("{}{field}: ", if j == 0 { "(" } else { ", " })));
                    let t = &laid_out(&d.name, t);
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
        let vals: Vec<Local> = args.iter().map(|a| self.expr1(a)).collect();
        self.io_result(op, vals, ty)
    }

    /// The same once the arguments are computed (the first: the path or the address). A value of
    /// several slots (a `Response`) comes as the only element of a List made by the runtime.
    fn io_result(&mut self, op: StrOp, vals: Vec<Local>, ty: &Type) -> Slots {
        let Type::Result(value, error) = ty else { unreachable!("a result checked by the type checker") };
        let Type::Enum(d) = &**error else { unreachable!("IoError is an enum") };
        let d = d.clone();
        let (p, to) = (vals[0], vals.get(1).copied());
        let (boxed, two) = (matches!(op, StrOp::Modified), matches!(op, StrOp::CopyFile | StrOp::MoveFile));
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
        let n = scalars(value).len();
        // A time may be below 0: it comes in a List too.
        if n == 1 && !boxed {
            self.emit(Inst::Copy(*v.last().unwrap(), r));
        } else if n > 0 {
            let zero = self.constant(0);
            for k in 0..n {
                self.emit(Inst::ListLoad(v[v.len() - n + k], r, zero, k));
            }
            self.emit(Inst::ListFree(r));
        }
        self.go(End::Jump(after), err);
        // Variant = failure − 1 = −r − 1.
        let minus_one = self.constant(-1);
        self.emit(Inst::Binary(v[1], Op::Minus, minus_one, r, NOWHERE));
        // `copy(a) to(b)`: past 16, the failure is `b`'s.
        let p = if two {
            let (sixteen, second, which) = (self.constant(16), self.int_local(), self.int_local());
            self.emit(Inst::Binary(second, Op::Ge, v[1], sixteen, NOWHERE));
            let (yes, no) = (self.new_block(), self.new_block());
            self.emit(Inst::Copy(which, p));
            self.go(End::Branch(second, yes, no), yes);
            self.emit(Inst::Binary(v[1], Op::Minus, v[1], sixteen, NOWHERE));
            self.emit(Inst::Copy(which, to.unwrap()));
            self.go(End::Jump(no), no);
            which
        } else {
            p
        };
        for i in 0..d.variants().len() {
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
            Type::Box(t) => {
                let (a, b) = (self.unbox(a[0], t), self.unbox(b[0], t));
                self.equal(&a, &b, t, span)
            }
            // A `BigInt` is written one way only: equal texts.
            Type::String | Type::BigInt => {
                let d = self.int_local();
                self.emit(Inst::Str(d, StrOp::Eq, vec![a[0], b[0]]));
                d
            }
            Type::Struct(_) | Type::Enum(_) if self.writes_equals(ty) => self.call_on(ty, "equals", [a, b].concat(), vec![], span, &Type::Bool)[0],
            Type::Struct(_) | Type::Tuple(_) => {
                let same = self.constant(1);
                for (i, t) in members(ty).iter().enumerate() {
                    let (start, n) = slice_of(ty, i);
                    let eq = self.equal(&a[start..start + n], &b[start..start + n], t, span);
                    self.emit(Inst::Binary(same, Op::Times, same, eq, NOWHERE));
                }
                same
            }
            Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::List(_) => {
                let f = self.helper(Helper::Equal(ty.clone()));
                let d = self.int_local();
                self.emit(Inst::Call(vec![d], f, [a, b].concat()));
                d
            }
            _ => self.operation(Op::Eq, a[0], b[0], *ty == Type::Float, NOWHERE),
        }
    }

    /// Does `ty` follow `Equal` with its own `equals`? Only then: an `equals` alone changes
    /// nothing (nothing implicit).
    fn writes_equals(&self, ty: &Type) -> bool {
        let name = method_name(&ty.owner(), "equals");
        self.info.follows(ty, "Equal") && (self.numbers.contains_key(name.as_str()) || self.generic_sites.contains_key(name.as_str()))
    }

    /// A number for `v`, the same for two values `equal` finds equal (same cases, same order):
    /// where a key is put in the index of a Map or a Set (`find_in_index`).
    fn hash(&mut self, v: &[Local], ty: &Type) -> Local {
        match ty {
            // Written inline, the hash of a `Cell` would hold that of its `next`, and so on forever.
            // ponytail: values differing only inside a box share a number; a `Hash` helper if Maps of such keys get slow.
            Type::Box(_) => self.constant(0),
            Type::Int | Type::Bool | Type::Char | Type::Date | Type::DateTime | Type::Pointer => v[0],
            Type::String | Type::BigInt => {
                let d = self.int_local();
                self.emit(Inst::Str(d, StrOp::Hash, vec![v[0]]));
                d
            }
            // Its own `equals` may look at some fields only: all its values get the same number.
            Type::Struct(_) | Type::Enum(_) if self.writes_equals(ty) => self.constant(0),
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
                    let n = scalars(&t).len();
                    let x = self.hash(&v[at..at + n], &t);
                    self.emit(Inst::Mix(h, h, x));
                    at += n;
                }
                h
            }
            // A `Float` (`0.0 = -0.0`, written differently), an `any`.
            _ => self.constant(0),
        }
    }

    /// Inside the `Equal` helper: same length, then the same elements in the same order.
    fn equal_lists(&mut self, a: Local, b: Local, elem: &Type) -> Local {
        let (n, m, same) = (self.int_local(), self.int_local(), self.int_local());
        self.emit(Inst::ListLength(n, a));
        self.emit(Inst::ListLength(m, b));
        self.emit(Inst::Binary(same, Op::Eq, n, m, NOWHERE));
        let (compare, after) = (self.new_block(), self.new_block());
        self.go(End::Branch(same, compare, after), compare);
        self.times(n, |t, i| {
            let (x, y) = (t.load(a, i, elem), t.load(b, i, elem));
            let eq = t.equal(&x, &y, elem, Span::new(0, 0));
            t.emit(Inst::Binary(same, Op::Times, same, eq, NOWHERE));
        });
        self.go(End::Jump(after), after);
        same
    }

    /// Adds a copy of each element of `from` to `list`.
    fn add_all(&mut self, list: Local, from: Local, elem: &Type) {
        self.each_element(from, elem, |t, _, v| {
            let copy = t.clone_value(&v, elem);
            t.push(list, &copy);
        });
    }

    /// Inside the `Equal` helper: same variant, then the same fields for that variant.
    fn equal_variants(&mut self, a: &[Local], b: &[Local], ty: &Type) -> Local {
        let d = enum_view(ty).expect("an enum, an optional or a result");
        let same = self.int_local();
        self.emit(Inst::Binary(same, Op::Eq, a[0], b[0], NOWHERE));
        let after = self.new_block();
        let compare = self.new_block();
        self.go(End::Branch(same, compare, after), compare);
        for (i, (_, fields)) in d.variants().iter().enumerate() {
            let (yes, no) = (self.new_block(), self.new_block());
            let (number, test) = (self.constant(i as i64), self.int_local());
            self.emit(Inst::Binary(test, Op::Eq, a[0], number, NOWHERE));
            self.go(End::Branch(test, yes, no), yes);
            let mut at = variant_slice(&d, i).0;
            for (_, t) in fields {
                let t = &laid_out(&d.name, t);
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
        if matches!(ty, Type::Int | Type::Char | Type::Float | Type::Date | Type::DateTime) {
            return self.operation(Op::Lt, a[0], b[0], *ty == Type::Float, NOWHERE);
        }
        if matches!(ty, Type::String | Type::BigInt) {
            let (d, op) = (self.int_local(), if *ty == Type::BigInt { StrOp::BigLess } else { StrOp::Less });
            self.emit(Inst::Str(d, op, vec![a[0], b[0]]));
            return d;
        }
        self.call_on(ty, "less", [a, b].concat(), vec![], span, &Type::Bool)[0]
    }

    /// Runs `body` on each element of `list` (with its index), in order.
    fn each_element(&mut self, list: Local, elem: &Type, mut body: impl FnMut(&mut Self, Local, Slots)) {
        let n = self.int_local();
        self.emit(Inst::ListLength(n, list));
        self.times(n, |t, i| {
            let v = t.load(list, i, elem);
            body(t, i, v);
        });
    }

    /// Runs `body` with `i` = 0, 1… up to `n` excluded.
    fn times(&mut self, n: Local, mut body: impl FnMut(&mut Self, Local)) {
        let i = self.constant(0);
        let (head, b_body, exit) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(head), head);
        let more = self.int_local();
        self.emit(Inst::Binary(more, Op::Lt, i, n, NOWHERE));
        self.go(End::Branch(more, b_body, exit), b_body);
        body(self, i);
        let one = self.constant(1);
        self.emit(Inst::Binary(i, Op::Plus, i, one, NOWHERE));
        self.go(End::Jump(head), exit);
    }

    /// The slots of a field laid out as `t`: the value `v` itself, or a box holding it (`laid_out`).
    fn boxing(&mut self, v: Slots, t: &Type) -> Slots {
        let Type::Box(inner) = t else { return v };
        let b = self.int_local();
        self.emit(Inst::ListNew(b, scalars(inner).len()));
        self.push(b, &v);
        vec![b]
    }

    /// The value inside the box `b` (see `laid_out`): borrowed from it.
    fn unbox(&mut self, b: Local, t: &Type) -> Slots {
        let zero = self.constant(0);
        self.load(b, zero, t)
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
    fn narrow(&mut self, names: &[(&str, Option<usize>)]) {
        let mut vars = HashMap::new();
        for &(n, is) in names {
            let (v, ty) = self.variable(n);
            match (ty, is) {
                (Type::Optional(inner), None) => {
                    vars.insert(n.to_string(), (v[1..].to_vec(), *inner));
                }
                // `a is Dog`: the Dog's slots, read from the box (they stay there).
                (Type::Any(_), Some(id)) => {
                    let ty = self.info.tested[&id].clone();
                    let (zero, slots) = (self.constant(0), self.new_slots(&ty));
                    for (j, &d) in slots.iter().enumerate() {
                        self.emit(Inst::ListLoad(d, v[0], zero, j + 1));
                    }
                    vars.insert(n.to_string(), (slots, ty));
                }
                _ => unreachable!("narrowing checked by the type checker"),
            }
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

    /// Where member `i` of the value at `place` (of type `ty`) lives, and its type: inside its
    /// box if it has one (`laid_out`).
    fn member(&mut self, place: Place, ty: &Type, i: usize) -> (Place, Type) {
        let place = place.narrow(slice_of(ty, i));
        match element(ty, i) {
            Type::Box(t) => {
                let list = self.read(&place)[0];
                let index = self.constant(0);
                (Place::Element { list, index, start: 0, kinds: scalars(&t) }, *t)
            }
            t => (place, t),
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
                        Step::Index(i) => self.member(place, &ty, *i),
                        Step::Name(n) => {
                            let i = field_index(&ty, &n.text);
                            self.member(place, &ty, i)
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
                (self.member(p, &t, *i).0, ty)
            }
            ExprKind::Method { receiver, name, args } if args.is_empty() && self.ty(receiver).is_field(&name.text) => {
                let (p, t) = self.place(receiver);
                let i = field_index(&t, &name.text);
                (self.member(p, &t, i).0, ty)
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
    /// it (`unindex` in runtime/kova.rs and interp.rs).
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
    /// A Float's bits, for the runtime.
    fn seconds(&mut self, v: f64) -> Local {
        let (f, bits) = (self.float_constant(v), self.int_local());
        self.emit(Inst::Convert(bits, Conversion::ToBits, f, NOWHERE));
        bits
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
    /// `for x in var l`: puts back each `x` of the loops from number `from` on into its element.
    fn write_back(&mut self, from: usize) {
        // The innermost first: a box of a `match var` field before the value holding it.
        for (_, place, v) in self.writes.clone().into_iter().rev().filter(|w| w.0 >= from) {
            self.write(&place, &v);
        }
    }

    fn borrowing(&mut self, name: &str, v: Slots, ty: Type, body: &Block) {
        self.names.push(Scope { borrowed: true, vars: HashMap::from([(name.to_string(), (v, ty))]) });
        self.block(body);
        self.names.pop();
    }

    fn block(&mut self, b: &Block) {
        let base = self.names.len();
        self.names.push(Scope::default());
        self.statements(&b.stmts);
        self.free_scopes(base);
        self.names.truncate(base);
    }

    /// In order. After `if x = none { return }`, `x` is its value until the end of the block
    /// (`Info::guards`): a borrowed scope for it, then a new one for the names declared after.
    /// From a statement that changes it (`types::changes`, only a `var`'s), `x` is its optional again.
    fn statements(&mut self, stmts: &[Stmt]) {
        let mut held: Vec<(String, (Slots, Type))> = vec![];
        for s in stmts {
            let (back, kept): (Vec<_>, _) = held.into_iter().partition(|(n, _)| changes(s, n));
            held = kept;
            // `x := x + 1`: the value is computed with `x` its value, then `x` is its optional.
            let plain = matches!(s, Stmt::Assign { target: Target::Place { name, fields, .. }, op: None, value }
                if fields.is_empty() && back.iter().any(|(n, _)| *n == name.text) && !changes_in(value, &name.text));
            if plain {
                self.reopen = Some(back.into_iter().collect());
            } else if !back.is_empty() {
                self.names.push(Scope { borrowed: true, vars: back.into_iter().collect() });
                self.names.push(Scope::default());
            }
            self.stmt(s);
            if let Stmt::If { cond, .. } = s
                && let Some(names) = self.info.guards.get(&cond.id)
            {
                held.extend(names.iter().map(|n| (n.clone(), self.variable(n))));
                let names: Vec<(&str, Option<usize>)> = names.iter().map(|n| (n.as_str(), None)).collect();
                self.narrow(&names);
                self.names.push(Scope::default());
            }
        }
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
                    if let Some(vars) = self.reopen.take() {
                        self.names.push(Scope { borrowed: true, vars });
                        self.names.push(Scope::default());
                    }
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
                    let r = if let Type::List(elem) = &ty {
                        self.add_all(old, v, elem);
                        old
                    } else if ty == Type::String {
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
                self.write_back(self.loops.len());
                let (_, exit, depth) = *self.loops.last().unwrap();
                self.free_scopes(depth);
                self.finish(End::Jump(exit));
            }
            Stmt::Continue(_) => {
                self.write_back(self.loops.len());
                let (next, _, depth) = *self.loops.last().unwrap();
                self.free_scopes(depth);
                self.finish(End::Jump(next));
            }
            Stmt::If { cond, then, otherwise } => {
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
            Stmt::Loop { counter, start, end, inclusive, step, body } => {
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
                let test = match (*inclusive, *step > 0) {
                    (true, true) => Op::Le,
                    (false, true) => Op::Lt,
                    (true, false) => Op::Ge,
                    (false, false) => Op::Gt,
                };
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
                let by = self.constant(*step);
                self.emit(Inst::Binary(i, Op::Plus, i, by, loc));
                self.go(End::Jump(head), exit);
            }
            Stmt::Each { name, value, mode, list, body } => {
                let elem = match self.ty(list) {
                    Type::List(elem) | Type::Set(elem) => *elem,
                    Type::Map(k, v) => pair(&k, &v),
                    _ => unreachable!("List or Map checked by the type checker"),
                };
                // The List cannot change during the loop (`ownership.rs`): its length is read once.
                // `for x in move l`: the loop owns `l`, freed at the end of the statement (or at a
                // `return`); each element taken is emptied in it, so only the others are freed.
                let moving = *mode == ast::Mode::Move;
                let l = if moving {
                    let l = self.owned(list)[0];
                    self.temps.push((l, self.ty(list)));
                    l
                } else {
                    self.expr1(list)
                };
                let n = self.int_local();
                self.emit(Inst::ListLength(n, l));
                let i = self.constant(0);
                let (head, b_body, next, exit) = (self.new_block(), self.new_block(), self.new_block(), self.new_block());
                self.go(End::Jump(head), head);
                let more = self.int_local();
                self.emit(Inst::Binary(more, Op::Lt, i, n, NOWHERE));
                self.go(End::Branch(more, b_body, exit), b_body);
                let v = self.load(l, i, &elem);
                if moving {
                    let zero = self.constant(0);
                    for (k, kind) in heap(&elem).into_iter().enumerate() {
                        if kind.is_some() {
                            self.emit(Inst::ListStore(l, i, k, zero));
                        }
                    }
                }
                let base = self.names.len();
                self.loops.push((next, exit, base));
                if *mode == ast::Mode::Var {
                    let place = Place::Element { list: l, index: i, start: 0, kinds: scalars(&elem) };
                    self.writes.push((self.loops.len(), place, v.clone()));
                }
                // Borrowed from the List: nobody frees it (unless `move`: freed at the end of each turn).
                let vars = match value {
                    None => HashMap::from([(name.text.clone(), (v, elem))]),
                    Some(value) if matches!(self.ty(list), Type::List(_)) => {
                        HashMap::from([(name.text.clone(), (vec![i], Type::Int)), (value.text.clone(), (v, elem))])
                    }
                    // `for name, age in ages`: the key's slots, then the value's.
                    Some(value) => {
                        let (k, t) = (element(&elem, 0), element(&elem, 1));
                        let n = scalars(&k).len();
                        HashMap::from([(name.text.clone(), (v[..n].to_vec(), k)), (value.text.clone(), (v[n..].to_vec(), t))])
                    }
                };
                self.names.push(Scope { borrowed: !moving, vars });
                self.block(body);
                self.free_scopes(base);
                self.names.truncate(base);
                self.go(End::Jump(next), next);
                self.write_back(self.loops.len());
                if *mode == ast::Mode::Var {
                    self.writes.pop();
                }
                self.loops.pop();
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
        self.write_back(0);
        // The temporaries of the enclosing statements (a loop bound…) are freed on this path
        // only: they stay listed, the normal path frees them at the end of their statement.
        for (l, kind) in self.temps.clone().into_iter().rev() {
            self.free_slot(l, &kind);
        }
        // The `ensure`s may read a `move` parameter: checked before it is freed.
        for (c, vars) in self.ensures.clone() {
            self.result = Some((v.clone(), self.ret.clone()));
            let named = vars.map(|vars| self.names.push(Scope { borrowed: true, vars })).is_some();
            self.check(c, self.site);
            if named {
                self.names.pop();
            }
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
        self.called_from(site, &mut m);
        // The program stops: the temporaries of the message are never freed.
        self.temps.truncate(mark);
        self.emit(Inst::Fail(m));
        self.go(End::Unreachable, next);
    }

    /// `mode build else "message"` in the other mode: the call stops the program, like a
    /// `require` that fails, with the clause shown under the message.
    fn stop(&mut self, span: Span, parts: &[Part], site: Option<(Local, Local)>) {
        let mark = self.temps.len();
        let mut m = vec![Piece::Label("error_prefix")];
        for p in parts {
            self.part_pieces(p, &mut m);
        }
        let l = self.loc(span);
        let clause = format!(":{}:{}\n  {}\n", l.line, l.col, &self.src[span.start..span.end]);
        m.extend([Piece::Text("\n  --> ".into()), Piece::File(l.file), Piece::Text(clause)]);
        self.called_from(site, &mut m);
        self.temps.truncate(mark);
        self.emit(Inst::Fail(m));
        let dead = self.new_block();
        self.go(End::Unreachable, dead);
    }

    /// `  called from main.kova:10:11`, for a function that receives its call's location.
    fn called_from(&self, site: Option<(Local, Local)>, m: &mut Vec<Piece>) {
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
    }

    /// A value the way it would be written in Kova: `3`, `2.5`, `'a'`, `(1, true)`,
    /// `Dog { name: "Rex", age: 3 }`, `[1, 2]`.
    fn value_pieces(&mut self, v: &[Local], ty: &Type, m: &mut Vec<Piece>) {
        match ty {
            Type::Box(t) => {
                let inner = self.unbox(v[0], t);
                self.value_pieces(&inner, t, m)
            }
            // A type following `Display` is always shown by its `to_string`.
            Type::Struct(_) | Type::Enum(_) if self.info.follows(ty, "Display") && !self.debug => {
                let text = self.call_on(ty, "to_string", v.to_vec(), vec![], Span::new(0, 0), &Type::String);
                m.push(Piece::Value(text[0], Show::String));
            }
            Type::Struct(d) if d.fields().is_empty() => m.push(Piece::Text(format!("{} {{}}", ty.text()))),
            Type::Struct(d) => {
                m.push(Piece::Text(format!("{} {{ ", ty.text())));
                for (i, ((name, _), t)) in d.fields().iter().zip(members(ty)).enumerate() {
                    let sep = if i > 0 { ", " } else { "" };
                    m.push(Piece::Text(format!("{sep}{name}: ")));
                    let (start, n) = slice_of(ty, i);
                    self.value_pieces(&v[start..start + n], &t, m);
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
            Type::Date | Type::DateTime => self.date_pieces(v[0], ty, m),
            // `Pointer(140245…)`: its address, which only means something to C.
            Type::Pointer => {
                m.push(Piece::Text("Pointer(".into()));
                m.push(Piece::Value(v[0], Show::Int));
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

    // Dates (PLAN Part 4, "Dates"): a `Date` is its number of days since 1970-01-01, a `DateTime`
    // its seconds since 1970-01-01 00:00. Howard Hinnant's `days_from_civil` and `civil_from_days`
    // in plain `Int` operations, so run and build count the same days. The year starts on March 1st,
    // so that February, the only month of varying length, comes last.

    /// `a op k`.
    fn with(&mut self, op: Op, a: Local, k: i64) -> Local {
        let k = self.constant(k);
        self.operation(op, a, k, false, NOWHERE)
    }

    fn op(&mut self, op: Op, a: Local, b: Local) -> Local {
        self.operation(op, a, b, false, NOWHERE)
    }

    /// The days since 1970-01-01 of a year (1 or later), a month and a day.
    fn days_from_civil(&mut self, y: Local, m: Local, d: Local) -> Local {
        let early = self.with(Op::Le, m, 2);
        let y = self.op(Op::Minus, y, early);
        let era = self.with(Op::Div, y, 400);
        let t = self.with(Op::Times, era, 400);
        let yoe = self.op(Op::Minus, y, t);
        // The month counted from March: `m - 3`, or `m + 9` for January and February.
        let late = self.with(Op::Gt, m, 2);
        let t = self.with(Op::Times, late, 12);
        let mp = self.with(Op::Plus, m, 9);
        let mp = self.op(Op::Minus, mp, t);
        let t = self.with(Op::Times, mp, 153);
        let t = self.with(Op::Plus, t, 2);
        let t = self.with(Op::Div, t, 5);
        let t = self.op(Op::Plus, t, d);
        let doy = self.with(Op::Minus, t, 1);
        let t = self.with(Op::Times, yoe, 365);
        let q = self.with(Op::Div, yoe, 4);
        let t = self.op(Op::Plus, t, q);
        let c = self.with(Op::Div, yoe, 100);
        let t = self.op(Op::Minus, t, c);
        let doe = self.op(Op::Plus, t, doy);
        let t = self.with(Op::Times, era, 146097);
        let t = self.op(Op::Plus, t, doe);
        self.with(Op::Minus, t, 719468)
    }

    /// Year, month, day of a number of days since 0000-03-01 (never negative for a year 1 or later).
    fn civil_from_days(&mut self, z: Local) -> (Local, Local, Local) {
        let era = self.with(Op::Div, z, 146097);
        let t = self.with(Op::Times, era, 146097);
        let doe = self.op(Op::Minus, z, t);
        let a = self.with(Op::Div, doe, 1460);
        let b = self.with(Op::Div, doe, 36524);
        let c = self.with(Op::Div, doe, 146096);
        let t = self.op(Op::Minus, doe, a);
        let t = self.op(Op::Plus, t, b);
        let t = self.op(Op::Minus, t, c);
        let yoe = self.with(Op::Div, t, 365);
        let t = self.with(Op::Times, era, 400);
        let y = self.op(Op::Plus, yoe, t);
        let t = self.with(Op::Times, yoe, 365);
        let q = self.with(Op::Div, yoe, 4);
        let t = self.op(Op::Plus, t, q);
        let c = self.with(Op::Div, yoe, 100);
        let t = self.op(Op::Minus, t, c);
        let doy = self.op(Op::Minus, doe, t);
        let t = self.with(Op::Times, doy, 5);
        let t = self.with(Op::Plus, t, 2);
        let mp = self.with(Op::Div, t, 153);
        let t = self.with(Op::Times, mp, 153);
        let t = self.with(Op::Plus, t, 2);
        let t = self.with(Op::Div, t, 5);
        let t = self.op(Op::Minus, doy, t);
        let d = self.with(Op::Plus, t, 1);
        // `mp + 3`, or `mp - 9` for January and February.
        let late = self.with(Op::Ge, mp, 10);
        let t = self.with(Op::Times, late, 12);
        let m = self.with(Op::Plus, mp, 3);
        let m = self.op(Op::Minus, m, t);
        let early = self.with(Op::Le, m, 2);
        let y = self.op(Op::Plus, y, early);
        (y, m, d)
    }

    /// Year, month, day, then hour, minute, second of a `Date` or a `DateTime`.
    fn civil(&mut self, v: Local, ty: &Type) -> Vec<Local> {
        if *ty == Type::Date {
            let z = self.with(Op::Plus, v, 719468);
            let (y, m, d) = self.civil_from_days(z);
            return vec![y, m, d];
        }
        let s = self.with(Op::Plus, v, 719468 * 86400);
        let z = self.with(Op::Div, s, 86400);
        let (y, m, d) = self.civil_from_days(z);
        let rest = self.with(Op::Rem, s, 86400);
        let hour = self.with(Op::Div, rest, 3600);
        let t = self.with(Op::Rem, rest, 3600);
        let minute = self.with(Op::Div, t, 60);
        let second = self.with(Op::Rem, rest, 60);
        vec![y, m, d, hour, minute, second]
    }

    /// `low <= x <= high`, as 1 or 0.
    fn within(&mut self, x: Local, low: i64, high: i64) -> Local {
        let a = self.with(Op::Ge, x, low);
        let b = self.with(Op::Le, x, high);
        self.op(Op::Times, a, b)
    }

    /// A runtime error if `ok` is 0.
    fn unless(&mut self, ok: Local, fault: Fault, loc: Loc) {
        let bad = self.int_local();
        self.emit(Inst::Not(bad, ok));
        self.emit(Inst::FaultIf(bad, fault, loc));
    }

    /// `Date.year(y) month(m) day(d)` (3 numbers) or a `DateTime` (5 or 6): checked, then counted.
    fn date_made(&mut self, v: &[Local], loc: Loc) -> Local {
        let mut ok = self.within(v[0], 1, 9999);
        for (x, high) in [(v[1], 12), (v[2], 31)] {
            let w = self.within(x, 1, high);
            ok = self.op(Op::Times, ok, w);
        }
        self.unless(ok, Fault::BadDate, loc);
        let days = self.days_from_civil(v[0], v[1], v[2]);
        // November 31 is counted as December 1st: the day found again is not the one given.
        let z = self.with(Op::Plus, days, 719468);
        let (_, _, d) = self.civil_from_days(z);
        let same = self.op(Op::Eq, d, v[2]);
        self.unless(same, Fault::BadDate, loc);
        if v.len() == 3 {
            return days;
        }
        let mut ok = self.within(v[3], 0, 23);
        let mut total = self.with(Op::Times, days, 86400);
        for (i, k) in [(3, 3600), (4, 60), (5, 1)] {
            let Some(&x) = v.get(i) else { break };
            if i > 3 {
                let w = self.within(x, 0, 59);
                ok = self.op(Op::Times, ok, w);
            }
            let t = self.with(Op::Times, x, k);
            total = self.op(Op::Plus, total, t);
        }
        self.unless(ok, Fault::BadTime, loc);
        total
    }

    /// `d.year`, `t.minute`, `d.plus_days(3)`, `d.days_until(e)`, `d.weekday`, `d.to_string`, `d.clone`.
    fn date_method(&mut self, v: Local, ty: &Type, name: &str, arg: Option<Local>, loc: Loc) -> Local {
        let parts = ["year", "month", "day", "hour", "minute", "second"];
        if let Some(i) = parts.iter().position(|p| *p == name) {
            return self.civil(v, ty)[i];
        }
        let day = if *ty == Type::Date { 1 } else { 86400 };
        let step = [("plus_days", day), ("plus_hours", 3600), ("plus_minutes", 60), ("plus_seconds", 1)];
        if let Some(&(_, k)) = step.iter().find(|(n, _)| *n == name) {
            let n = self.constant(k);
            let by = self.operation(Op::Times, arg.unwrap(), n, false, loc);
            let r = self.operation(Op::Plus, v, by, false, loc);
            // Still between 0001-01-01 and 9999-12-31 23:59:59.
            let (first, after) = (-719162 * day, 2932897 * day);
            let low = self.with(Op::Ge, r, first);
            let high = self.with(Op::Lt, r, after);
            let ok = self.op(Op::Times, low, high);
            self.unless(ok, Fault::BadDate, loc);
            return r;
        }
        if name == "plus_months" {
            return self.plus_months(v, ty, arg.unwrap(), loc);
        }
        match name {
            "days_until" | "seconds_until" => return self.op(Op::Minus, arg.unwrap(), v),
            // The days counted from 0000-03-01, so that the division rounds down.
            "date" => {
                let s = self.with(Op::Plus, v, 719468 * 86400);
                let z = self.with(Op::Div, s, 86400);
                return self.with(Op::Minus, z, 719468);
            }
            // 1970-01-01 was a Thursday (3, from Monday = 0); 719468 days is 1 more than whole weeks.
            "weekday" => {
                let z = if *ty == Type::Date { v } else { self.date_method(v, ty, "date", None, loc) };
                let t = self.with(Op::Plus, z, 719468 + 2);
                return self.with(Op::Rem, t, 7);
            }
            _ => {}
        }
        if name == "to_string" {
            let mut pieces = vec![];
            self.date_pieces(v, ty, &mut pieces);
            let text = self.join(pieces);
            self.temps.push((text, Type::String));
            return text;
        }
        let copy = self.int_local();
        self.emit(Inst::Copy(copy, v));
        copy
    }

    /// `d.plus_months(n)`: the same day `n` months later, or the last day of that month if it is
    /// shorter (January 31 + 1 month = February 28 or 29); a `DateTime` keeps its time.
    fn plus_months(&mut self, v: Local, ty: &Type, n: Local, loc: Loc) -> Local {
        let parts = self.civil(v, ty);
        let (y, m, d) = (parts[0], parts[1], parts[2]);
        // The months since year 0, from 12 (0001-01) to 12 * 10000 - 1 (9999-12).
        let t = self.with(Op::Times, y, 12);
        let t = self.op(Op::Plus, t, m);
        let t = self.with(Op::Minus, t, 1);
        let total = self.operation(Op::Plus, t, n, false, loc);
        let ok = self.within(total, 12, 12 * 10000 - 1);
        self.unless(ok, Fault::BadDate, loc);
        let ny = self.with(Op::Div, total, 12);
        let t = self.with(Op::Rem, total, 12);
        let nm = self.with(Op::Plus, t, 1);
        // Its length: 30 + (m + m / 8) % 2, but February: 28, 29 in a leap year.
        let t = self.with(Op::Div, nm, 8);
        let t = self.op(Op::Plus, nm, t);
        let t = self.with(Op::Rem, t, 2);
        let long = self.with(Op::Plus, t, 30);
        let r4 = self.with(Op::Rem, ny, 4);
        let by4 = self.with(Op::Eq, r4, 0);
        let r100 = self.with(Op::Rem, ny, 100);
        let not100 = self.with(Op::Ne, r100, 0);
        let r400 = self.with(Op::Rem, ny, 400);
        let by400 = self.with(Op::Eq, r400, 0);
        let t = self.op(Op::Times, by4, not100);
        let t = self.op(Op::Plus, t, by400);
        let leap = self.with(Op::Gt, t, 0);
        let feb = self.with(Op::Plus, leap, 28);
        let is_feb = self.with(Op::Eq, nm, 2);
        let t = self.op(Op::Minus, feb, long);
        let t = self.op(Op::Times, is_feb, t);
        let length = self.op(Op::Plus, long, t);
        // min(d, length).
        let fits = self.op(Op::Le, d, length);
        let t = self.op(Op::Minus, d, length);
        let t = self.op(Op::Times, fits, t);
        let nd = self.op(Op::Plus, length, t);
        let days = self.days_from_civil(ny, nm, nd);
        if *ty == Type::Date {
            return days;
        }
        // The time of day, kept.
        let date = self.date_method(v, ty, "date", None, loc);
        let start = self.with(Op::Times, date, 86400);
        let time = self.op(Op::Minus, v, start);
        let t = self.with(Op::Times, days, 86400);
        self.op(Op::Plus, t, time)
    }

    /// `2026-10-23`, `2026-10-23 14:05:30`: as ISO 8601 writes them, every number with its zeros.
    fn date_pieces(&mut self, v: Local, ty: &Type, m: &mut Vec<Piece>) {
        let parts = self.civil(v, ty);
        for (i, &x) in parts.iter().enumerate() {
            m.push(Piece::Text(["", "-", "-", " ", ":", ":"][i].into()));
            for k in if i == 0 { [1000, 100, 10, 1].as_slice() } else { &[10, 1] } {
                let t = self.with(Op::Div, x, *k);
                let t = self.with(Op::Rem, t, 10);
                let c = self.with(Op::Plus, t, '0' as i64);
                m.push(Piece::Value(c, Show::Char));
            }
        }
    }

    /// The pieces of a part of a text: a String or a number as `print` shows it, a struct
    /// as `Dog { name: "Rex", age: 3 }`, a tuple as `(1, "a")`.
    fn part_pieces(&mut self, p: &Part, m: &mut Vec<Piece>) {
        match p {
            Part::Text(t) => m.push(Piece::Text(t.clone())),
            Part::Code(e) => match &self.ty(e) {
                ty @ (Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_) | Type::Date | Type::DateTime | Type::Pointer) => {
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

    /// Calls `method` of `ty`: for a `Stack<Int>`, its own copy of `Stack.push` (also when the
    /// type checker only saw a `T`, in a copy of a generic function).
    fn call_on(&mut self, ty: &Type, method: &str, vals: Slots, places: Vec<(Place, Slots)>, span: Span, ret: &Type) -> Slots {
        let name = method_name(&ty.owner(), method);
        if ty.args().is_empty() || !self.generic_sites.contains_key(name.as_str()) {
            return self.call(&name, vals, places, span, ret);
        }
        let types = ty.args().iter().map(|t| substitute(t, &self.subst)).collect();
        let num = self.helper(Helper::Instance(name.clone(), types));
        self.call_to(num, self.generic_sites[name.as_str()], vals, places, span, ret)
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

    /// `sqlite.open("notes.db", var db)` (PLAN Part 4 "`extern`, step 1"): each argument made a C
    /// value (a text copied, an `Int32` checked), the call, then its result made a Kova value.
    fn extern_call(&mut self, name: &str, args: &[Expr], span: Span, ty: &Type) -> Slots {
        let k = extern_order(self.info).iter().position(|n| *n == name).expect("an extern function");
        let (params, result) = (self.info.externs[name].params.clone(), self.info.externs[name].result);
        let (mut vals, mut places) = (vec![], vec![]);
        let modes = self.signatures[name].1.clone();
        for ((a, mode), (c, _)) in args.iter().zip(modes).zip(&params) {
            let value = if let ExprKind::Mark(_, x) = &a.kind { x } else { a };
            match (c, mode) {
                (CType::Callback(k), _) => {
                    let (cs, r) = self.info.externs[name].callbacks[*k].clone();
                    let address = self.callback(value, &cs, r, span);
                    vals.push(address);
                }
                (_, Mode::Var) => {
                    let (p, ty) = self.place(value);
                    let v = self.read(&p);
                    let out = self.new_slots(&ty);
                    vals.extend(&v);
                    places.push((p, out));
                }
                _ => vals.extend(self.expr(value)),
            }
        }
        let loc = self.loc(span);
        let (mut given, mut copies, mut at) = (vec![], vec![], 0);
        for (c, _) in &params {
            at += match c {
                CType::Struct(s) => self.c_leaves(*s, &vals[at..], loc, &mut given, &mut copies),
                c => self.c_leaves_of(*c, &vals[at..], loc, &mut given, &mut copies),
            };
        }
        let got = match result {
            CType::Nothing => vec![],
            CType::Float => self.new_slots(&Type::Float),
            _ => vec![self.int_local()],
        };
        // What C left at each `var` parameter's address (each leaf of a struct), then made a Kova value.
        let kinds: Vec<Vec<CType>> = params
            .iter()
            .filter(|(_, var)| *var)
            .map(|(c, _)| match c {
                CType::Struct(s) => c_layout(*s, &self.info.c_structs).0.into_iter().map(|(_, c)| c).collect(),
                c => vec![*c],
            })
            .collect();
        let raw: Vec<Slots> = kinds.iter().map(|cs| cs.iter().map(|&c| if c == CType::Float { self.new_slots(&Type::Float)[0] } else { self.int_local() }).collect()).collect();
        self.emit(Inst::Extern([got.clone(), raw.concat()].concat(), k, given, loc));
        for ((place, out), (r, cs)) in places.iter().zip(raw.iter().zip(&kinds)) {
            let v = self.kova_of_leaves(cs, r, None, loc);
            for (&o, x) in out.iter().zip(v) {
                self.emit(Inst::Copy(o, x));
            }
            self.write(place, out);
        }
        let zero = self.constant(0);
        let v = match result {
            CType::Nothing => vec![],
            CType::Bool | CType::MaybePointer => {
                let present = self.int_local();
                self.emit(Inst::Binary(present, Op::Ne, got[0], zero, NOWHERE));
                if result == CType::Bool { vec![present] } else { vec![present, got[0]] }
            }
            CType::Text | CType::WideText | CType::Pointer | CType::Struct(_) => {
                let null = self.int_local();
                self.emit(Inst::Binary(null, Op::Eq, got[0], zero, NOWHERE));
                let fault = if matches!(result, CType::Text | CType::WideText) { Fault::NullText } else { Fault::NullPointer };
                self.emit(Inst::FaultIf(null, fault, loc));
                match result {
                    CType::Pointer => got,
                    CType::Struct(k) => self.c_struct_at(k, got[0], None, loc),
                    _ => {
                        let s = self.int_local();
                        let op = if result == CType::Text { StrOp::FromCText } else { StrOp::FromCWideText };
                        self.emit(Inst::Str(s, op, vec![got[0]]));
                        vec![s]
                    }
                }
            }
            CType::MaybeText | CType::MaybeWideText => {
                let (present, s) = (self.int_local(), self.int_local());
                self.emit(Inst::Binary(present, Op::Ne, got[0], zero, NOWHERE));
                let op = if result == CType::MaybeText { StrOp::FromCText } else { StrOp::FromCWideText };
                self.emit(Inst::Str(s, op, vec![got[0]]));
                vec![present, s]
            }
            // Its leaves read 0 at NULL: an empty value under `none`.
            CType::MaybeStruct(k) => {
                let present = self.int_local();
                self.emit(Inst::Binary(present, Op::Ne, got[0], zero, NOWHERE));
                [vec![present], self.c_struct_at(k, got[0], Some(present), loc)].concat()
            }
            _ => got,
        };
        // Only now: the result may be one of the texts lent to C (`memcpy` gives its target back).
        for (copy, free) in copies {
            let done = self.int_local();
            self.emit(Inst::Str(done, free, vec![copy]));
        }
        self.track(&v, ty);
        v
    }

    /// The Kova slots of C values (`cs` their C types): a `Bool` 0 or 1 (C wrote any `int`), a
    /// `Pointer?` present if not NULL, a text copied (C keeps its own; NULL stops the program,
    /// unless the struct holding it is `none`: `present`).
    fn kova_of_leaves(&mut self, cs: &[CType], raws: &[Local], present: Option<Local>, loc: Loc) -> Slots {
        let mut out = vec![];
        for (&c, &l) in cs.iter().zip(raws) {
            let zero = self.constant(0);
            match c {
                CType::Bool | CType::MaybePointer | CType::MaybeText | CType::MaybeWideText => {
                    let there = self.int_local();
                    self.emit(Inst::Binary(there, Op::Ne, l, zero, NOWHERE));
                    out.push(there);
                }
                _ => {}
            }
            match c {
                CType::Bool => {}
                CType::Text | CType::WideText | CType::MaybeText | CType::MaybeWideText => {
                    if matches!(c, CType::Text | CType::WideText) {
                        let null = self.int_local();
                        self.emit(Inst::Binary(null, Op::Eq, l, zero, NOWHERE));
                        if let Some(p) = present {
                            self.emit(Inst::Binary(null, Op::Times, null, p, NOWHERE));
                        }
                        self.emit(Inst::FaultIf(null, Fault::NullText, loc));
                    }
                    let text = self.int_local();
                    let op = if matches!(c, CType::Text | CType::MaybeText) { StrOp::FromCText } else { StrOp::FromCWideText };
                    self.emit(Inst::Str(text, op, vec![l]));
                    out.push(text);
                }
                _ => out.push(l),
            }
        }
        out
    }

    /// A copy of the struct `s` C keeps at `address` (PLAN Part 4 "`extern`, step 4"): each leaf
    /// read (`StrOp::CRead`, 0 at NULL), then made a Kova value; `present`: whether the address is
    /// one, for a `T?`.
    fn c_struct_at(&mut self, s: usize, address: Local, present: Option<Local>, loc: Loc) -> Slots {
        let (leaves, _, _) = c_layout(s, &self.info.c_structs);
        let (mut raws, mut cs) = (vec![], vec![]);
        for (offset, c) in leaves {
            let (at, kind, r) = (self.constant(offset as i64), self.constant(matches!(c, CType::Int32 | CType::Bool) as i64), self.int_local());
            self.emit(Inst::Str(r, StrOp::CRead, vec![address, at, kind]));
            raws.push(if c == CType::Float {
                let x = self.new_slots(&Type::Float)[0];
                self.emit(Inst::Convert(x, Conversion::FromBits, r, NOWHERE));
                x
            } else {
                r
            });
            cs.push(c);
        }
        self.kova_of_leaves(&cs, &raws, present, loc)
    }

    /// The C values of a Kova value of C type `c` (its slots first in `v`): an `Int32` checked,
    /// `none` the value slot left at 0 (NULL), a text copied for the call (freed after it, by
    /// `copies`); how many Kova slots it read.
    fn c_leaves_of(&mut self, c: CType, v: &[Local], loc: Loc, given: &mut Slots, copies: &mut Vec<(Local, StrOp)>) -> usize {
        match c {
            CType::Text | CType::MaybeText | CType::WideText | CType::MaybeWideText => {
                let plain = matches!(c, CType::Text | CType::WideText);
                let (present, s) = if plain { (self.constant(1), v[0]) } else { (v[0], v[1]) };
                let (copy, minus, zero) = (self.int_local(), self.constant(-1), self.int_local());
                let (make, free) = if matches!(c, CType::Text | CType::MaybeText) { (StrOp::CText, StrOp::CFree) } else { (StrOp::CWideText, StrOp::CWideFree) };
                self.emit(Inst::Str(copy, make, vec![s, present]));
                self.emit(Inst::Binary(zero, Op::Eq, copy, minus, NOWHERE));
                self.emit(Inst::FaultIf(zero, Fault::TextWithZero, loc));
                given.push(copy);
                copies.push((copy, free));
                return if plain { 1 } else { 2 };
            }
            CType::Int32 => {
                let (low, high, (min, max)) = (self.int_local(), self.int_local(), (self.constant(i32::MIN as i64), self.constant(i32::MAX as i64)));
                self.emit(Inst::Binary(low, Op::Lt, v[0], min, NOWHERE));
                self.emit(Inst::Binary(high, Op::Gt, v[0], max, NOWHERE));
                self.emit(Inst::FaultIf(low, Fault::Int32Range, loc));
                self.emit(Inst::FaultIf(high, Fault::Int32Range, loc));
                given.push(v[0]);
            }
            CType::MaybePointer => {
                given.push(v[1]);
                return 2;
            }
            _ => given.push(v[0]),
        }
        1
    }

    /// The C values of each leaf of struct `s` (PLAN Part 4 "`extern`, step 3"), in the order of
    /// `types::c_layout`; how many Kova slots it read.
    fn c_leaves(&mut self, s: usize, v: &[Local], loc: Loc, given: &mut Slots, copies: &mut Vec<(Local, StrOp)>) -> usize {
        let mut at = 0;
        for c in self.info.c_structs[s].clone() {
            at += match c {
                CType::Struct(j) => self.c_leaves(j, &v[at..], loc, given, copies),
                c => self.c_leaves_of(c, &v[at..], loc, given, copies),
            };
        }
        at
    }

    /// A function C calls back (PLAN Part 4 "`extern`, step 2"): `callback in main` takes the C
    /// values (an `Int32` or a `Bool` already widened by codegen's trampoline), makes them Kova
    /// values, runs the block or calls the function given by its name, and gives back its result
    /// as a C value. The address C receives is the trampoline's (`Inst::Callback`).
    fn callback(&mut self, e: &Expr, params: &[CType], result: CType, span: Span) -> Local {
        let Type::Block(ps, r) = self.ty(e) else { unreachable!("callback checked by the type checker") };
        let loc = self.loc(span);
        let mut t = self.inner((*r).clone());
        let raw: Slots = params.iter().map(|&c| if c == CType::Float { t.new_slots(&Type::Float)[0] } else { t.int_local() }).collect();
        let nb_params = t.locals.len();
        t.current = t.new_block();
        let zero = t.constant(0);
        let mut vals = vec![];
        for ((&c, &l), ty) in params.iter().zip(&raw).zip(&ps) {
            let v = match c {
                CType::Bool | CType::MaybePointer => {
                    let present = t.int_local();
                    t.emit(Inst::Binary(present, Op::Ne, l, zero, NOWHERE));
                    if c == CType::Bool { vec![present] } else { vec![present, l] }
                }
                // A struct C gives by its address: a copy (`MSG*`).
                CType::Struct(k) => {
                    let null = t.int_local();
                    t.emit(Inst::Binary(null, Op::Eq, l, zero, NOWHERE));
                    t.emit(Inst::FaultIf(null, Fault::NullPointer, loc));
                    t.c_struct_at(k, l, None, loc)
                }
                CType::MaybeStruct(k) => {
                    let present = t.int_local();
                    t.emit(Inst::Binary(present, Op::Ne, l, zero, NOWHERE));
                    [vec![present], t.c_struct_at(k, l, Some(present), loc)].concat()
                }
                _ => vec![l],
            };
            vals.push((v, ty.clone()));
        }
        let x = match self.info.callbacks.get(&e.id) {
            Some(f) => t.call(f, vals.into_iter().flat_map(|(v, _)| v).collect(), vec![], span, &r),
            None => t.run(e, &None, vals, Some(&*r).filter(|r| **r != Type::Unit)),
        };
        let out = match result {
            CType::Nothing => vec![],
            // `none` leaves its value slot at 0: NULL.
            CType::MaybePointer => vec![x[1]],
            CType::Int32 => {
                let (low, high) = (t.int_local(), t.int_local());
                let (min, max) = (t.constant(i32::MIN as i64), t.constant(i32::MAX as i64));
                t.emit(Inst::Binary(low, Op::Lt, x[0], min, NOWHERE));
                t.emit(Inst::Binary(high, Op::Gt, x[0], max, NOWHERE));
                t.emit(Inst::FaultIf(low, Fault::Int32Range, loc));
                t.emit(Inst::FaultIf(high, Fault::Int32Range, loc));
                vec![x[0]]
            }
            _ => vec![x[0]],
        };
        let returns = out.iter().map(|&l| t.locals[l]).collect();
        t.free_temps(0);
        t.finish(End::Return(out));
        let name = format!("callback in {}", t.owner.trim_start_matches("block in "));
        let f = t.function(name, nb_params, returns);
        let num = self.made_function(f);
        let address = self.int_local();
        self.emit(Inst::Callback(address, num, params.to_vec(), result));
        address
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
        let held = self.held(e);
        let slots: Slots = held.iter().flat_map(|(_, v, _)| v.clone()).collect();
        let env = if slots.is_empty() { self.constant(0) } else {
            let env = self.int_local();
            self.emit(Inst::ListNew(env, slots.len()));
            self.push(env, &slots);
            env
        };
        self.envs.push((env, held.iter().map(|(_, v, _)| v.clone()).collect()));
        let mut t = self.inner((*r).clone());
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
        let num = self.made_function(f);
        let fun = self.int_local();
        self.emit(Inst::FuncRef(fun, num));
        vec![fun, env]
    }

    /// `l.map workers(n) { … }`, `l.each workers(n) { … }` (PLAN Part 4 "Several things at once"):
    /// built, `n` threads share the elements (`StrOp::Parallel`), each with its own copy of what
    /// the block reads (as `workers`); `map` writes each result at its element's place, in a List
    /// made that long first. `kova run` does them one after the other, like `map` and `each`.
    fn parallel(&mut self, e: &Expr, list: Local, elem: &Type, name: &str, args: &[Expr]) -> Slots {
        let n = self.expr1(&args[0]);
        let (one, bad, loc) = (self.constant(1), self.int_local(), self.loc(e.span));
        self.emit(Inst::Binary(bad, Op::Lt, n, one, NOWHERE));
        self.emit(Inst::FaultIf(bad, Fault::BadWorkers, loc));
        let base = name.trim_end_matches(" workers(…)");
        if !self.built {
            return self.block_method(e, list, elem, base, &args[1..]);
        }
        let (block, ty) = (&args[1], self.ty(e));
        let made = match &ty {
            Type::List(t) => Some((**t).clone()),
            _ => None,
        };
        let length = self.int_local();
        self.emit(Inst::ListLength(length, list));
        let out = match &made {
            Some(t) => {
                let out = self.int_local();
                self.emit(Inst::ListNew(out, scalars(t).len()));
                self.times(length, |t, _| {
                    let i = t.int_local();
                    t.emit(Inst::ListGrow(i, out));
                });
                out
            }
            None => self.constant(0),
        };
        let held = self.held(block);
        let slots: Slots = [list, out].into_iter().chain(held.iter().flat_map(|(_, v, _)| v.clone())).collect();
        let env = self.int_local();
        self.emit(Inst::ListNew(env, slots.len()));
        self.push(env, &slots);

        let mut t = self.inner(Type::Unit);
        let (inner_env, next) = (t.int_local(), t.int_local());
        t.current = t.new_block();
        let (zero, l, o) = (t.constant(0), t.int_local(), t.int_local());
        t.emit(Inst::ListLoad(l, inner_env, zero, 0));
        t.emit(Inst::ListLoad(o, inner_env, zero, 1));
        let mut k = 2;
        for (name, _, ty) in &held {
            let v = t.new_slots(ty);
            for &x in &v {
                t.emit(Inst::ListLoad(x, inner_env, zero, k));
                k += 1;
            }
            // Its own: freed when the thread has no element left.
            let own = t.clone_value(&v, ty);
            t.names[1].vars.insert(name.clone(), (own, ty.clone()));
        }
        let (count, i, more) = (t.int_local(), t.int_local(), t.int_local());
        t.emit(Inst::ListLength(count, l));
        let (head, body, exit) = (t.new_block(), t.new_block(), t.new_block());
        t.go(End::Jump(head), head);
        t.emit(Inst::Str(i, StrOp::Next, vec![next]));
        t.emit(Inst::Binary(more, Op::Lt, i, count, NOWHERE));
        t.go(End::Branch(more, body, exit), body);
        let v = t.load(l, i, elem);
        match &made {
            Some(r) => {
                let x = t.run(block, &None, vec![(v, elem.clone())], Some(r));
                for (k, &s) in x.iter().enumerate() {
                    t.emit(Inst::ListStore(o, i, k, s));
                }
            }
            None => {
                t.run(block, &None, vec![(v, elem.clone())], None);
            }
        }
        t.go(End::Jump(head), exit);
        t.free_scopes(1);
        t.finish(End::Return(vec![]));
        let name = format!("workers in {}", t.owner.trim_start_matches("block in "));
        let f = t.function(name, 2, vec![]);
        let num = self.made_function(f);
        let (fun, done) = (self.int_local(), self.int_local());
        self.emit(Inst::FuncRef(fun, num));
        self.emit(Inst::Str(done, StrOp::Parallel, vec![n, length, fun, env]));
        self.emit(Inst::ListFree(env));
        match made {
            Some(_) => {
                self.track(&[out], &ty);
                vec![out]
            }
            None => vec![],
        }
    }

    /// The variables block `e` uses around it (`Info::captures`): names, slots and types.
    fn held(&self, e: &Expr) -> Vec<(String, Slots, Type)> {
        (self.info.captures.get(&e.id).into_iter().flatten())
            .map(|n| {
                let (v, t) = self.variable(n);
                (n.clone(), v, t)
            })
            .collect()
    }

    /// A translator for a function made from this one's code (a block's), giving `ret`.
    fn inner(&self, ret: Type) -> Translator<'a> {
        Translator {
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
            ret,
            result: None,
            site: None,
            outs: vec![],
            names: vec![Scope::borrowed(), Scope::default()],
            locals: vec![],
            blocks: vec![],
            current: 0,
            loops: vec![],
            writes: vec![],
            temps: vec![],
            envs: vec![],
            env_marks: vec![],
            reopen: None,
        }
    }

    /// The number of `f`, a function made from this one's code (`Helper::Block`).
    fn made_function(&self, f: Function) -> usize {
        let k = {
            let mut made = self.made.borrow_mut();
            made.push(Some(f));
            made.len() - 1
        };
        self.helper(Helper::Block(k))
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
        let block = self.written_block(args.last().expect("block checked by the type checker"));
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
        // `x.fixed(2)`, a text; `x.round(2)`, a Float: their digits are the same in run and build.
        if name == "fixed" || (name == "round" && !args.is_empty()) {
            let (digits, bits, dst) = (self.expr1(&args[0]), self.int_local(), self.int_local());
            self.emit(Inst::Convert(bits, Conversion::ToBits, v, NOWHERE));
            self.emit(Inst::Str(dst, if name == "fixed" { StrOp::Fixed } else { StrOp::RoundTo }, vec![bits, digits]));
            if name == "fixed" {
                self.temps.push((dst, Type::String));
                return vec![dst];
            }
            let x = self.new_slots(&Type::Float)[0];
            self.emit(Inst::Convert(x, Conversion::FromBits, dst, NOWHERE));
            return vec![x];
        }
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
        let maths = [
            ("sin", Math::Sin),
            ("cos", Math::Cos),
            ("tan", Math::Tan),
            ("asin", Math::Asin),
            ("acos", Math::Acos),
            ("atan", Math::Atan),
            ("ln", Math::Ln),
            ("log10", Math::Log10),
            ("exp", Math::Exp),
        ];
        if let Some(&(_, m)) = maths.iter().find(|(n, _)| *n == name) {
            self.emit(Inst::Math(out, m, v, v));
            return vec![out];
        }
        if name == "pow" && float && self.ty(&args[0]) == Type::Float {
            let y = self.expr1(&args[0]);
            self.emit(Inst::Math(out, Math::Pow, v, y));
            return vec![out];
        }
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
        let block = block.map(|b| self.written_block(b));
        // A merge sort from the bottom up, stable: runs of 1, 2, 4… elements merged from `src` into
        // `dst`, then the two Lists change roles. Elements are moved, never copied: `dst` starts with
        // `n` empty elements, and the List left over at the end is freed without its elements.
        let ty = Type::List(Box::new(elem.clone()));
        let width = scalars(elem).len();
        let src = self.clone_value(&[list], &ty)[0];
        let given = block.filter(|b| !matches!(b.kind, ExprKind::Block { .. })).map(|b| self.expr(b));
        let (n, dst, zero, one, two) = (self.int_local(), self.int_local(), self.constant(0), self.constant(1), self.constant(2));
        self.emit(Inst::ListLength(n, src));
        self.emit(Inst::ListNew(dst, width));
        let k = self.int_local();
        self.emit(Inst::Copy(k, n));
        self.repeat(
            |t| t.compare(Op::Gt, k, zero),
            |t| {
                let i = t.int_local();
                t.emit(Inst::ListGrow(i, dst));
                t.emit(Inst::Binary(k, Op::Minus, k, one, NOWHERE));
            },
        );
        let run = self.int_local();
        self.emit(Inst::Copy(run, one));
        self.repeat(
            |t| t.compare(Op::Lt, run, n),
            |t| {
                let lo = t.int_local();
                t.emit(Inst::Copy(lo, zero));
                t.repeat(
                    |t| t.compare(Op::Lt, lo, n),
                    |t| {
                        let (mid, hi, i, j, k) = (t.int_local(), t.int_local(), t.int_local(), t.int_local(), t.int_local());
                        t.emit(Inst::Binary(mid, Op::Plus, lo, run, NOWHERE));
                        t.at_most(mid, n);
                        t.emit(Inst::Binary(hi, Op::Plus, mid, run, NOWHERE));
                        t.at_most(hi, n);
                        t.emit(Inst::Copy(i, lo));
                        t.emit(Inst::Copy(j, mid));
                        t.emit(Inst::Copy(k, lo));
                        t.repeat(
                            |t| t.compare(Op::Lt, k, hi),
                            |t| {
                                // The right element goes first only when it is strictly before the
                                // left one: equal elements keep their order.
                                let (left, right, test, take_left, done) = (t.new_block(), t.new_block(), t.new_block(), t.new_block(), t.new_block());
                                let more_left = t.compare(Op::Lt, i, mid);
                                t.go(End::Branch(more_left, test, right), test);
                                let more_right = t.compare(Op::Lt, j, hi);
                                let compare = t.new_block();
                                t.go(End::Branch(more_right, compare, take_left), compare);
                                let (a, b) = (t.load(src, j, elem), t.load(src, i, elem));
                                let first = match block {
                                    Some(block) => t.run(block, &given, vec![(a, elem.clone()), (b, elem.clone())], Some(&Type::Bool))[0],
                                    None => t.less(&a, &b, elem, span),
                                };
                                t.go(End::Branch(first, right, take_left), take_left);
                                t.go(End::Jump(left), left);
                                t.move_element(src, i, dst, k, width);
                                t.emit(Inst::Binary(i, Op::Plus, i, one, NOWHERE));
                                t.go(End::Jump(done), right);
                                t.move_element(src, j, dst, k, width);
                                t.emit(Inst::Binary(j, Op::Plus, j, one, NOWHERE));
                                t.go(End::Jump(done), done);
                                t.emit(Inst::Binary(k, Op::Plus, k, one, NOWHERE));
                            },
                        );
                        t.emit(Inst::Copy(lo, hi));
                    },
                );
                let swap = t.int_local();
                t.emit(Inst::Copy(swap, src));
                t.emit(Inst::Copy(src, dst));
                t.emit(Inst::Copy(dst, swap));
                t.emit(Inst::Binary(run, Op::Times, run, two, NOWHERE));
            },
        );
        self.emit(Inst::ListFree(dst));
        self.track(&[src], &ty);
        vec![src]
    }

    /// `l.shuffled` (PLAN Part 4 "The library after Python, step 3"): a copy, then Fisher and
    /// Yates: from the end, each element changes place with one drawn among those before it or
    /// itself (`random`'s chance, so `KOVA_SEED` gives the same order again).
    fn shuffled(&mut self, list: Local, elem: &Type) -> Slots {
        let ty = Type::List(Box::new(elem.clone()));
        let width = scalars(elem).len();
        let copy = self.clone_value(&[list], &ty)[0];
        let (i, zero, one) = (self.int_local(), self.constant(0), self.constant(1));
        self.emit(Inst::ListLength(i, copy));
        self.emit(Inst::Binary(i, Op::Minus, i, one, NOWHERE));
        self.repeat(
            |t| t.compare(Op::Gt, i, zero),
            |t| {
                let (to, j) = (t.int_local(), t.int_local());
                t.emit(Inst::Binary(to, Op::Plus, i, one, NOWHERE));
                t.emit(Inst::Str(j, StrOp::Random, vec![zero, to]));
                for slot in 0..width {
                    let (a, b) = (t.int_local(), t.int_local());
                    t.emit(Inst::ListLoad(a, copy, i, slot));
                    t.emit(Inst::ListLoad(b, copy, j, slot));
                    t.emit(Inst::ListStore(copy, i, slot, b));
                    t.emit(Inst::ListStore(copy, j, slot, a));
                }
                t.emit(Inst::Binary(i, Op::Minus, i, one, NOWHERE));
            },
        );
        self.track(&[copy], &ty);
        vec![copy]
    }

    /// `l.chosen`: a copy of an element drawn by `random`'s chance, `none` if `l` is empty.
    /// A copy of the element `index` gives from the List's length, or `none` when the List is empty.
    fn element_or_none(&mut self, list: Local, elem: &Type, index: impl FnOnce(&mut Self, Local) -> Local) -> Slots {
        let ty = Type::Optional(Box::new(elem.clone()));
        let v = self.new_slots(&ty);
        v.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
        let (n, zero) = (self.int_local(), self.constant(0));
        self.emit(Inst::ListLength(n, list));
        let (some, yes, done) = (self.int_local(), self.new_block(), self.new_block());
        self.emit(Inst::Binary(some, Op::Gt, n, zero, NOWHERE));
        self.go(End::Branch(some, yes, done), yes);
        let i = index(self, n);
        let element = self.load(list, i, elem);
        let copy = self.clone_value(&element, elem);
        self.emit(Inst::Const(v[0], 1));
        self.copy(&v[1..], &copy);
        self.go(End::Jump(done), done);
        self.track(&v, &ty);
        v
    }

    fn chosen(&mut self, list: Local, elem: &Type) -> Slots {
        self.element_or_none(list, elem, |t, n| {
            let (i, zero) = (t.int_local(), t.constant(0));
            t.emit(Inst::Str(i, StrOp::Random, vec![zero, n]));
            i
        })
    }

    /// `l.min`, `l.max`: the first of the smallest, or of the biggest.
    fn extreme(&mut self, list: Local, elem: &Type, biggest: bool, span: Span) -> Slots {
        self.element_or_none(list, elem, |t, _| {
            let best = t.constant(0);
            t.each_element(list, elem, |t, i, v| {
                let kept = t.load(list, best, elem);
                let better = if biggest { t.less(&kept, &v, elem, span) } else { t.less(&v, &kept, elem, span) };
                let (yes, next) = (t.new_block(), t.new_block());
                t.go(End::Branch(better, yes, next), yes);
                t.emit(Inst::Copy(best, i));
                t.go(End::Jump(next), next);
            });
            best
        })
    }

    /// `l.is_empty`, also for a Map or a Set, laid out like a List.
    fn is_empty(&mut self, list: Local) -> Slots {
        let (n, zero) = (self.int_local(), self.constant(0));
        self.emit(Inst::ListLength(n, list));
        vec![self.compare(Op::Eq, n, zero)]
    }

    /// `dst := a op b`, a new Bool local.
    fn compare(&mut self, op: Op, a: Local, b: Local) -> Local {
        let dst = self.int_local();
        self.emit(Inst::Binary(dst, op, a, b, NOWHERE));
        dst
    }

    /// `while test { body }`, with the test made again at each turn.
    fn repeat(&mut self, test: impl FnOnce(&mut Self) -> Local, body: impl FnOnce(&mut Self)) {
        let (top, inside, out) = (self.new_block(), self.new_block(), self.new_block());
        self.go(End::Jump(top), top);
        let more = test(self);
        self.go(End::Branch(more, inside, out), inside);
        body(self);
        self.go(End::Jump(top), out);
    }

    /// `x := n` when `x > n`.
    fn at_most(&mut self, x: Local, n: Local) {
        let (lower, past) = (self.new_block(), self.new_block());
        let over = self.compare(Op::Gt, x, n);
        self.go(End::Branch(over, lower, past), lower);
        self.emit(Inst::Copy(x, n));
        self.go(End::Jump(past), past);
    }

    /// Element `i` of `from` becomes element `k` of `to` (its slots copied, `from`'s left as they were).
    fn move_element(&mut self, from: Local, i: Local, to: Local, k: Local, width: usize) {
        for slot in 0..width {
            let v = self.int_local();
            self.emit(Inst::ListLoad(v, from, i, slot));
            self.emit(Inst::ListStore(to, k, slot, v));
        }
    }

    /// `block`, or the block a function given by its name stands for (`Checker::named_block`).
    fn written_block<'b>(&self, block: &'b Expr) -> &'b Expr
    where
        'a: 'b,
    {
        self.info.filled.get(&block.id).filter(|f| matches!(f.kind, ExprKind::Block { .. })).unwrap_or(block)
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

    fn if_value(&mut self, cond: &Expr, then: &Block, otherwise: &Block, ty: &Type) -> Slots {
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
        let base = self.names.len();
        self.names.push(Scope::default());
        let (last, before) = b.stmts.split_last().expect("block checked by the type checker");
        self.statements(before);
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
        self.free_scopes(base);
        self.names.truncate(base);
        v
    }

    /// `match`: jumps to the arm of the value's variant; with `dst`, each arm's value goes there.
    /// On a place (`match shape`), the arms borrow its fields; a copied value is copied first, so
    /// that the arms may change the place. A computed value belongs to the `match`: each arm frees
    /// what it does not bind.
    fn match_(&mut self, m: &Match, dst: Option<(&[Local], &Type)>) {
        let ty = self.ty(&m.subject);
        // `None` for a `match` on values (`match n { 0 => … }`).
        let view = enum_view(&ty);
        // `match chain(3).next`: a part of a temporary is borrowed too, the temporary freed after.
        let part = match &m.subject.kind {
            ExprKind::Field(..) => true,
            ExprKind::Method { receiver, name, args } => args.is_empty() && self.ty(receiver).is_field(&name.text),
            ExprKind::Index(x, i) => !matches!(self.ty(x), Type::Map(..)) && i.range().is_none(),
            _ => false,
        };
        let borrowed = part || place_root(&m.subject, &self.info.types).is_some();
        let mut place = None;
        let v = if borrowed {
            let (p, _) = self.place(&m.subject);
            let v = self.read(&p);
            place = Some(p);
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
            match (&arm.case, arm.case.variant()) {
                // `1, 2 =>`: different values, so at most one test gives 1. A text written there is
                // freed at once: on another path, it is never made.
                (Case::Values(values), _) => {
                    let mark = self.temps.len();
                    let mut test = None;
                    for x in values {
                        let is = match &x.kind {
                            // `1..=5 =>`: from 1 to 5.
                            ExprKind::Binary(op @ (BinOp::Range | BinOp::RangeInclusive), a, b) => {
                                let (lo, hi) = (self.expr1(a), self.expr1(b));
                                let above = self.op(Op::Ge, v[0], lo);
                                let below = self.op(if *op == BinOp::Range { Op::Lt } else { Op::Le }, v[0], hi);
                                self.op(Op::Times, above, below)
                            }
                            _ => {
                                let x = self.expr(x);
                                self.equal(&v, &x, &ty, m.subject.span)
                            }
                        };
                        test = Some(match test {
                            None => is,
                            Some(t) => self.op(Op::Plus, t, is),
                        });
                    }
                    self.free_temps(mark);
                    self.go(End::Branch(test.unwrap(), body, next), body);
                }
                // The last arm takes what is left: the type checker made sure nothing else is.
                (_, Some((name, _))) if k + 1 < m.arms.len() => {
                    let d = view.as_ref().expect("an enum, an optional or a result");
                    // `Saturday, Sunday =>`: the variants are different, so at most one test gives 1.
                    let mut test = None;
                    for name in std::iter::once(name).chain(arm.also.iter().map(|n| n.text.as_str())) {
                        let (number, is) = (self.constant(d.variant(name).unwrap().0 as i64), self.int_local());
                        self.emit(Inst::Binary(is, Op::Eq, v[0], number, NOWHERE));
                        test = Some(match test {
                            None => is,
                            Some(t) => self.op(Op::Plus, t, is),
                        });
                    }
                    self.go(End::Branch(test.unwrap(), body, next), body);
                }
                _ => self.go(End::Jump(body), body),
            }
            self.names.push(Scope { borrowed, vars: HashMap::new() });
            let marks = self.writes.len();
            match arm.case.variant() {
                Some((name, binds)) if !binds.is_empty() => {
                    let d = view.as_ref().expect("an enum, an optional or a result");
                    let (i, fields) = d.variant(name).unwrap();
                    let mut at = variant_slice(d, i).0;
                    // `match var`: the value changed through the names goes back to its place.
                    if m.mode == ast::Mode::Var {
                        self.writes.push((self.loops.len(), place.clone().expect("a place checked by the type checker"), v.clone()));
                    }
                    for (p, (_, t)) in binds.into_iter().zip(fields) {
                        let n = scalars(&laid_out(&d.name, t)).len();
                        let mut slots = v[at..at + n].to_vec();
                        if let Type::Box(_) = laid_out(&d.name, t) {
                            // Out of its box: given to the name, the box alone is freed.
                            slots = self.unbox(v[at], t);
                            if !borrowed {
                                self.emit(Inst::ListFree(v[at]));
                            } else if m.mode == ast::Mode::Var {
                                let index = self.constant(0);
                                self.writes.push((self.loops.len(), Place::Element { list: v[at], index, start: 0, kinds: scalars(t) }, slots.clone()));
                            }
                        }
                        self.bind(p, &slots, t);
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
            for (_, p, v) in self.writes.split_off(marks).into_iter().rev() {
                self.write(&p, &v);
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
        // `connect("x")`: the call with its default values (`types::Info::filled`).
        if let Some(full) = self.info.filled.get(&e.id).filter(|f| !std::ptr::eq(*f, e)) {
            return self.value(full);
        }
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
                    let i = field_index(&ty, &name.text);
                    // A function C calls: the address of its trampoline (PLAN Part 4 "`extern`, step 5").
                    if let Some((cs, r)) = self.info.callback_fields.get(&method_name(&ty.owner(), &name.text)).cloned() {
                        parts[i] = vec![self.callback(value, &cs, r, value.span)];
                        continue;
                    }
                    let v = self.owned(value);
                    parts[i] = self.boxing(v, &element(&ty, i));
                }
                let v: Slots = parts.concat();
                self.track(&v, &ty);
                v
            }
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
            // `Set["a", "b"]` or `Set<String>()`: added one after the other, a value given twice is kept once.
            ExprKind::SetLit(_) | ExprKind::New(_) if matches!(self.ty(e), Type::Set(_)) => {
                let ty = self.ty(e);
                let Type::Set(elem) = &ty else { unreachable!() };
                let l = self.int_local();
                self.emit(Inst::ListNew(l, scalars(elem).len()));
                if let ExprKind::SetLit(elems) = &e.kind {
                    for x in elems {
                        let v = self.owned(x);
                        self.set_add(l, &v, elem);
                    }
                }
                self.track(&[l], &ty);
                vec![l]
            }
            ExprKind::SetLit(_) => unreachable!("a Set, checked by the type checker"),
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
                        ty @ (Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::List(_) | Type::Map(..) | Type::Set(_) | Type::Any(_) | Type::Date | Type::DateTime | Type::Pointer) => {
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
                    "files.read" => Some(StrOp::ReadFile),
                    "files.write" => Some(StrOp::WriteFile),
                    "files.append" => Some(StrOp::AppendFile),
                    "files.list" => Some(StrOp::ListDir),
                    "files.make_dir" => Some(StrOp::MakeDir),
                    "files.remove" => Some(StrOp::RemoveFile),
                    "files.copy(…) to(…)" => Some(StrOp::CopyFile),
                    "files.move(…) to(…)" => Some(StrOp::MoveFile),
                    "files.size" => Some(StrOp::FileSize),
                    "files.modified" => Some(StrOp::Modified),
                    "files.matching" => Some(StrOp::Matching),
                    "files.sha256" => Some(StrOp::FileSha256),
                    _ => None,
                };
                if let Some(op) = io {
                    let ty = self.ty(e);
                    return self.io_call(op, args, &ty);
                }
                // `get(url)`, `post(url) body(text)` (PLAN Part 4 "The web"): the address, the
                // method, what is sent, and whether the redirections are followed.
                if matches!(name.text.as_str(), "web.get" | "web.post(…) body(…)") {
                    let ty = self.ty(e);
                    let url = self.expr1(&args[0]);
                    let (method, body) = match args.get(1) {
                        Some(b) => (self.text("POST"), self.expr1(b)),
                        None => (self.text("GET"), self.text("")),
                    };
                    let (follow, headers, seconds) = (self.constant(1), self.constant(0), self.seconds(DEFAULT_SECONDS));
                    let v = self.io_result(StrOp::Web, vec![url, method, body, follow, headers, seconds], &ty);
                    self.emit(Inst::Free(method));
                    if args.len() == 1 {
                        self.emit(Inst::Free(body));
                    }
                    return v;
                }
                // `serve(…)`, `serve_files(…) at(…)`, then `log(…)` and `workers(…)` parts in any
                // order (PLAN "The web server, what it says", "Several questions at once").
                let parts: Vec<&str> = name.text.split(' ').collect();
                let base = parts[0].trim_end_matches("(…)");
                if let "web.serve" | "web.serve_network" | "web.serve_files" | "web.serve_files_network" = base {
                    let files = base.starts_with("web.serve_files");
                    let firsts = if files { 2 } else { 1 };
                    let given: Vec<Local> = args[..firsts].iter().map(|a| self.expr1(a)).collect();
                    let (mut log, mut workers) = (None, None);
                    for (part, a) in parts.iter().zip(args).skip(firsts) {
                        let v = Some(self.expr1(a));
                        if *part == "log(…)" { log = v } else { workers = v }
                    }
                    let log = log.unwrap_or_else(|| self.constant(0));
                    let loc = self.loc(name.span);
                    let everyone = base.ends_with("_network");
                    return match files {
                        true => self.serve(given[1], log, everyone, None, Some(given[0]), None, loc),
                        false => self.serve(given[0], log, everyone, args.last(), None, workers, loc),
                    };
                }
                if matches!(name.text.as_str(), "system.run" | "system.run(…) args(…)") {
                    let ty = self.ty(e);
                    let program = self.expr1(&args[0]);
                    let words = match args.get(1) {
                        Some(w) => self.expr1(w),
                        None => self.constant(0),
                    };
                    return self.io_result(StrOp::Run, vec![program, words], &ty);
                }
                // `send(r)`: a `Request`'s fields (`types::request`), in `StrOp::Web`'s order.
                if name.text == "web.send" {
                    let ty = self.ty(e);
                    let r = self.expr(&args[0]);
                    let seconds = self.int_local();
                    self.emit(Inst::Convert(seconds, Conversion::ToBits, r[4], NOWHERE));
                    return self.io_result(StrOp::Web, vec![r[1], r[0], r[3], r[5], r[2], seconds], &ty);
                }
                if name.text == "ui.show" {
                    let vals: Vec<Local> = args.iter().map(|a| self.expr1(a)).collect();
                    let clicked = self.int_local();
                    self.emit(Inst::Str(clicked, StrOp::UiShow, vals));
                    return vec![clicked];
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
                if name.text == "secure.token" {
                    let (n, zero) = (self.expr1(&args[0]), self.constant(0));
                    let (below, loc) = (self.int_local(), self.loc(args[0].span));
                    self.emit(Inst::Binary(below, Op::Lt, n, zero, NOWHERE));
                    self.emit(Inst::FaultIf(below, Fault::NegativeSize, loc));
                    let t = self.int_local();
                    self.emit(Inst::Str(t, StrOp::SecureToken, vec![n]));
                    self.track(&[t], &Type::String);
                    return vec![t];
                }
                // `secure.password_hash(p) memory(mb) passes(n)`, the parts left out filled in.
                if name.text.starts_with("secure.password_hash") {
                    let p = self.expr1(&args[0]);
                    let mut at = 1;
                    let mut part = |t: &mut Self, word: &str, low: i64, high: i64, default: i64| {
                        if !name.text.contains(word) {
                            return t.constant(default);
                        }
                        let v = t.expr1(&args[at]);
                        let loc = t.loc(args[at].span);
                        at += 1;
                        let (below, above) = (t.with(Op::Lt, v, low), t.with(Op::Gt, v, high));
                        t.emit(Inst::FaultIf(below, Fault::PasswordSettings, loc));
                        t.emit(Inst::FaultIf(above, Fault::PasswordSettings, loc));
                        v
                    };
                    let mb = part(self, " memory(", 8, 1024, 19);
                    let passes = part(self, " passes(", 1, 16, 2);
                    let t = self.int_local();
                    self.emit(Inst::Str(t, StrOp::PasswordHash, vec![p, mb, passes]));
                    self.track(&[t], &Type::String);
                    return vec![t];
                }
                if let n @ ("secure.uuid" | "secure.key" | "secure.password_matches(…) hash(…)" | "ui.typed" | "ui.drawn" | "ui.draw" | "ui.act" | "ui.put" | "ui.room") = name.text.as_str() {
                    let vals: Vec<Local> = args.iter().map(|a| self.expr1(a)).collect();
                    let t = self.int_local();
                    let op = match n {
                        "secure.uuid" => StrOp::Uuid,
                        "secure.key" => StrOp::SecureKey,
                        "ui.typed" => StrOp::UiTyped,
                        "ui.drawn" => StrOp::UiDrawn,
                        "ui.draw" => StrOp::UiDraw,
                        "ui.act" => StrOp::UiAct,
                        "ui.put" => StrOp::UiPlace,
                        "ui.room" => StrOp::UiRoom,
                        _ => StrOp::PasswordMatches,
                    };
                    self.emit(Inst::Str(t, op, vals));
                    if matches!(n, "ui.draw" | "ui.act" | "ui.put") {
                        return vec![];
                    }
                    if !matches!(n, "secure.password_matches(…) hash(…)" | "ui.room") {
                        self.track(&[t], &Type::String);
                    }
                    return vec![t];
                }
                if name.text == "secure.encrypt(…) key(…)" {
                    let vals: Vec<Local> = args.iter().map(|a| self.expr1(a)).collect();
                    let (t, zero, bad) = (self.int_local(), self.constant(0), self.int_local());
                    self.emit(Inst::Str(t, StrOp::Encrypt, vals));
                    self.emit(Inst::Binary(bad, Op::Eq, t, zero, NOWHERE));
                    let loc = self.loc(args[1].span);
                    self.emit(Inst::FaultIf(bad, Fault::BadKey, loc));
                    self.track(&[t], &Type::String);
                    return vec![t];
                }
                if name.text == "secure.decrypt(…) key(…)" {
                    let vals: Vec<Local> = args.iter().map(|a| self.expr1(a)).collect();
                    let (t, zero, present) = (self.int_local(), self.constant(0), self.int_local());
                    self.emit(Inst::Str(t, StrOp::Decrypt, vals));
                    self.emit(Inst::Binary(present, Op::Ne, t, zero, NOWHERE));
                    let v = vec![present, t];
                    self.track(&v, &self.ty(e));
                    return v;
                }
                // `random(1..=6)`: the end made exclusive, then the range must hold a number.
                if name.text == "random" || name.text == "secure.random" {
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
                    let op = if name.text == "random" { StrOp::Random } else { StrOp::SecureRandom };
                    self.emit(Inst::Str(n, op, vec![from, to]));
                    return vec![n];
                }
                if let n @ ("files.exists" | "files.is_folder") = name.text.as_str() {
                    let (p, d) = (self.expr1(&args[0]), self.int_local());
                    self.emit(Inst::Str(d, if n == "files.exists" { StrOp::Exists } else { StrOp::IsFolder }, vec![p]));
                    return vec![d];
                }
                if self.info.externs.contains_key(&name.text) {
                    return self.extern_call(&name.text, args, name.span, &self.ty(e));
                }
                if let Some(level) = name.text.strip_prefix("log.").and_then(|l| crate::shared::LEVELS.iter().position(|n| *n == l)) {
                    let (level, text, done) = (self.constant(level as i64), self.expr1(&args[0]), self.int_local());
                    self.emit(Inst::Str(done, StrOp::Log, vec![level, text]));
                    return vec![];
                }
                if name.text == "log.level" {
                    let (level, done) = (self.expr1(&args[0]), self.int_local());
                    self.emit(Inst::Str(done, StrOp::LogLevel, vec![level]));
                    return vec![];
                }
                if name.text == "system.env" {
                    let (n, value) = (self.expr1(&args[0]), self.int_local());
                    self.emit(Inst::Str(value, StrOp::Env, vec![n]));
                    let (zero, present) = (self.constant(0), self.int_local());
                    self.emit(Inst::Binary(present, Op::Ne, value, zero, NOWHERE));
                    let v = vec![present, value];
                    self.track(&v, &self.ty(e));
                    return v;
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
            ExprKind::Static { ty, args, .. } if matches!(ty.text.as_str(), "Date" | "DateTime") && args.is_empty() => {
                let now = self.int_local();
                self.emit(Inst::Str(now, StrOp::LocalNow, vec![]));
                if ty.text == "DateTime" {
                    return vec![now];
                }
                // `Date.today`: the days, counted from 0000-03-01 so that the division rounds down.
                let s = self.with(Op::Plus, now, 719468 * 86400);
                let z = self.with(Op::Div, s, 86400);
                vec![self.with(Op::Minus, z, 719468)]
            }
            // `Answer.text("Hello")`, `Answer.json(book)`, `Answer.status(404)`, `Answer.file(path)`.
            ExprKind::Static { ty, name, args, .. } if ty.text == "web.Answer" => {
                let (status, empty) = (self.int_local(), self.text(""));
                self.emit(Inst::Const(status, 200));
                let text = "text/plain; charset=utf-8";
                let v = match name.text.as_str() {
                    "text" => {
                        let body = self.owned(&args[0]);
                        self.answer_made(status, Some(text), body[0], empty)
                    }
                    "json" => {
                        let (value, t) = (self.expr(&args[0]), self.ty(&args[0]));
                        let body = self.json_text(&value, &t);
                        self.answer_made(status, Some("application/json"), body, empty)
                    }
                    "status" => {
                        let n = self.expr1(&args[0]);
                        self.emit(Inst::Copy(status, n));
                        let body = self.int_local();
                        self.emit(Inst::Str(body, StrOp::StatusText, vec![n]));
                        self.answer_made(status, Some(text), body, empty)
                    }
                    _ => {
                        self.emit(Inst::Free(empty));
                        let path = self.expr1(&args[0]);
                        self.file_answer(path)
                    }
                };
                self.track(&v, &crate::types::answer());
                v
            }
            // `Request.put(url)`: no header, no body, `DEFAULT_SECONDS`, redirections followed.
            ExprKind::Static { ty, name, args, .. } if ty.text == "web.Request" => {
                let url = self.owned(&args[0]);
                let (method, headers) = (self.text(&name.text.to_uppercase()), self.int_local());
                self.emit(Inst::ListNew(headers, 2));
                let (body, seconds, follow) = (self.text(""), self.float_constant(DEFAULT_SECONDS), self.constant(1));
                let v = vec![method, url[0], headers, body, seconds, follow];
                self.track(&v, &crate::types::request());
                v
            }
            ExprKind::Static { ty, name, args, .. } if ty.text == "String" && name.text.starts_with("from_base64") => {
                let (code, t) = (self.expr1(&args[0]), self.int_local());
                let op = if name.text == "from_base64" { StrOp::FromBase64 } else { StrOp::FromBase64Url };
                self.emit(Inst::Str(t, op, vec![code]));
                let (zero, present) = (self.constant(0), self.int_local());
                self.emit(Inst::Binary(present, Op::Ne, t, zero, NOWHERE));
                let v = vec![present, t];
                self.track(&v, &self.ty(e));
                v
            }
            ExprKind::Static { name, args, .. } if name.text == "from_csv" && matches!(self.ty(e), Type::Result(..)) => {
                let text = self.expr1(&args[0]);
                let ty = self.ty(e);
                self.from_csv(text, &ty)
            }
            ExprKind::Static { name, args, .. } if name.text == "from_json" && matches!(self.ty(e), Type::Result(..)) => {
                let text = self.expr1(&args[0]);
                let ty = self.ty(e);
                self.from_json(text, &ty)
            }
            ExprKind::Static { ty, name, args, .. } if matches!(ty.text.as_str(), "Date" | "DateTime") => {
                let vals: Vec<Local> = args.iter().map(|a| self.expr1(a)).collect();
                let loc = self.loc(name.span);
                vec![self.date_made(&vals, loc)]
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
            // The number of the real type, first in the box, compared with the number of the type tested.
            ExprKind::Is(x, _) => {
                let Type::Any(i) = self.ty(x) else { unreachable!("any checked by the type checker") };
                let ty = &self.info.tested[&e.id];
                let b = self.expr1(x);
                let same = self.int_local();
                match self.info.impls.get(&i).and_then(|ts| ts.iter().position(|t| t == ty)) {
                    Some(k) => {
                        let (zero, tag, number) = (self.constant(0), self.int_local(), self.constant(k as i64));
                        self.emit(Inst::ListLoad(tag, b, zero, 0));
                        self.emit(Inst::Binary(same, Op::Eq, tag, number, NOWHERE));
                    }
                    // Never put into an `any` of this interface anywhere in the program.
                    None => self.emit(Inst::Const(same, 0)),
                }
                vec![same]
            }
            // On `none` (or `err`), leaves the function with `none` (or the same `err`);
            // otherwise the value, owned by the expression.
            // `x else y`: the value inside is copied, `x` is only read, as `if x != none` would.
            ExprKind::Else(x, fallback) => {
                let ty = self.ty(e);
                let out = self.new_slots(&ty);
                let v = self.expr(x);
                let (present, empty, done) = (self.new_block(), self.new_block(), self.new_block());
                self.go(End::Branch(v[0], present, empty), present);
                let skip = if let Type::Result(_, err) = self.ty(x) { scalars(&err).len() } else { 0 };
                let copy = self.clone_value(&v[1 + skip..], &ty);
                self.copy(&out, &copy);
                self.go(End::Jump(done), empty);
                let other = self.owned(fallback);
                self.copy(&out, &other);
                self.go(End::Jump(done), done);
                self.track(&out, &ty);
                out
            }
            // In `main` or a `test`, with nowhere to go up: the program stops here, saying why.
            ExprKind::Try(x) if self.ret == Type::Unit => {
                let v = self.owned(x);
                let (empty, present) = (self.new_block(), self.new_block());
                self.go(End::Branch(v[0], present, empty), empty);
                let (mark, skip) = (self.temps.len(), if let Type::Result(_, err) = self.ty(x) { scalars(&err).len() } else { 0 });
                let mut m = vec![Piece::Label("error_prefix")];
                match self.ty(x) {
                    Type::Result(_, err) => self.value_pieces(&v[1..=skip], &err, &mut m),
                    _ => m.push(Piece::Label("try_none")),
                }
                let l = self.loc(e.span);
                let place = format!(":{}:{}\n", l.line, l.col);
                m.extend([Piece::Text("\n  --> ".into()), Piece::File(l.file), Piece::Text(place)]);
                self.temps.truncate(mark);
                self.emit(Inst::Fail(m));
                self.go(End::Unreachable, present);
                let value = v[1 + skip..].to_vec();
                self.track(&value, &self.ty(e));
                value
            }
            ExprKind::Try(x) => {
                let v = self.owned(x);
                let (empty, present) = (self.new_block(), self.new_block());
                self.go(End::Branch(v[0], present, empty), empty);
                let nothing = self.new_slots(&self.ret.clone());
                nothing.iter().for_each(|&l| self.emit(Inst::Const(l, 0)));
                // The error's slots come right after the tag in both results.
                let skip = match (&self.ty(x), self.info.converts.get(&e.id)) {
                    (Type::Result(_, err), None) => {
                        let n = scalars(err).len();
                        for k in 1..=n {
                            self.emit(Inst::Copy(nothing[k], v[k]));
                        }
                        n
                    }
                    // Put into case `i` of the function's error enum: its tag, then the error in the case's slots.
                    (Type::Result(_, err), Some(&i)) => {
                        let n = scalars(err).len();
                        let Type::Result(_, into) = self.ret.clone() else { unreachable!("result checked by the type checker") };
                        let Type::Enum(d) = &*into else { unreachable!("enum checked by the type checker") };
                        let held = self.boxing(v[1..=n].to_vec(), &laid_out(&d.name, &d.variants()[i].1[0].1));
                        let (start, _) = variant_slice(d, i);
                        self.emit(Inst::Const(nothing[1], i as i64));
                        for (k, &l) in held.iter().enumerate() {
                            self.emit(Inst::Copy(nothing[1 + start + k], l));
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
                let mut fields: Slots = vec![];
                for (a, (_, t)) in args.iter().zip(&d.variants()[i].1) {
                    let v = self.owned(a);
                    fields.extend(self.boxing(v, &laid_out(&d.name, t)));
                }
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
                if !matches!(self.ty(a), Type::Int | Type::Float | Type::Bool | Type::Char | Type::Date | Type::DateTime | Type::Pointer) =>
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
                if matches!(self.ty(a), Type::String | Type::BigInt | Type::Struct(_) | Type::Enum(_)) =>
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
            ExprKind::Binary(BinOp::Plus, a, b) if matches!(self.ty(a), Type::List(_)) => {
                let ty = self.ty(a);
                let Type::List(elem) = &ty else { unreachable!() };
                let (va, vb) = (self.expr1(a), self.expr1(b));
                let dst = self.clone_value(&[va], &ty)[0];
                self.add_all(dst, vb, elem);
                self.temps.push((dst, ty.clone()));
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
            ExprKind::Method { receiver, name, args } if args.is_empty() && self.ty(receiver).is_field(&name.text) => {
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
            // `r.header("Accept") is("text/plain")`, `r.body(…)`, `r.seconds(…)`, `r.no_redirections`:
            // a copy of `r`, changed.
            // `q.path`, `q.query("author")`, `q.header("Accept")`: what a question received says.
            ExprKind::Method { receiver, name, args } if self.ty(receiver) == crate::types::request() && matches!(name.text.as_str(), "path" | "query" | "header") => {
                let r = self.expr(receiver);
                match name.text.as_str() {
                    "path" => {
                        let path = self.int_local();
                        self.emit(Inst::Str(path, StrOp::UrlPath, vec![r[1]]));
                        self.track(&[path], &Type::String);
                        vec![path]
                    }
                    "query" => {
                        let (given, value, zero) = (self.expr1(&args[0]), self.int_local(), self.constant(0));
                        self.emit(Inst::Str(value, StrOp::UrlQuery, vec![r[1], given]));
                        let present = self.int_local();
                        self.emit(Inst::Binary(present, Op::Ne, value, zero, NOWHERE));
                        let v = vec![present, value];
                        self.track(&v, &Type::Optional(Box::new(Type::String)));
                        v
                    }
                    _ => {
                        let given = self.expr1(&args[0]);
                        let key = self.int_local();
                        self.emit(Inst::Str(key, StrOp::ToLower, vec![given]));
                        self.track(&[key], &Type::String);
                        self.lookup(r[2], &[key], &Type::String, &Type::String, false)
                    }
                }
            }
            // `a.header("Content-Type") is("text/html")`: a copy of `a`, the name in small letters
            // (so that it replaces Kova's own `content-type`).
            ExprKind::Method { receiver, args, .. } if self.ty(receiver) == crate::types::answer() => {
                let ty = crate::types::answer();
                let a = self.expr(receiver);
                let v = self.clone_value(&a, &ty);
                let (given, value) = (self.expr1(&args[0]), self.owned(&args[1]));
                let key = self.int_local();
                self.emit(Inst::Str(key, StrOp::ToLower, vec![given]));
                self.track(&[key], &Type::String);
                let place = self.entry(v[1], &[key], &Type::String, &Type::String);
                let old = self.read(&place);
                self.free_value(&old, &Type::String);
                self.write(&place, &value);
                self.track(&v, &ty);
                v
            }
            ExprKind::Method { receiver, name, args } if self.ty(receiver) == crate::types::request() => {
                let ty = crate::types::request();
                let r = self.expr(receiver);
                let v = self.clone_value(&r, &ty);
                match name.text.as_str() {
                    "header(…) is(…)" => {
                        let (key, value) = (self.expr(&args[0]), self.owned(&args[1]));
                        let place = self.entry(v[2], &key, &Type::String, &Type::String);
                        let old = self.read(&place);
                        self.free_value(&old, &Type::String);
                        self.write(&place, &value);
                    }
                    "body" => {
                        let body = self.owned(&args[0]);
                        self.emit(Inst::Free(v[3]));
                        self.emit(Inst::Copy(v[3], body[0]));
                    }
                    "seconds" => {
                        let seconds = self.expr1(&args[0]);
                        self.emit(Inst::Copy(v[4], seconds));
                    }
                    _ => self.emit(Inst::Const(v[5], 0)),
                }
                self.track(&v, &ty);
                v
            }
            // `books.to_csv`, `books.to_csv separator(';')`.
            ExprKind::Method { receiver, name, args } if name.text.starts_with("to_csv") && matches!(self.ty(receiver), Type::List(_)) => {
                let Type::List(elem) = self.ty(receiver) else { unreachable!() };
                let l = self.expr1(receiver);
                let sep = match args.first() {
                    Some(a) => self.expr1(a),
                    None => self.constant(',' as i64),
                };
                let (spec, text) = (self.text(&csv_spec(&elem)), self.int_local());
                self.emit(Inst::Str(text, StrOp::CsvWrite, vec![l, spec, sep]));
                self.emit(Inst::Free(spec));
                self.track(&[text], &Type::String);
                vec![text]
            }
            // `book.to_json` (PLAN Part 4 "JSON").
            ExprKind::Method { receiver, name, args } if name.text == "to_json" && args.is_empty() && matches!(self.ty(receiver), Type::Struct(_) | Type::List(_) | Type::Map(..)) => {
                let ty = self.ty(receiver);
                let v = self.expr(receiver);
                let text = self.json_text(&v, &ty);
                self.track(&[text], &Type::String);
                vec![text]
            }
            // `r.header("Content-Type")`: looked up in small letters in `r.headers`.
            ExprKind::Method { receiver, name, args } if name.text == "header" && self.ty(receiver) == crate::types::response() => {
                let r = self.expr(receiver);
                let given = self.expr1(&args[0]);
                let key = self.int_local();
                self.emit(Inst::Str(key, StrOp::ToLower, vec![given]));
                self.track(&[key], &Type::String);
                self.lookup(r[3], &[key], &Type::String, &Type::String, false)
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
                self.call_on(&ty, &name.text, vals, places, name.span, &result)
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
                    "is_empty" => self.is_empty(map),
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
                    "is_empty" => self.is_empty(set),
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
                    "each workers(…)" | "map workers(…)" => self.parallel(e, list, &elem, &name.text, args),
                    "sorted" => self.sorted(list, &elem, args.last(), e.span),
                    "shuffled" => self.shuffled(list, &elem),
                    "chosen" => self.chosen(list, &elem),
                    "first" => self.element_or_none(list, &elem, |t, _| t.constant(0)),
                    "last" => self.element_or_none(list, &elem, |t, n| {
                        let (one, i) = (t.constant(1), t.int_local());
                        t.emit(Inst::Binary(i, Op::Minus, n, one, NOWHERE));
                        i
                    }),
                    "min" | "max" => self.extreme(list, &elem, name.text == "max", e.span),
                    "is_empty" => self.is_empty(list),
                    "contains" => {
                        let x = self.expr(&args[0]);
                        let found = self.int_local();
                        self.emit(Inst::Const(found, 0));
                        self.each_element(list, &elem, |t, _, v| {
                            let same = t.equal(&v, &x, &elem, e.span);
                            let (yes, next) = (t.new_block(), t.new_block());
                            t.go(End::Branch(same, yes, next), yes);
                            t.emit(Inst::Const(found, 1));
                            t.go(End::Jump(next), next);
                        });
                        vec![found]
                    }
                    "sum" => {
                        let (total, zero, loc) = (self.new_slots(&elem)[0], self.constant(0), self.loc(name.span));
                        let float = *elem == Type::Float;
                        self.emit(if float { Inst::Convert(total, Conversion::FromBits, zero, NOWHERE) } else { Inst::Copy(total, zero) });
                        self.each_element(list, &elem, |t, _, v| {
                            t.emit(if float { Inst::FBinary(total, Op::Plus, total, v[0], loc) } else { Inst::Binary(total, Op::Plus, total, v[0], loc) });
                        });
                        vec![total]
                    }
                    "reversed" => {
                        let (ty, out, n, one) = (self.ty(e), self.int_local(), self.int_local(), self.constant(1));
                        self.emit(Inst::ListNew(out, scalars(&elem).len()));
                        self.emit(Inst::ListLength(n, list));
                        self.times(n, |t, i| {
                            let (last, j) = (t.int_local(), t.int_local());
                            t.emit(Inst::Binary(last, Op::Minus, n, one, NOWHERE));
                            t.emit(Inst::Binary(j, Op::Minus, last, i, NOWHERE));
                            let v = t.load(list, j, &elem);
                            let copy = t.clone_value(&v, &elem);
                            t.push(out, &copy);
                        });
                        self.track(&[out], &ty);
                        vec![out]
                    }
                    "mean" | "median" => {
                        let (n, zero) = (self.int_local(), self.constant(0));
                        self.emit(Inst::ListLength(n, list));
                        let (empty, loc) = (self.int_local(), self.loc(name.span));
                        self.emit(Inst::Binary(empty, Op::Eq, n, zero, NOWHERE));
                        self.emit(Inst::FaultIf(empty, Fault::NoElements, loc));
                        let (float, bits) = (self.constant((*elem == Type::Float) as i64), self.int_local());
                        self.emit(Inst::Str(bits, if name.text == "mean" { StrOp::Mean } else { StrOp::Median }, vec![list, float]));
                        let x = self.new_slots(&Type::Float)[0];
                        self.emit(Inst::Convert(x, Conversion::FromBits, bits, NOWHERE));
                        vec![x]
                    }
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
                if let ty @ (Type::Date | Type::DateTime) = self.ty(receiver) {
                    let arg = args.first().map(|a| self.expr1(a));
                    let loc = self.loc(name.span);
                    return vec![self.date_method(v, &ty, &name.text, arg, loc)];
                }
                if self.ty(receiver) == Type::String && name.text == "is_empty" {
                    let (n, zero) = (self.int_local(), self.constant(0));
                    self.emit(Inst::Str(n, StrOp::Length, vec![v]));
                    return vec![self.compare(Op::Eq, n, zero)];
                }
                let string = |op| match op {
                    "length" => StrOp::Length,
                    _ => StrOp::Clone,
                };
                if self.ty(receiver) == Type::String
                    && let Some(op) = string_op(&name.text)
                {
                    let mut vals = vec![v];
                    vals.extend(args.iter().map(|a| self.expr1(a)));
                    if let Some((_, code)) = crate::shared::PAINTS.iter().find(|(p, _)| *p == name.text) {
                        vals.push(self.constant(*code));
                    }
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
                // `s.groups(pattern)`: `none` if nothing matches.
                if self.ty(receiver) == Type::String && name.text == "groups" {
                    let p = self.expr1(&args[0]);
                    let (l, zero, present) = (self.int_local(), self.constant(0), self.int_local());
                    self.emit(Inst::Str(l, StrOp::Groups, vec![v, p]));
                    self.emit(Inst::Binary(present, Op::Ne, l, zero, NOWHERE));
                    let v = vec![present, l];
                    self.track(&v, &self.ty(e));
                    return v;
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
                if self.ty(receiver) == Type::String && matches!(name.text.as_str(), "to_date" | "to_datetime") {
                    let op = if name.text == "to_date" { StrOp::ToDate } else { StrOp::ToDateTime };
                    let (at, wrong, present) = (self.int_local(), self.constant(i64::MIN), self.int_local());
                    self.emit(Inst::Str(at, op, vec![v]));
                    self.emit(Inst::Binary(present, Op::Ne, at, wrong, NOWHERE));
                    return vec![present, at];
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
pub fn names_in(e: &Expr, names: &mut Vec<String>) {
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
        ExprKind::Try(x) | ExprKind::Outcome { value: x, .. } | ExprKind::Is(x, _) => names_in(x, names),
        ExprKind::Else(x, y) => {
            names_in(x, names);
            names_in(y, names);
        }
        ExprKind::Method { receiver, args, .. } => {
            names_in(receiver, names);
            args.iter().for_each(|a| names_in(a, names));
        }
        ExprKind::Unary(_, x) | ExprKind::Field(x, _) | ExprKind::Mark(_, x) => names_in(x, names),
        ExprKind::List(elems) | ExprKind::SetLit(elems) => elems.iter().for_each(|a| names_in(a, names)),
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
        ExprKind::Int(_) | ExprKind::BigInt(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::None => {}
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
