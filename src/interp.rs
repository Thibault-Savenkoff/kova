// Interpreter (`kova run`): executes the KIR directly, instruction by instruction.

use std::io::{BufRead, BufWriter, Write};

use crate::debug::Debugger;

use crate::kir::*;
use crate::locale::{self, t};

/// What a String or List slot points to.
enum Object {
    Str(String),
    /// `length` elements, their slots one after the other (`width` slots each). A Map or a Set
    /// also has an `index` (a List, made by `kir.rs` `find`; 0 = none), like in runtime.c.
    List { width: usize, length: usize, slots: Vec<i64>, index: i64 },
}

/// The Strings and Lists of the running program; their slot holds their number here (0 = none).
#[derive(Default)]
struct Heap {
    objects: Vec<Option<Object>>,
    /// Numbers of freed Strings, to reuse.
    free: Vec<usize>,
    allocated: usize,
    live: usize,
    /// The program's command-line arguments, for `args`.
    args: Vec<String>,
    /// Under `kova debug`: what stops the program and shows its variables.
    debugger: Option<Box<Debugger>>,
    /// While the debugger shows a variable: a String or a List already given away (slot 0) or
    /// freed was read (`broken`), as an empty one (`nothing`).
    looking: bool,
    broken: std::cell::Cell<bool>,
    nothing: (usize, Vec<i64>),
}

impl Heap {
    fn make(&mut self, s: String) -> i64 {
        self.add(Object::Str(s))
    }
    fn add(&mut self, o: Object) -> i64 {
        self.allocated += 1;
        self.live += 1;
        if self.objects.is_empty() {
            self.objects.push(None); // number 0 = nothing
        }
        match self.free.pop() {
            Some(i) => {
                self.objects[i] = Some(o);
                i as i64
            }
            None => {
                self.objects.push(Some(o));
                self.objects.len() as i64 - 1
            }
        }
    }
    fn get(&self, h: i64) -> &str {
        match self.objects.get(h as usize) {
            Some(Some(Object::Str(s))) => s,
            _ if self.looking => {
                self.broken.set(true);
                ""
            }
            _ => panic!("String used after being freed"),
        }
    }
    /// The List `h`: (width of an element, its length, its slots).
    fn list(&mut self, h: i64) -> (usize, &mut usize, &mut Vec<i64>) {
        if self.looking && !matches!(self.objects.get(h as usize), Some(Some(Object::List { .. }))) {
            self.broken.set(true);
            self.nothing = (0, vec![]);
            return (1, &mut self.nothing.0, &mut self.nothing.1);
        }
        match &mut self.objects[h as usize] {
            Some(Object::List { width, length, slots, .. }) => (*width, length, slots),
            _ => panic!("List used after being freed"),
        }
    }
    fn index(&mut self, h: i64) -> &mut i64 {
        match &mut self.objects[h as usize] {
            Some(Object::List { index, .. }) => index,
            _ => panic!("List used after being freed"),
        }
    }
    fn release(&mut self, h: i64) {
        if h != 0 {
            if let Object::List { index, .. } = self.objects[h as usize].take().expect("freed twice") {
                self.release(index);
            }
            self.free.push(h as usize);
            self.live -= 1;
        }
    }
}

/// Element `i` of a Map or a Set goes out of its index (`s`, its slots), and the positions
/// after it move back by one: the same as `unindex` in runtime.c (comments there).
fn unindex(s: &mut [i64], i: i64) {
    let cap = s.len() as i64 / 2 - 1;
    if i >= s[0] {
        return;
    }
    let mut j = 0;
    for k in 1..=cap as usize {
        if s[2 * k + 1] == i + 1 {
            j = k as i64;
        }
        if s[2 * k + 1] > i + 1 {
            s[2 * k + 1] -= 1;
        }
    }
    s[0] -= 1;
    let mut k = j % cap + 1;
    while s[2 * k as usize + 1] != 0 {
        let home = s[2 * k as usize] % cap + 1;
        let stays = if j < k { j < home && home <= k } else { j < home || home <= k };
        if !stays {
            s[2 * j as usize] = s[2 * k as usize];
            s[2 * j as usize + 1] = s[2 * k as usize + 1];
            j = k;
        }
        k = k % cap + 1;
    }
    s[2 * j as usize] = 0;
    s[2 * j as usize + 1] = 0;
}

