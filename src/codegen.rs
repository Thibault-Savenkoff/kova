// Native backend (`kova build`): KIR → machine code with Cranelift → `main.o`,
// then `cc` glues `main.o` and the C runtime into an executable.
//
// Each KIR local becomes a Cranelift `Variable`; each KIR block, a Cranelift block.

use std::collections::HashMap;
use std::process::Command;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{AbiParam, Block, FuncRef, GlobalValue, InstBuilder, MemFlagsData, Signature, TrapCode, Value};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::kir::*;

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");

pub fn build(p: &Program, file: &str, output: &str) -> Result<(), String> {
    let object = make_object(p, file).map_err(|e| format!("génération du code : {e}"))?;

    let tmp = std::env::temp_dir().join(format!("kova-build-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let (o, c) = (tmp.join("main.o"), tmp.join("runtime.c"));
    std::fs::write(&o, object).map_err(|e| e.to_string())?;
    std::fs::write(&c, RUNTIME_C).map_err(|e| e.to_string())?;

    let res = Command::new("cc").args(["-O2", "-o"]).arg(output).arg(&o).arg(&c).arg("-lm").output();
    let _ = std::fs::remove_dir_all(&tmp);
    match res {
        Err(e) => Err(format!("impossible de lancer `cc` (il sert à assembler l'exécutable) : {e}")),
        Ok(out) if !out.status.success() => {
            Err(format!("`cc` a échoué :\n{}", String::from_utf8_lossy(&out.stderr)))
        }
        Ok(_) => Ok(()),
    }
}

/// The C runtime functions called by the generated code.
struct Runtime {
    print: FuncId,
    fault: FuncId,
    write_text: FuncId,
    write_value: FuncId,
    fail: FuncId,
}

fn make_object(p: &Program, file: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed")?;
    flags.set("is_pic", "true")?; // Fedora's gcc makes "PIE" executables
    // A function returning many values (big tuple): Cranelift stores them in memory.
    flags.set("enable_multi_ret_implicit_sret", "true")?;
    let isa = cranelift_codegen::isa::lookup(target_lexicon::Triple::host())?.finish(settings::Flags::new(flags))?;
    let mut module = ObjectModule::new(ObjectBuilder::new(isa, "kova", default_libcall_names())?);

    // The file name, read by the runtime for its error messages.
    let mut name = DataDescription::new();
    name.define(format!("{file}\0").into_bytes().into_boxed_slice());
    let id = module.declare_data("kova_file", Linkage::Export, false, false)?;
    module.define_data(id, &name)?;

    let call_conv = module.isa().default_call_conv();
    let config = module.isa().frontend_config();
    let signature = |params: &[Scalar], returns: &[Scalar]| {
        let mut s = Signature::new(call_conv);
        s.params.extend(params.iter().map(|&t| AbiParam::new(machine_type(t))));
        s.returns.extend(returns.iter().map(|&t| AbiParam::new(machine_type(t))));
        s
    };
    let ints = [Scalar::Int; 3];
    let rt = Runtime {
        print: module.declare_function("kova_print", Linkage::Import, &signature(&ints, &[]))?,
        fault: module.declare_function("kova_fault", Linkage::Import, &signature(&ints, &[]))?,
        write_text: module.declare_function("kova_write_text", Linkage::Import, &signature(&ints[..2], &[]))?,
        write_value: module.declare_function("kova_write_value", Linkage::Import, &signature(&ints[..2], &[]))?,
        fail: module.declare_function("kova_fail", Linkage::Import, &signature(&[], &[]))?,
    };
    // The texts of contract messages, a single copy of each in the executable.
    let mut data: HashMap<String, DataId> = HashMap::new();
    let mut ids = vec![];
    for (i, f) in p.functions.iter().enumerate() {
        let sig = signature(&f.locals[..f.nb_params], &f.returns);
        let id = if i == p.main {
            module.declare_function("kova_main", Linkage::Export, &sig)?
        } else {
            module.declare_function(&format!("kova_f{i}"), Linkage::Local, &sig)?
        };
        ids.push(id);
    }

    let mut ctx = module.make_context();
    let mut fbc = FunctionBuilderContext::new();
    for (i, f) in p.functions.iter().enumerate() {
        ctx.func.signature = signature(&f.locals[..f.nb_params], &f.returns);
        let b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let refs: Vec<FuncRef> = ids.iter().map(|&id| module.declare_func_in_func(id, b.func)).collect();
        let rt_print = module.declare_func_in_func(rt.print, b.func);
        let rt_fault = module.declare_func_in_func(rt.fault, b.func);
        let mut texts = HashMap::new();
        let fails = f.blocks.iter().flat_map(|b| &b.insts).filter_map(|i| if let Inst::Fail(m) = i { Some(m) } else { None });
        for m in fails.flatten() {
            let t = match m {
                Piece::Text(t) => t.as_str(),
                Piece::File => file,
                Piece::Value(..) => continue,
            };
            if !data.contains_key(t) {
                let id = module.declare_data(&format!("kova_text{}", data.len()), Linkage::Local, false, false)?;
                let mut d = DataDescription::new();
                d.define(t.as_bytes().into());
                module.define_data(id, &d)?;
                data.insert(t.to_string(), id);
            }
            texts.entry(t.to_string()).or_insert_with(|| module.declare_data_in_func(data[t], b.func));
        }
        let fail = Fail {
            write_text: module.declare_func_in_func(rt.write_text, b.func),
            write_value: module.declare_func_in_func(rt.write_value, b.func),
            stop: module.declare_func_in_func(rt.fail, b.func),
            texts,
            file: file.to_string(),
        };
        Generator { b, vars: vec![], types: f.locals.clone(), rt_fault, fail }.function(f, &refs, rt_print, config);
        module.define_function(ids[i], &mut ctx)?;
        module.clear_context(&mut ctx);
    }
    Ok(module.finish().emit()?)
}

fn machine_type(t: Scalar) -> cranelift_codegen::ir::Type {
    match t {
        Scalar::Int => I64,
        Scalar::Float => F64,
    }
}

struct Generator<'a> {
    b: FunctionBuilder<'a>,
    vars: Vec<Variable>,
    types: Vec<Scalar>,
    rt_fault: FuncRef,
    fail: Fail,
}

/// What is needed to write the message of a broken contract.
struct Fail {
    write_text: FuncRef,
    write_value: FuncRef,
    stop: FuncRef,
    texts: HashMap<String, GlobalValue>,
    file: String,
}

impl Generator<'_> {
    fn function(mut self, f: &Function, refs: &[FuncRef], rt_print: FuncRef, config: TargetFrontendConfig) {
        let b = &mut self.b;
        self.vars = f.locals.iter().map(|&t| b.declare_var(machine_type(t))).collect();
        let blocks: Vec<Block> = f.blocks.iter().map(|_| b.create_block()).collect();

        // Entry block: stores the parameters in their variables, then jumps to block 0.
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let params = b.block_params(entry).to_vec();
        for (i, v) in params.into_iter().enumerate() {
            b.def_var(self.vars[i], v);
        }
        for (i, &t) in f.locals.iter().enumerate().skip(f.nb_params) {
            let zero = match t {
                Scalar::Int => b.ins().iconst(I64, 0),
                Scalar::Float => b.ins().f64const(0.0),
            };
            b.def_var(self.vars[i], zero);
        }
        b.ins().jump(blocks[0], &[]);

        for (k, block) in f.blocks.iter().enumerate() {
            self.b.switch_to_block(blocks[k]);
            for inst in &block.insts {
                self.inst(inst, refs, rt_print);
            }
            let b = &mut self.b;
            match &block.end {
                End::Jump(s) => {
                    b.ins().jump(blocks[*s], &[]);
                }
                End::Branch(c, yes, no) => {
                    let c = b.use_var(self.vars[*c]);
                    b.ins().brif(c, blocks[*yes], &[], blocks[*no], &[]);
                }
                End::Return(v) => {
                    let vals: Vec<Value> = v.iter().map(|&l| b.use_var(self.vars[l])).collect();
                    b.ins().return_(&vals);
                }
                End::Unreachable => {
                    b.ins().trap(TrapCode::unwrap_user(1));
                }
            }
        }
        self.b.seal_all_blocks();
        self.b.finalize(config);
    }

    fn read(&mut self, l: Local) -> Value {
        self.b.use_var(self.vars[l])
    }

    /// If `cond` is true: calls `kova_fault` (which stops the program).
    fn fault_if(&mut self, cond: Value, fault: Fault, loc: Loc) {
        let b = &mut self.b;
        let (fault_block, next) = (b.create_block(), b.create_block());
        b.set_cold_block(fault_block);
        b.ins().brif(cond, fault_block, &[], next, &[]);
        b.switch_to_block(fault_block);
        let args = [fault as i64, loc.line as i64, loc.col as i64].map(|v| b.ins().iconst(I64, v));
        b.ins().call(self.rt_fault, &args);
        b.ins().trap(TrapCode::unwrap_user(1));
        b.switch_to_block(next);
    }

    fn inst(&mut self, inst: &Inst, refs: &[FuncRef], rt_print: FuncRef) {
        match inst {
            Inst::Const(d, v) => {
                let v = match self.types[*d] {
                    Scalar::Int => self.b.ins().iconst(I64, *v),
                    Scalar::Float => self.b.ins().f64const(f64::from_bits(*v as u64)),
                };
                self.b.def_var(self.vars[*d], v);
            }
            Inst::FBinary(d, op, a, b, loc) => {
                let (a, b) = (self.read(*a), self.read(*b));
                let v = self.float_op(*op, a, b, *loc);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::FNeg(d, a) => {
                let a = self.read(*a);
                let v = self.b.ins().fneg(a);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Convert(d, c, a, loc) => {
                let a = self.read(*a);
                let v = match c {
                    Conversion::IntToFloat => self.b.ins().fcvt_from_sint(F64, a),
                    Conversion::FloatToInt => {
                        // "Ordered" comparisons: false for NaN, so NaN is refused.
                        let min = self.b.ins().f64const(-9223372036854775808.0);
                        let max = self.b.ins().f64const(9223372036854775808.0);
                        let big_enough = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, a, min);
                        let small_enough = self.b.ins().fcmp(FloatCC::LessThan, a, max);
                        let ok = self.b.ins().band(big_enough, small_enough);
                        let ko = self.b.ins().bxor_imm_s(ok, 1);
                        self.fault_if(ko, Fault::BadConversion, *loc);
                        self.b.ins().fcvt_to_sint(I64, a)
                    }
                };
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Copy(d, s) => {
                let v = self.read(*s);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Not(d, a) => {
                let a = self.read(*a);
                let v = self.b.ins().bxor_imm_s(a, 1);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Neg(d, a, loc) => {
                let a = self.read(*a);
                let zero = self.b.ins().iconst(I64, 0);
                let (v, overflow) = self.b.ins().ssub_overflow(zero, a);
                self.fault_if(overflow, Fault::Overflow, *loc);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Binary(d, op, a, b, loc) => {
                let (a, b) = (self.read(*a), self.read(*b));
                let v = self.int_op(*op, a, b, *loc);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Call(d, f, args) => {
                let vals: Vec<Value> = args.iter().map(|&l| self.read(l)).collect();
                let call = self.b.ins().call(refs[*f], &vals);
                let results = self.b.inst_results(call).to_vec();
                for (&d, r) in d.iter().zip(results) {
                    self.b.def_var(self.vars[d], r);
                }
            }
            Inst::Print { value, show, newline } => {
                let (v, show) = self.value_and_show(*value, *show);
                let newline = self.b.ins().iconst(I64, *newline as i64);
                self.b.ins().call(rt_print, &[v, show, newline]);
            }
            Inst::Fail(pieces) => {
                for m in pieces {
                    let t = match m {
                        Piece::Value(l, show) => {
                            let (v, show) = self.value_and_show(*l, *show);
                            self.b.ins().call(self.fail.write_value, &[v, show]);
                            continue;
                        }
                        Piece::Text(t) => t,
                        Piece::File => &self.fail.file,
                    };
                    let address = self.b.ins().symbol_value(I64, self.fail.texts[t]);
                    let n = self.b.ins().iconst(I64, t.len() as i64);
                    self.b.ins().call(self.fail.write_text, &[address, n]);
                }
                self.b.ins().call(self.fail.stop, &[]);
            }
        }
    }

    /// A slot and its display kind, ready for the runtime.
    fn value_and_show(&mut self, l: Local, show: Show) -> (Value, Value) {
        let mut v = self.read(l);
        if show == Show::Float {
            // The runtime receives the 64 bits of the Float in an integer.
            v = self.b.ins().bitcast(I64, MemFlagsData::new(), v);
        }
        (v, self.b.ins().iconst(I64, show as i64))
    }

    /// Same rules as `interp::compute_float`.
    fn float_op(&mut self, op: Op, a: Value, b: Value, loc: Loc) -> Value {
        let compare = |g: &mut Self, cc: FloatCC| {
            let c = g.b.ins().fcmp(cc, a, b);
            g.b.ins().uextend(I64, c)
        };
        match op {
            Op::Plus => self.b.ins().fadd(a, b),
            Op::Minus => self.b.ins().fsub(a, b),
            Op::Times => self.b.ins().fmul(a, b),
            Op::Div => {
                let zero = self.b.ins().f64const(0.0);
                let by_zero = self.b.ins().fcmp(FloatCC::Equal, b, zero); // true for -0.0 too
                self.fault_if(by_zero, Fault::DivisionByZero, loc);
                self.b.ins().fdiv(a, b)
            }
            Op::Eq => compare(self, FloatCC::Equal),
            Op::Ne => compare(self, FloatCC::NotEqual), // true for NaN, as in Rust
            Op::Lt => compare(self, FloatCC::LessThan),
            Op::Le => compare(self, FloatCC::LessThanOrEqual),
            Op::Gt => compare(self, FloatCC::GreaterThan),
            Op::Ge => compare(self, FloatCC::GreaterThanOrEqual),
            Op::Rem => unreachable!("`%` refused on Floats by the type checker"),
        }
    }

    /// Same rules as `interp::compute`.
    fn int_op(&mut self, op: Op, a: Value, b: Value, loc: Loc) -> Value {
        let compare = |g: &mut Self, cc: IntCC| {
            let c = g.b.ins().icmp(cc, a, b);
            g.b.ins().uextend(I64, c)
        };
        match op {
            Op::Plus | Op::Minus | Op::Times => {
                let ins = self.b.ins();
                let (v, overflow) = match op {
                    Op::Plus => ins.sadd_overflow(a, b),
                    Op::Minus => ins.ssub_overflow(a, b),
                    _ => ins.smul_overflow(a, b),
                };
                self.fault_if(overflow, Fault::Overflow, loc);
                v
            }
            Op::Div | Op::Rem => {
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, b, 0);
                self.fault_if(zero, Fault::DivisionByZero, loc);
                let minus_one = self.b.ins().icmp_imm_s(IntCC::Equal, b, -1);
                if op == Op::Div {
                    // -9223372036854775808 / -1 doesn't fit in an Int.
                    let min = self.b.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
                    let overflow = self.b.ins().band(min, minus_one);
                    self.fault_if(overflow, Fault::Overflow, loc);
                    self.b.ins().sdiv(a, b)
                } else {
                    // x % -1 = x % 1 = 0; avoids MIN % -1, which crashes the processor.
                    let one = self.b.ins().iconst(I64, 1);
                    let b = self.b.ins().select(minus_one, one, b);
                    self.b.ins().srem(a, b)
                }
            }
            Op::Eq => compare(self, IntCC::Equal),
            Op::Ne => compare(self, IntCC::NotEqual),
            Op::Lt => compare(self, IntCC::SignedLessThan),
            Op::Le => compare(self, IntCC::SignedLessThanOrEqual),
            Op::Gt => compare(self, IntCC::SignedGreaterThan),
            Op::Ge => compare(self, IntCC::SignedGreaterThanOrEqual),
        }
    }
}
