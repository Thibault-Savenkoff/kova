// Interpreter (`kova run`): executes the KIR directly, instruction by instruction.

use std::io::{BufWriter, Stdout, Write};

use crate::kir::*;
use crate::locale::{self, t};

/// The Strings of the running program; a String's slot holds its number here (0 = none).
#[derive(Default)]
struct Heap {
    strings: Vec<Option<String>>,
    /// Numbers of freed Strings, to reuse.
    free: Vec<usize>,
    allocated: usize,
    live: usize,
}

impl Heap {
    fn make(&mut self, s: String) -> i64 {
        self.allocated += 1;
        self.live += 1;
        if self.strings.is_empty() {
            self.strings.push(None); // number 0 = no String
        }
        match self.free.pop() {
            Some(i) => {
                self.strings[i] = Some(s);
                i as i64
            }
            None => {
                self.strings.push(Some(s));
                self.strings.len() as i64 - 1
            }
        }
    }
    fn get(&self, h: i64) -> &str {
        self.strings[h as usize].as_deref().expect("String used after being freed")
    }
    fn release(&mut self, h: i64) {
        if h != 0 {
            self.strings[h as usize].take().expect("String freed twice");
            self.free.push(h as usize);
            self.live -= 1;
        }
    }
}

/// Runs `main`. On a runtime error: prints the message and returns code 101.
/// With `KOVA_LEAKS` set (the test runner sets it): at the end, a String never freed
/// is an error (code 102); `KOVA_LEAKS=count` also prints how many were made.
pub fn run(p: &Program, file: &str) -> i32 {
    let mut out = BufWriter::new(std::io::stdout());
    let mut heap = Heap::default();
    let result = call(p, p.main, &[], &mut out, file, &mut heap);
    out.flush().unwrap();
    match result {
        Ok(_) => leaks(heap.allocated, heap.live),
        Err(message) => {
            eprint!("{message}");
            101
        }
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
pub fn test(p: &Program, file: &str) -> i32 {
    if p.tests.is_empty() {
        println!("{}", t!("tests_none", file = file));
        return 0;
    }
    let mut out = BufWriter::new(std::io::stdout());
    let mut failed = 0;
    for (name, num) in &p.tests {
        let result = call(p, *num, &[], &mut out, file, &mut Heap::default());
        let _ = match result {
            Ok(_) => writeln!(out, "✅ {name}"),
            Err(message) => {
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

/// `Err` = the message of the runtime error, as it will be printed.
type Outcome = Result<Vec<i64>, String>;

// ponytail: Rust recursion = the interpreter's stack; infinite recursion
// crashes without a nice message (same in native). To handle with stack errors.
fn call(p: &Program, num: usize, args: &[i64], out: &mut BufWriter<Stdout>, file: &str, heap: &mut Heap) -> Outcome {
    let fault = |f: Fault, loc: &Loc| fault_text(f, file, *loc);
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
                    let r = call(p, *g, &vals, out, file, heap)?;
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
                        StrOp::Length => s(0).chars().count() as i64,
                        StrOp::Clone => {
                            let copy = s(0).to_string();
                            heap.make(copy)
                        }
                    }
                }
                Inst::Free(l) => {
                    heap.release(locals[*l]);
                    locals[*l] = 0;
                }
                Inst::Print { value, show, newline } => {
                    let _ = write!(out, "{}", text(locals[*value], *show, heap));
                    if *newline {
                        let _ = writeln!(out);
                    }
                }
                Inst::Fail(pieces) => {
                    return Err(pieces
                        .iter()
                        .map(|m| match m {
                            Piece::Text(t) => t.clone(),
                            Piece::Label(k) => locale::text(k, &[]),
                            Piece::Value(l, show) => text(locals[*l], *show, heap),
                            Piece::File => file.to_string(),
                        })
                        .collect());
                }
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