/// `Inst::Mix`: the same in src/codegen.rs.
fn mix(a: i64, b: i64) -> i64 {
    // splitmix64's finalizer: every bit counts in the low ones that `% cap` reads (keys like
    // `i * 1024` spread out).
    let mut x = (a ^ b) as u64;
    x = (x ^ x >> 30).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ x >> 27).wrapping_mul(0x94d049bb133111eb);
    ((x ^ x >> 31) >> 1) as i64
}

/// Runs `main`. On a runtime error: prints the message and returns code 101.
/// With `KOVA_LEAKS` set (the test runner sets it): at the end, a String never freed
/// is an error (code 102); `KOVA_LEAKS=count` also prints how many were made.
/// `args` = what `args` gives the program.
pub fn run(p: &Program, files: &[String], args: &[String]) -> i32 {
    let mut out = BufWriter::new(std::io::stdout());
    let mut heap = Heap { args: args.to_vec(), ..Heap::default() };
    let result = call(p, p.main.expect("checked by main.rs"), &[], &mut out, files, &mut heap);
    out.flush().unwrap();
    match result {
        Ok(_) => leaks(heap.allocated, heap.live),
        Err(Stop::Error(message)) => {
            eprint!("{message}");
            101
        }
        // What is still alive is not counted as a leak (`kova_exit` in runtime/runtime.c).
        Err(Stop::Exit(code)) => code,
    }
}

/// `kova debug`: runs `main` with the debugger (`debug.rs`); the program's output goes to it.
/// Returns the exit code (101 after a runtime error), and the debugger back.
pub fn debug(p: &Program, files: &[String], args: &[String], debugger: Debugger) -> (i32, Box<Debugger>) {
    let files = &crate::debug::files(files)[..];
    let mut out = crate::debug::Console::default();
    let mut heap = Heap { args: args.to_vec(), debugger: Some(Box::new(debugger)), ..Heap::default() };
    let result = call(p, p.main.expect("checked by debug.rs"), &[], &mut out, files, &mut heap);
    out.flush().unwrap();
    let code = match result {
        Ok(_) => 0,
        Err(Stop::Error(message)) => {
            crate::debug::output("stderr", &message);
            101
        }
        Err(Stop::Exit(code)) => code,
    };
    (code, heap.debugger.expect("given back after each stop"))
}

/// Under the debugger, at a statement (`Inst::Line` number `at` of block `block`): notes where the
/// function is and its variables' values, and stops there if it must (a breakpoint, a step).
fn line(p: &Program, num: usize, at: (usize, usize), loc: Loc, locals: &[i64], out: &mut dyn Write, files: &[String], heap: &mut Heap) {
    let Some(d) = &mut heap.debugger else { return };
    if let Some(frame) = d.frames.last_mut() {
        frame.at = Some((num, at.0, at.1, loc));
    }
    snapshot(p, locals, d);
    if let Some(reason) = d.stops(loc, files) {
        pause(p, out, files, heap, reason, None);
    }
}

/// Copies the current values of the variables of the innermost function, for the debugger.
fn snapshot(p: &Program, locals: &[i64], d: &mut Debugger) {
    let Some(frame) = d.frames.last_mut() else { return };
    frame.values = watches(p, frame.at).iter().flat_map(|w| w.slots.iter().map(|&l| locals[l])).collect();
}

/// The variables visible at a statement (`Frame::at`).
fn watches(p: &Program, at: Option<(usize, usize, usize, Loc)>) -> &[Watch] {
    match at.map(|(f, b, i, _)| &p.functions[f].blocks[b].insts[i]) {
        Some(Inst::Line(_, w)) => w,
        _ => &[],
    }
}

