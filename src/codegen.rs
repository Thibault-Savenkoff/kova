// Backend natif (`kova build`) : KIR → code machine avec Cranelift → `main.o`,
// puis `cc` colle `main.o` et le runtime C en un exécutable.
//
// Chaque local du KIR devient une `Variable` Cranelift ; chaque bloc KIR, un bloc Cranelift.

use std::process::Command;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Block, FuncRef, InstBuilder, Signature, TrapCode, Value};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::kir::*;

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");

pub fn construire(p: &Programme, fichier: &str, sortie: &str) -> Result<(), String> {
    let objet = produire_objet(p, fichier).map_err(|e| format!("génération du code : {e}"))?;

    let tmp = std::env::temp_dir().join(format!("kova-build-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let (o, c) = (tmp.join("main.o"), tmp.join("runtime.c"));
    std::fs::write(&o, objet).map_err(|e| e.to_string())?;
    std::fs::write(&c, RUNTIME_C).map_err(|e| e.to_string())?;

    let res = Command::new("cc").args(["-O2", "-o"]).arg(sortie).arg(&o).arg(&c).output();
    let _ = std::fs::remove_dir_all(&tmp);
    match res {
        Err(e) => Err(format!("impossible de lancer `cc` (il sert à assembler l'exécutable) : {e}")),
        Ok(out) if !out.status.success() => {
            Err(format!("`cc` a échoué :\n{}", String::from_utf8_lossy(&out.stderr)))
        }
        Ok(_) => Ok(()),
    }
}

/// Les fonctions du runtime C appelées par le code produit.
struct Runtime {
    afficher: FuncId,
    panne: FuncId,
}

fn produire_objet(p: &Programme, fichier: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut reglages = settings::builder();
    reglages.set("opt_level", "speed")?;
    reglages.set("is_pic", "true")?; // gcc de Fedora fabrique des exécutables « PIE »
    let isa = cranelift_codegen::isa::lookup(target_lexicon::Triple::host())?
        .finish(settings::Flags::new(reglages))?;
    let mut module = ObjectModule::new(ObjectBuilder::new(isa, "kova", default_libcall_names())?);

    // Le nom du fichier, lu par le runtime pour ses messages d'erreur.
    let mut nom = DataDescription::new();
    nom.define(format!("{fichier}\0").into_bytes().into_boxed_slice());
    let id = module.declare_data("kova_fichier", Linkage::Export, false, false)?;
    module.define_data(id, &nom)?;

    let convention = module.isa().default_call_conv();
    let config = module.isa().frontend_config();
    let signature = |n_params: usize, renvoie: bool| {
        let mut s = Signature::new(convention);
        s.params.extend((0..n_params).map(|_| AbiParam::new(I64)));
        if renvoie {
            s.returns.push(AbiParam::new(I64));
        }
        s
    };
    let rt = Runtime {
        afficher: module.declare_function("kova_afficher", Linkage::Import, &signature(3, false))?,
        panne: module.declare_function("kova_panne", Linkage::Import, &signature(3, false))?,
    };
    let mut ids = vec![];
    for (i, f) in p.fonctions.iter().enumerate() {
        let sig = signature(f.nb_params, f.renvoie);
        let id = if i == p.principale {
            module.declare_function("kova_main", Linkage::Export, &sig)?
        } else {
            module.declare_function(&format!("kova_f{i}"), Linkage::Local, &sig)?
        };
        ids.push(id);
    }

    let mut ctx = module.make_context();
    let mut fbc = FunctionBuilderContext::new();
    for (i, f) in p.fonctions.iter().enumerate() {
        ctx.func.signature = signature(f.nb_params, f.renvoie);
        let b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let refs: Vec<FuncRef> = ids.iter().map(|&id| module.declare_func_in_func(id, b.func)).collect();
        let rt_afficher = module.declare_func_in_func(rt.afficher, b.func);
        let rt_panne = module.declare_func_in_func(rt.panne, b.func);
        Generateur { b, vars: vec![], rt_panne }.fonction(f, &refs, rt_afficher, config);
        module.define_function(ids[i], &mut ctx)?;
        module.clear_context(&mut ctx);
    }
    Ok(module.finish().emit()?)
}

struct Generateur<'a> {
    b: FunctionBuilder<'a>,
    vars: Vec<Variable>,
    rt_panne: FuncRef,
}

impl Generateur<'_> {
    fn fonction(mut self, f: &Fonction, refs: &[FuncRef], rt_afficher: FuncRef, config: TargetFrontendConfig) {
        let b = &mut self.b;
        self.vars = (0..f.nb_locaux).map(|_| b.declare_var(I64)).collect();
        let blocs: Vec<Block> = f.blocs.iter().map(|_| b.create_block()).collect();

        // Bloc d'entrée : range les paramètres dans leurs variables, puis saute au bloc 0.
        let entree = b.create_block();
        b.append_block_params_for_function_params(entree);
        b.switch_to_block(entree);
        let params = b.block_params(entree).to_vec();
        for (i, v) in params.into_iter().enumerate() {
            b.def_var(self.vars[i], v);
        }
        for i in f.nb_params..f.nb_locaux {
            let zero = b.ins().iconst(I64, 0);
            b.def_var(self.vars[i], zero);
        }
        b.ins().jump(blocs[0], &[]);

        for (k, bloc) in f.blocs.iter().enumerate() {
            self.b.switch_to_block(blocs[k]);
            for inst in &bloc.instrs {
                self.inst(inst, refs, rt_afficher);
            }
            let b = &mut self.b;
            match &bloc.fin {
                Fin::Saut(s) => {
                    b.ins().jump(blocs[*s], &[]);
                }
                Fin::Si(c, oui, non) => {
                    let c = b.use_var(self.vars[*c]);
                    b.ins().brif(c, blocs[*oui], &[], blocs[*non], &[]);
                }
                Fin::Retour(v) => {
                    let vals: Vec<Value> = v.iter().map(|&l| b.use_var(self.vars[l])).collect();
                    b.ins().return_(&vals);
                }
                Fin::Inatteignable => {
                    b.ins().trap(TrapCode::unwrap_user(1));
                }
            }
        }
        self.b.seal_all_blocks();
        self.b.finalize(config);
    }

    fn lire(&mut self, l: Local) -> Value {
        self.b.use_var(self.vars[l])
    }

    /// Si `cond` est vrai : appelle `kova_panne` (qui arrête le programme).
    fn panne_si(&mut self, cond: Value, panne: Panne, lieu: Lieu) {
        let b = &mut self.b;
        let (panne_bloc, suite) = (b.create_block(), b.create_block());
        b.set_cold_block(panne_bloc);
        b.ins().brif(cond, panne_bloc, &[], suite, &[]);
        b.switch_to_block(panne_bloc);
        let args = [panne as i64, lieu.ligne as i64, lieu.colonne as i64].map(|v| b.ins().iconst(I64, v));
        b.ins().call(self.rt_panne, &args);
        b.ins().trap(TrapCode::unwrap_user(1));
        b.switch_to_block(suite);
    }

    fn inst(&mut self, inst: &Inst, refs: &[FuncRef], rt_afficher: FuncRef) {
        match inst {
            Inst::Const(d, v) => {
                let v = self.b.ins().iconst(I64, *v);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Copie(d, s) => {
                let v = self.lire(*s);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Non(d, a) => {
                let a = self.lire(*a);
                let v = self.b.ins().bxor_imm_s(a, 1);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Neg(d, a, lieu) => {
                let a = self.lire(*a);
                let zero = self.b.ins().iconst(I64, 0);
                let (v, deborde) = self.b.ins().ssub_overflow(zero, a);
                self.panne_si(deborde, Panne::Depassement, *lieu);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Binaire(d, op, a, b, lieu) => {
                let (a, b) = (self.lire(*a), self.lire(*b));
                let v = self.binaire(*op, a, b, *lieu);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Appel(d, f, args) => {
                let vals: Vec<Value> = args.iter().map(|&l| self.lire(l)).collect();
                let call = self.b.ins().call(refs[*f], &vals);
                if let Some(d) = d {
                    let r = self.b.inst_results(call)[0];
                    self.b.def_var(self.vars[*d], r);
                }
            }
            Inst::Afficher { valeur, booleen, ligne } => {
                let v = self.lire(*valeur);
                let booleen = self.b.ins().iconst(I64, *booleen as i64);
                let ligne = self.b.ins().iconst(I64, *ligne as i64);
                self.b.ins().call(rt_afficher, &[v, booleen, ligne]);
            }
        }
    }

    /// Mêmes règles que `interp::calculer`.
    fn binaire(&mut self, op: Op, a: Value, b: Value, lieu: Lieu) -> Value {
        let comparer = |g: &mut Self, cc: IntCC| {
            let c = g.b.ins().icmp(cc, a, b);
            g.b.ins().uextend(I64, c)
        };
        match op {
            Op::Plus | Op::Moins | Op::Fois => {
                let ins = self.b.ins();
                let (v, deborde) = match op {
                    Op::Plus => ins.sadd_overflow(a, b),
                    Op::Moins => ins.ssub_overflow(a, b),
                    _ => ins.smul_overflow(a, b),
                };
                self.panne_si(deborde, Panne::Depassement, lieu);
                v
            }
            Op::Divise | Op::Reste => {
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, b, 0);
                self.panne_si(zero, Panne::DivisionParZero, lieu);
                let moins_un = self.b.ins().icmp_imm_s(IntCC::Equal, b, -1);
                if op == Op::Divise {
                    // -9223372036854775808 / -1 ne tient pas dans un Int.
                    let min = self.b.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
                    let deborde = self.b.ins().band(min, moins_un);
                    self.panne_si(deborde, Panne::Depassement, lieu);
                    self.b.ins().sdiv(a, b)
                } else {
                    // x % -1 = x % 1 = 0 ; évite le cas MIN % -1 qui fait planter le processeur.
                    let un = self.b.ins().iconst(I64, 1);
                    let b = self.b.ins().select(moins_un, un, b);
                    self.b.ins().srem(a, b)
                }
            }
            Op::Egal => comparer(self, IntCC::Equal),
            Op::Different => comparer(self, IntCC::NotEqual),
            Op::Inf => comparer(self, IntCC::SignedLessThan),
            Op::InfEgal => comparer(self, IntCC::SignedLessThanOrEqual),
            Op::Sup => comparer(self, IntCC::SignedGreaterThan),
            Op::SupEgal => comparer(self, IntCC::SignedGreaterThanOrEqual),
        }
    }
}
