// Interpreter (`kova run`): executes the KIR directly, instruction by instruction.

use std::io::{BufRead, BufWriter, Stdout, Write};

use crate::kir::*;
use crate::locale::{self, t};

/// What a String or List slot points to.
enum Object {
    Str(String),
    /// `length` elements, their slots one after the other (`width` slots each).
    List { width: usize, length: usize, slots: Vec<i64> },
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
        match &self.objects[h as usize] {
            Some(Object::Str(s)) => s,
            _ => panic!("String used after being freed"),
        }
    }
    /// The List `h`: (width of an element, its length, its slots).
    fn list(&mut self, h: i64) -> (usize, &mut usize, &mut Vec<i64>) {
        match &mut self.objects[h as usize] {
            Some(Object::List { width, length, slots }) => (*width, length, slots),
            _ => panic!("List used after being freed"),
        }
    }
    fn release(&mut self, h: i64) {
        if h != 0 {
            self.objects[h as usize].take().expect("freed twice");
            self.free.push(h as usize);
            self.live -= 1;
        }
    }
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
pub fn test(p: &Program, files: &[String]) -> i32 {
    if p.tests.is_empty() {
        println!("{}", t!("tests_none", file = files[0]));
        return 0;
    }
    let mut out = BufWriter::new(std::io::stdout());
    let mut failed = 0;
    for (name, num) in &p.tests {
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
    let total = p.tests.len();
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
fn call(p: &Program, num: usize, args: &[i64], out: &mut BufWriter<Stdout>, files: &[String], heap: &mut Heap) -> Outcome {
    let fault = |f: Fault, loc: &Loc| Stop::Error(fault_text(f, files, *loc));
    let f = &p.functions[num];
    // Every slot fits in 64 bits; a Float is stored bit for bit.
    let mut locals = vec![0i64; f.locals.len()];
    locals[..args.len()].copy_from_slice(args);
    let mut block = 0;
    loop {
        let b = &f.blocks[block];
        for inst in &b.insts {
            match inst {
                Inst::Const(d, v) => locals[*d] = *v,
                Inst::Copy(d, s) => locals[*d] = locals[*s],
                Inst::Binary(d, op, a, b, loc) => {
                    locals[*d] = compute(*op, locals[*a], locals[*b]).map_err(|e| fault(e, loc))?;
                }
                Inst::Neg(d, a, loc) => {
                    locals[*d] = locals[*a].checked_neg().ok_or_else(|| fault(Fault::Overflow, loc))?;
                }
                Inst::Not(d, a) => locals[*d] = 1 - locals[*a],
                Inst::Call(d, g, a) => {
                    let vals: Vec<i64> = a.iter().map(|&l| locals[l]).collect();
                    let r = call(p, *g, &vals, out, files, heap)?;
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
                        StrOp::Less => (s(0) < s(1)) as i64,
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
                            heap.add(Object::List { width: 1, length: slots.len(), slots })
                        }
                        StrOp::Args => {
                            let slots: Vec<i64> = heap.args.clone().into_iter().map(|a| heap.make(a)).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots })
                        }
                        StrOp::Chars => {
                            let slots: Vec<i64> = s(0).chars().map(|c| c as i64).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots })
                        }
                    }
                }
                Inst::Free(l) | Inst::ListFree(l) => {
                    heap.release(locals[*l]);
                    locals[*l] = 0;
                }
                Inst::ListNew(d, width) => {
                    locals[*d] = heap.add(Object::List { width: *width, length: 0, slots: vec![] });
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