/// Stopped: the debugger answers the editor (call stack, variables) until it says to go on.
fn pause(p: &Program, out: &mut dyn Write, files: &[String], heap: &mut Heap, reason: &str, text: Option<&str>) {
    let _ = out.flush();
    let Some(mut d) = heap.debugger.take() else { return };
    d.stopped(reason, text);
    while let Some(frame) = d.variables_asked() {
        let at = d.frames.get(frame).and_then(|f| f.at);
        let values = d.frames.get(frame).map_or(vec![], |f| f.values.clone());
        let mut shown = vec![];
        let mut k = 0;
        for w in watches(p, at) {
            let v = &values[k..k + w.slots.len()];
            k += w.slots.len();
            heap.looking = true;
            heap.broken.set(false);
            let text = match call(p, w.look, v, out, files, heap) {
                Ok(r) => {
                    let t = heap.get(r[0]).to_string();
                    heap.release(r[0]);
                    t
                }
                Err(_) => String::new(),
            };
            let text = if heap.broken.get() { t!("debug_given_away") } else { text };
            heap.looking = false;
            shown.push((w.name.clone(), w.ty.clone(), text));
        }
        d.variables(shown);
    }
    heap.debugger = Some(d);
}

/// Same check as `kova_leaks` in `runtime/runtime.c`.
fn leaks(allocated: usize, live: usize) -> i32 {
    let Ok(mode) = std::env::var("KOVA_LEAKS") else { return 0 };
    let text = |k| locale::text(k, &[]);
    if mode == "count" {
        eprintln!("{}{allocated}", text("leak_allocated"));
    }
    if live == 0 {
        return 0;
    }
    eprintln!("{}{}{live}", text("error_prefix"), text("leak_live"));
    102
}

/// `kova test`: runs every `test` block, even if another one failed. Code 1 if at least one fails.
/// `only`: the name of the one test to run.
pub fn test(p: &Program, files: &[String], only: Option<&str>) -> i32 {
    if p.tests.is_empty() {
        println!("{}", t!("tests_none", file = files[0]));
        return 0;
    }
    let tests: Vec<_> = p.tests.iter().filter(|(name, _)| only.is_none_or(|o| o == name)).collect();
    if let (Some(name), true) = (only, tests.is_empty()) {
        eprintln!("{}", t!("test_unknown", name = name, file = files[0]));
        return 1;
    }
    let mut out = BufWriter::new(std::io::stdout());
    let mut failed = 0;
    for (name, num) in &tests {
        let result = call(p, *num, &[], &mut out, files, &mut Heap::default());
        let _ = match result {
            Ok(_) => writeln!(out, "✅ {name}"),
            // `exit` stops every test, like it stops a program.
            Err(Stop::Exit(code)) => {
                out.flush().unwrap();
                return code;
            }
            Err(Stop::Error(message)) => {
                failed += 1;
                let message: Vec<String> = message.lines().map(|l| format!("   {l}")).collect();
                writeln!(out, "❌ {name}\n{}", message.join("\n"))
            }
        };
    }
    let total = tests.len();
    let _ = writeln!(out, "\n{}", t!("tests_summary", passed = total - failed, failed = failed, total = total));
    out.flush().unwrap();
    if failed > 0 { 1 } else { 0 }
}

type Outcome = Result<Vec<i64>, Stop>;

/// Why a function stopped before its end.
enum Stop {
    /// A runtime error: its message, as it will be printed (code 101).
    Error(String),
    /// `exit(code)`.
    Exit(i32),
}

// ponytail: Rust recursion = the interpreter's stack; infinite recursion
// crashes without a nice message (same in native). To handle with stack errors.
fn call(p: &Program, num: usize, args: &[i64], out: &mut dyn Write, files: &[String], heap: &mut Heap) -> Outcome {
    let Some(d) = &mut heap.debugger else { return execute(p, num, args, out, files, heap) };
    d.enter(&p.functions[num].name);
    let result = execute(p, num, args, out, files, heap);
    // A runtime error: the debugger stops where it happens, with its message.
    if let Err(Stop::Error(message)) = &result
        && heap.debugger.as_mut().is_some_and(|d| d.first_error())
    {
        let message = message.clone();
        pause(p, out, files, heap, "exception", Some(&message));
    }
    if let Some(d) = &mut heap.debugger {
        d.leave();
    }
    result
}

