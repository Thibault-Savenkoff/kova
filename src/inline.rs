// Inlining (PLAN Part 4 "Speed, third pass"): a call of a small function becomes a copy of its body,
// on the KIR, so a built program and a web page both gain, and `kova run` stays the plain version
// that the differential tests compare them with.

use crate::kir::{End, Function, Inst, KBlock, Local, Piece, Program};

/// A function this small (in instructions) is copied wherever it is called.
const SMALL: usize = 40;
/// A function stops receiving copies past this size.
const LARGEST: usize = 20_000;
/// Rounds: what a small function calls is copied into it first, then the whole into its callers.
const ROUNDS: usize = 2;

/// Each local an instruction reads or writes.
fn locals(inst: &mut Inst, f: &mut impl FnMut(&mut Local)) {
    match inst {
        Inst::Const(a, _) | Inst::FuncRef(a, _) | Inst::Callback(a, ..) | Inst::Text(a, _) | Inst::Free(a) | Inst::ListNew(a, _) | Inst::ListFree(a) | Inst::Exit(a) | Inst::FaultIf(a, ..) => f(a),
        Inst::Print { value, .. } => f(value),
        Inst::Copy(a, b) | Inst::Neg(a, b, _) | Inst::Not(a, b) | Inst::FNeg(a, b) | Inst::FUnary(a, _, b) | Inst::Convert(a, _, b, _) | Inst::ListLength(a, b) | Inst::ListCheck(a, b, _) | Inst::ListGrow(a, b) | Inst::ListRemove(a, b) | Inst::ListIndex(a, b) | Inst::ListSetIndex(a, b) => {
            f(a);
            f(b);
        }
        Inst::Binary(a, _, b, c, _) | Inst::FBinary(a, _, b, c, _) | Inst::Math(a, _, b, c) | Inst::ListLoad(a, b, c, _) | Inst::ListStore(a, b, _, c) | Inst::Mix(a, b, c) => {
            f(a);
            f(b);
            f(c);
        }
        Inst::Call(a, _, b) | Inst::Extern(a, _, b, _) => {
            a.iter_mut().for_each(&mut *f);
            b.iter_mut().for_each(&mut *f);
        }
        Inst::CallIndirect(a, c, b) => {
            a.iter_mut().for_each(&mut *f);
            f(c);
            b.iter_mut().for_each(&mut *f);
        }
        Inst::Str(a, _, b) => {
            f(a);
            b.iter_mut().for_each(&mut *f);
        }
        Inst::Fail(pieces) => {
            for p in pieces {
                if let Piece::Value(l, _) = p {
                    f(l);
                }
            }
        }
        Inst::Line(_, watches) => watches.iter_mut().flat_map(|w| &mut w.slots).for_each(f),
        Inst::Threads(_) => {}
    }
}

fn size(f: &Function) -> usize {
    f.blocks.iter().map(|b| b.insts.len() + 1).sum()
}

/// Whether `f` calls the function `number`.
fn calls(f: &Function, number: usize) -> bool {
    f.blocks.iter().flat_map(|b| &b.insts).any(|i| matches!(i, Inst::Call(_, g, _) if *g == number))
}

/// Puts a copy of `g` in `f` where block `at` calls it, at instruction `i`. The instructions after
/// the call go to a new block, whose number is given.
fn splice(f: &mut Function, at: usize, i: usize, g: &Function) -> usize {
    let rest = f.blocks[at].insts.split_off(i + 1);
    let Some(Inst::Call(results, _, args)) = f.blocks[at].insts.pop() else { unreachable!("a call") };
    let (shift, first) = (f.locals.len(), f.blocks.len());
    f.locals.extend_from_slice(&g.locals);
    let after = first + g.blocks.len();
    let end = std::mem::replace(&mut f.blocks[at].end, End::Jump(first));
    // A function starts with every local at 0: freeing a slot that was never filled does nothing,
    // and a loop around this call must not find the last turn's values.
    let insts = &mut f.blocks[at].insts;
    for (p, a) in args.iter().enumerate() {
        insts.push(Inst::Copy(shift + p, *a));
    }
    insts.extend((g.nb_params..g.locals.len()).map(|l| Inst::Const(shift + l, 0)));
    for block in &g.blocks {
        let mut block = block.clone();
        block.insts.iter_mut().for_each(|inst| locals(inst, &mut |l| *l += shift));
        block.end = match block.end {
            End::Jump(b) => End::Jump(b + first),
            End::Branch(c, a, b) => End::Branch(c + shift, a + first, b + first),
            End::Unreachable => End::Unreachable,
            End::Return(values) => {
                block.insts.extend(results.iter().zip(&values).map(|(d, v)| Inst::Copy(*d, v + shift)));
                End::Jump(after)
            }
        };
        f.blocks.push(block);
    }
    f.blocks.push(KBlock { insts: rest, end });
    after
}

/// The program with its small functions copied into their callers.
pub fn inlined(p: &Program) -> Program {
    let mut p = p.clone();
    for _ in 0..ROUNDS {
        let functions = p.functions.clone();
        let small = |g: usize| size(&functions[g]) <= SMALL && !calls(&functions[g], g) && Some(g) != p.main;
        let small: Vec<bool> = (0..functions.len()).map(small).collect();
        for (number, f) in p.functions.iter_mut().enumerate() {
            let mut waiting: Vec<usize> = (0..f.blocks.len()).rev().collect();
            while let Some(at) = waiting.pop() {
                let found = f.blocks[at].insts.iter().position(|i| matches!(i, Inst::Call(_, g, _) if small[*g] && *g != number));
                let (Some(i), true) = (found, size(f) < LARGEST) else { continue };
                let Inst::Call(_, g, _) = f.blocks[at].insts[i] else { unreachable!() };
                waiting.push(splice(f, at, i, &functions[g]));
            }
        }
    }
    p
}