fn execute(p: &Program, num: usize, args: &[i64], out: &mut dyn Write, files: &[String], heap: &mut Heap) -> Outcome {
    let fault = |f: Fault, loc: &Loc| Stop::Error(fault_text(f, files, *loc));
    let f = &p.functions[num];
    // Every slot fits in 64 bits; a Float is stored bit for bit.
    let mut locals = vec![0i64; f.locals.len()];
    locals[..args.len()].copy_from_slice(args);
    let mut block = 0;
    loop {
        let b = &f.blocks[block];
        for (i, inst) in b.insts.iter().enumerate() {
            match inst {
                Inst::Const(d, v) => locals[*d] = *v,
                Inst::Copy(d, s) => locals[*d] = locals[*s],
                Inst::ListIndex(d, l) => locals[*d] = if locals[*l] == 0 { 0 } else { *heap.index(locals[*l]) },
                Inst::ListSetIndex(l, v) => *heap.index(locals[*l]) = locals[*v],
                Inst::Mix(d, a, b) => locals[*d] = mix(locals[*a], locals[*b]),
                Inst::Binary(d, op, a, b, loc) => {
                    locals[*d] = compute(*op, locals[*a], locals[*b]).map_err(|e| fault(e, loc))?;
                }
                Inst::Neg(d, a, loc) => {
                    locals[*d] = locals[*a].checked_neg().ok_or_else(|| fault(Fault::Overflow, loc))?;
                }
                Inst::Not(d, a) => locals[*d] = 1 - locals[*a],
                Inst::FuncRef(d, g) => locals[*d] = *g as i64,
                Inst::Call(..) | Inst::CallIndirect(..) => {
                    let (d, g, a) = match inst {
                        Inst::Call(d, g, a) => (d, *g, a),
                        Inst::CallIndirect(d, g, a) => (d, locals[*g] as usize, a),
                        _ => unreachable!(),
                    };
                    if let Some(dbg) = &mut heap.debugger {
                        snapshot(p, &locals, dbg);
                    }
                    let vals: Vec<i64> = a.iter().map(|&l| locals[l]).collect();
                    let r = call(p, g, &vals, out, files, heap)?;
                    for (&d, v) in d.iter().zip(r) {
                        locals[d] = v;
                    }
                }
                Inst::FBinary(d, op, a, b, loc) => {
                    locals[*d] = compute_float(*op, float(locals[*a]), float(locals[*b])).map_err(|e| fault(e, loc))?;
                }
                Inst::FNeg(d, a) => locals[*d] = bits(-float(locals[*a])),
                Inst::Convert(d, c, a, loc) => {
                    locals[*d] = match c {
                        Conversion::IntToFloat => bits(locals[*a] as f64),
                        Conversion::FloatToInt => {
                            let x = float(locals[*a]);
                            // False for NaN too: any comparison with NaN is false.
                            if !(x >= -9223372036854775808.0 && x < 9223372036854775808.0) {
                                return Err(fault(Fault::BadConversion, loc));
                            }
                            x as i64
                        }
                    }
                }
                Inst::Text(d, s) => locals[*d] = heap.make(s.clone()),
                Inst::Str(d, op, a) => {
                    let s = |k: usize| heap.get(locals[a[k]]);
                    locals[*d] = match op {
                        StrOp::Concat => {
                            let joined = format!("{}{}", s(0), s(1));
                            heap.make(joined)
                        }
                        StrOp::Eq => (s(0) == s(1)) as i64,
                        StrOp::Hash if locals[a[0]] == 0 => 0,
                        // FNV-1a, like `kova_str_hash` in runtime.c.
                        StrOp::Hash => s(0).bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)) as i64,
                        StrOp::Less => (s(0) < s(1)) as i64,
                        StrOp::BigLess => crate::bigint::less(s(0), s(1)) as i64,
                        StrOp::BigAdd => {
                            let v = crate::bigint::add(s(0), s(1));
                            heap.make(v)
                        }
                        StrOp::BigSub => {
                            let v = crate::bigint::sub(s(0), s(1));
                            heap.make(v)
                        }
                        StrOp::BigMul => {
                            let v = crate::bigint::mul(s(0), s(1));
                            heap.make(v)
                        }
                        StrOp::BigDiv => {
                            let v = crate::bigint::div(s(0), s(1));
                            heap.make(v)
                        }
                        StrOp::BigRem => {
                            let v = crate::bigint::rem(s(0), s(1));
                            heap.make(v)
                        }
                        StrOp::Length => s(0).chars().count() as i64,
                        // 0 (no String: a field of another variant of an enum) is copied as 0.
                        StrOp::Clone if locals[a[0]] == 0 => 0,
                        StrOp::Clone => {
                            let copy = s(0).to_string();
                            heap.make(copy)
                        }
                        StrOp::IsInt => crate::kir::parse_int(s(0)).is_some() as i64,
                        StrOp::ToInt => crate::kir::parse_int(s(0)).unwrap_or(0),
                        StrOp::ReadFile => match std::fs::read(s(0)).map(String::from_utf8) {
                            Ok(Ok(text)) => heap.make(text),
                            Ok(Err(_)) => -4,
                            Err(e) => match e.kind() {
                                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => -1,
                                std::io::ErrorKind::PermissionDenied => -2,
                                std::io::ErrorKind::IsADirectory => -3,
                                _ => -5,
                            },
                        },
                        // Under the debugger, the input is the editor's: the program reads nothing.
                        StrOp::ReadLine if heap.debugger.is_some() => 0,
                        StrOp::ReadLine => {
                            out.flush().unwrap();
                            let mut line = vec![];
                            match std::io::stdin().lock().read_until(b'\n', &mut line) {
                                Ok(0) | Err(_) => 0,
                                Ok(_) => {
                                    for end in *b"\n\r" {
                                        if line.last() == Some(&end) {
                                            line.pop();
                                        }
                                    }
                                    String::from_utf8(line).map_or(-1, |t| heap.make(t))
                                }
                            }
                        }
                        StrOp::Format(show) => {
                            let t = text(locals[a[0]], *show, heap);
                            heap.make(t)
                        }
                        // Spaces = the same 5 characters as `is_space` in runtime/runtime.c.
                        StrOp::Trim => {
                            let t = s(0).trim_matches(|c: char| c.is_ascii_whitespace()).to_string();
                            heap.make(t)
                        }
                        StrOp::Contains => s(0).contains(s(1)) as i64,
                        StrOp::StartsWith => s(0).starts_with(s(1)) as i64,
                        StrOp::EndsWith => s(0).ends_with(s(1)) as i64,
                        StrOp::Replace => {
                            let t = s(0).replace(s(1), s(2));
                            heap.make(t)
                        }
                        StrOp::Split | StrOp::Words => {
                            let parts: Vec<String> = match op {
                                StrOp::Split => s(0).split(s(1)).map(String::from).collect(),
                                _ => s(0).split_ascii_whitespace().map(String::from).collect(),
                            };
                            let slots: Vec<i64> = parts.into_iter().map(|p| heap.make(p)).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0 })
                        }
                        StrOp::Args => {
                            let slots: Vec<i64> = heap.args.clone().into_iter().map(|a| heap.make(a)).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0 })
                        }
                        StrOp::Chars => {
                            let slots: Vec<i64> = s(0).chars().map(|c| c as i64).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0 })
                        }
                    }
                }
                Inst::Free(l) | Inst::ListFree(l) => {
                    heap.release(locals[*l]);
                    locals[*l] = 0;
                }
                Inst::ListNew(d, width) => {
                    locals[*d] = heap.add(Object::List { width: *width, length: 0, slots: vec![], index: 0 });
                }
                // 0 = a List emptied by a move: its helper (`Drop`) still asks its length.
                Inst::ListLength(d, l) => locals[*d] = if locals[*l] == 0 { 0 } else { *heap.list(locals[*l]).1 as i64 },
                Inst::FaultIf(c, f, loc) if locals[*c] != 0 => return Err(fault(*f, loc)),
                Inst::FaultIf(..) => {}
                Inst::ListCheck(l, i, loc) => {
                    let length = *heap.list(locals[*l]).1 as i64;
                    if !(0..length).contains(&locals[*i]) {
                        return Err(fault(Fault::Index, loc));
                    }
                }
                Inst::ListLoad(d, l, i, k) => {
                    let (width, _, slots) = heap.list(locals[*l]);
                    locals[*d] = slots[locals[*i] as usize * width + k];
                }
                Inst::ListStore(l, i, k, v) => {
                    let value = locals[*v];
                    let (width, _, slots) = heap.list(locals[*l]);
                    slots[locals[*i] as usize * width + k] = value;
                }
                Inst::ListGrow(d, l) => {
                    let (width, length, slots) = heap.list(locals[*l]);
                    slots.resize(slots.len() + width, 0);
                    *length += 1;
                    locals[*d] = *length as i64 - 1;
                }
                Inst::ListRemove(l, i) => {
                    let index = *heap.index(locals[*l]);
                    if index != 0 {
                        unindex(heap.list(index).2, locals[*i]);
                    }
                    let i = locals[*i] as usize;
                    let (width, length, slots) = heap.list(locals[*l]);
                    slots.drain(i * width..(i + 1) * width);
                    *length -= 1;
                }
                Inst::Print { value, show, newline } => {
                    let _ = write!(out, "{}", text(locals[*value], *show, heap));
                    if *newline {
                        let _ = writeln!(out);
                    }
                }
                Inst::Fail(pieces) => {
                    return Err(Stop::Error(pieces
                        .iter()
                        .map(|m| match m {
                            Piece::Text(t) => t.clone(),
                            Piece::Label(k) => locale::text(k, &[]),
                            Piece::Value(l, Show::Site) => site_text(files, locals[*l]),
                            Piece::Value(l, show) => text(locals[*l], *show, heap),
                            Piece::File(i) => files[*i].clone(),
                        })
                        .collect()));
                }
                Inst::Exit(code) => return Err(Stop::Exit(locals[*code] as i32)),
                Inst::Line(loc, _) => line(p, num, (block, i), *loc, &locals, out, files, heap),
            }
        }
        match &b.end {
            End::Jump(s) => block = *s,
            End::Branch(c, yes, no) => block = if locals[*c] != 0 { *yes } else { *no },
            End::Return(v) => return Ok(v.iter().map(|&l| locals[l]).collect()),
            End::Unreachable => unreachable!("unreachable block reached in `{}`", f.name),
        }
    }
}

fn text(v: i64, show: Show, heap: &Heap) -> String {
    match show {
        Show::String => heap.get(v).to_string(),
        Show::Int => v.to_string(),
        Show::Bool => (v != 0).to_string(),
        Show::Float => float_text(float(v)),
        Show::Char => char::from_u32(v as u32).unwrap().to_string(),
        Show::Site => unreachable!("a site is only shown in an error message"),
    }
}

fn float(v: i64) -> f64 {
    f64::from_bits(v as u64)
}
fn bits(x: f64) -> i64 {
    x.to_bits() as i64
}

/// Float operations; comparisons give 0 or 1.
fn compute_float(op: Op, a: f64, b: f64) -> Result<i64, Fault> {
    Ok(match op {
        Op::Plus => bits(a + b),
        Op::Minus => bits(a - b),
        Op::Times => bits(a * b),
        // `b == 0.0` is true for -0.0 too.
        Op::Div if b == 0.0 => return Err(Fault::DivisionByZero),
        Op::Div => bits(a / b),
        Op::Eq => (a == b) as i64,
        Op::Ne => (a != b) as i64,
        Op::Lt => (a < b) as i64,
        Op::Le => (a <= b) as i64,
        Op::Gt => (a > b) as i64,
        Op::Ge => (a >= b) as i64,
        Op::Rem => unreachable!("`%` refused on Floats by the type checker"),
    })
}

/// Integer operations, with the same rules as native code (`codegen.rs`).
fn compute(op: Op, a: i64, b: i64) -> Result<i64, Fault> {
    let overflow = Fault::Overflow;
    Ok(match op {
        Op::Plus => a.checked_add(b).ok_or(overflow)?,
        Op::Minus => a.checked_sub(b).ok_or(overflow)?,
        Op::Times => a.checked_mul(b).ok_or(overflow)?,
        Op::Div if b == 0 => return Err(Fault::DivisionByZero),
        Op::Div => a.checked_div(b).ok_or(overflow)?,
        Op::Rem if b == 0 => return Err(Fault::DivisionByZero),
        // -9223372036854775808 % -1 is 0 (mathematically right, not an error).
        Op::Rem => a.wrapping_rem(b),
        Op::Eq => (a == b) as i64,
        Op::Ne => (a != b) as i64,
        Op::Lt => (a < b) as i64,
        Op::Le => (a <= b) as i64,
        Op::Gt => (a > b) as i64,
        Op::Ge => (a >= b) as i64,
    })
}
