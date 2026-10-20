// Ownership (PLAN 2.7): who owns each `String`, and when it is moved.
//
//   let a: String := "hi"
//   let b: String := a      // `a` gives its text to `b`: a move
//   print(a)                // error: `a` was moved (line 2)
//
// A value is moved where it is *given*: `let`, `:=`, `return`, a tuple element or struct field, the value
// of an `if` branch. Everywhere else (a call argument, an operand, `print`) it is only
// borrowed. Copyable types (`Int`, `(Int, Bool)`…) are copied instead (`types::is_copy`).
//
// This pass is the only one that decides: it marks each move in `Info::moves`, and the
// KIR empties exactly those variables (so the end of their block frees nothing twice).

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diag, Span, line_col};
use crate::locale::t;
use crate::types::{Info, Type, is_copy, method_name, place_root};

/// A visible name.
#[derive(Clone)]
struct Var {
    id: usize,
    /// A parameter: borrowed from the caller, it cannot be moved.
    borrowed: bool,
    /// Number of open blocks when it was declared (to know if it comes from outside a loop).
    depth: usize,
}

/// The variables moved on this path (id → where), or `None` if the path never gets here
/// (after a `return`, `break` or `continue`).
type State = Option<HashMap<usize, Span>>;

fn join(a: State, b: State) -> State {
    match (a, b) {
        (None, s) | (s, None) => s,
        (Some(mut a), Some(b)) => {
            a.extend(b);
            Some(a)
        }
    }
}

struct Loop {
    depth: usize,
    /// States at each `break` and at each `continue`.
    breaks: State,
    continues: State,
}

struct Owner<'a> {
    src: &'a str,
    info: &'a mut Info,
    scopes: Vec<HashMap<String, Var>>,
    next_id: usize,
    state: State,
    loops: Vec<Loop>,
    /// How each method receives its value (`None`: a static function), by name (`Dog.sound`).
    receivers: HashMap<String, Option<Receiver>>,
    diags: Vec<Diag>,
}

pub fn check(f: &File, info: &mut Info, src: &str) -> Vec<Diag> {
    let mut receivers = HashMap::new();
    let mut functions: Vec<&Function> = f.functions.iter().collect();
    for s in &f.structs {
        for m in &s.methods {
            receivers.insert(method_name(&s.name.text, &m.name.text), m.receiver);
            functions.push(m);
        }
    }
    let mut o = Owner { src, info, scopes: vec![], next_id: 0, state: None, loops: vec![], receivers, diags: vec![] };
    for func in functions {
        o.scopes = vec![HashMap::new()];
        // `self` is borrowed, except in a `move self` method that owns it.
        if let Some(r) = func.receiver {
            o.declare("self", r != Receiver::Move);
        }
        for p in &func.params {
            o.declare(&p.name.text, true);
        }
        o.state = Some(HashMap::new());
        for c in &func.contracts {
            o.contract(c);
        }
        o.block(&func.body);
    }
    for test in &f.tests {
        o.scopes = vec![HashMap::new()];
        o.state = Some(HashMap::new());
        o.block(&test.body);
    }
    o.diags
}

impl Owner<'_> {
    fn declare(&mut self, name: &str, borrowed: bool) {
        let v = Var { id: self.next_id, borrowed, depth: self.scopes.len() };
        self.next_id += 1;
        self.scopes.last_mut().unwrap().insert(name.to_string(), v);
    }

    fn lookup(&self, name: &str) -> Var {
        self.scopes.iter().rev().find_map(|s| s.get(name)).expect("variable checked by the type checker").clone()
    }

    fn line(&self, s: Span) -> usize {
        line_col(self.src, s.start).0
    }

    /// Reading `name`: an error if it was moved on this path.
    fn use_var(&mut self, name: &str, span: Span) {
        let v = self.lookup(name);
        let Some(moved) = self.state.as_mut().and_then(|s| s.remove(&v.id)) else { return };
        let line = self.line(moved);
        self.diags.push(
            Diag::new(span, t!("moved_use", name = name, line = line)).help(t!("moved_use_help", name = name, line = line)),
        );
    }

    fn block(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    /// A block whose last line is a value (`if` expression): that value is given.
    fn block_value(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        let (last, before) = b.stmts.split_last().expect("block checked by the type checker");
        for s in before {
            self.stmt(s);
        }
        match last {
            Stmt::Expr(e) => self.give(e),
            Stmt::If { cond, then, otherwise: Some(otherwise) } => self.if_value(cond, then, otherwise),
            _ => unreachable!("block checked by the type checker"),
        }
        self.scopes.pop();
    }

    fn if_value(&mut self, cond: &Expr, then: &Block, otherwise: &Block) {
        self.borrow(cond);
        let start = self.state.clone();
        self.block_value(then);
        let after_then = std::mem::replace(&mut self.state, start);
        self.block_value(otherwise);
        self.state = join(after_then, self.state.take());
    }

    fn bind(&mut self, p: &Pattern, ty: &Type) {
        match (p, ty) {
            (Pattern::Name(n), _) => self.declare(&n.text, false),
            (Pattern::Ignore(_), _) => {}
            (Pattern::Tuple(ps, _), Type::Tuple(ts)) => {
                for (p, t) in ps.iter().zip(ts) {
                    self.bind(p, t);
                }
            }
            (Pattern::Tuple(..), _) => unreachable!("pattern checked by the type checker"),
        }
    }

    /// `x := ...` on a whole variable gives it a value again: it is no longer moved.
    fn assign(&mut self, t: &Target) {
        match t {
            Target::Tuple(ts, _) => ts.iter().for_each(|t| self.assign(t)),
            Target::Place { name, fields, span } if !fields.is_empty() => self.use_var(&name.text, *span),
            Target::Place { name, .. } => {
                let id = self.lookup(&name.text).id;
                if let Some(s) = self.state.as_mut() {
                    s.remove(&id);
                }
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Declare { pattern, value, .. } => {
                self.give(value);
                let ty = self.info.types[value.id].clone();
                self.bind(pattern, &ty);
            }
            Stmt::Assign { target, op: None, value } => {
                self.give(value);
                self.assign(target);
            }
            Stmt::Assign { target, op: Some(_), value } => {
                self.borrow(value);
                if let Target::Place { name, span, .. } = target {
                    self.use_var(&name.text, *span);
                }
            }
            Stmt::Return { value, .. } => {
                if let Some(e) = value {
                    self.give(e);
                }
                self.state = None;
            }
            Stmt::Check(c) => self.contract(c),
            Stmt::Break(_) => {
                let s = self.state.take();
                let l = self.loops.last_mut().unwrap();
                l.breaks = join(l.breaks.take(), s);
            }
            Stmt::Continue(_) => {
                let s = self.state.take();
                let l = self.loops.last_mut().unwrap();
                l.continues = join(l.continues.take(), s);
            }
            Stmt::If { cond, then, otherwise } => {
                self.borrow(cond);
                let start = self.state.clone();
                self.block(then);
                let after_then = std::mem::replace(&mut self.state, start);
                if let Some(o) = otherwise {
                    self.block(o);
                }
                self.state = join(after_then, self.state.take());
            }
            Stmt::While { cond, body } => {
                self.borrow(cond);
                self.body_of_loop(body, None);
            }
            Stmt::Loop { counter, start, end, body, .. } => {
                self.borrow(start);
                self.borrow(end);
                self.body_of_loop(body, Some(counter));
            }
            Stmt::Expr(e) => self.borrow(e),
        }
    }

    /// The body runs again and again: a variable from outside the loop that is still moved
    /// at the end of the body (or at a `continue`) would be moved again on the next turn.
    fn body_of_loop(&mut self, body: &Block, counter: Option<&Name>) {
        let entry = self.state.clone();
        let depth = self.scopes.len();
        self.loops.push(Loop { depth, breaks: None, continues: None });
        self.scopes.push(HashMap::new());
        if let Some(c) = counter {
            self.declare(&c.text, false);
        }
        self.block(body);
        self.scopes.pop();
        let l = self.loops.pop().unwrap();
        let end = join(self.state.take(), l.continues);
        let outer: HashMap<usize, String> = self
            .scopes
            .iter()
            .flat_map(|s| s.iter())
            .filter(|(_, v)| v.depth <= l.depth)
            .map(|(n, v)| (v.id, n.clone()))
            .collect();
        let before = entry.clone().unwrap_or_default();
        let mut again: Vec<(Span, String)> = end
            .iter()
            .flatten()
            .filter(|(id, _)| !before.contains_key(id))
            .filter_map(|(id, span)| Some((*span, outer.get(id)?.clone())))
            .collect();
        again.sort_by_key(|(s, _)| s.start);
        for (span, name) in again {
            self.diags.push(Diag::new(span, t!("loop_move", name = name)).help(t!("loop_move_help", name = name)));
        }
        // After the loop: the condition was false at once, or after some turns, or a `break`.
        self.state = join(join(entry, end), l.breaks);
    }

    /// `e` is given (`let`, `:=`, `return`, tuple element, branch value): a variable is moved.
    fn give(&mut self, e: &Expr) {
        let ty = &self.info.types[e.id];
        if is_copy(ty) {
            return self.borrow(e);
        }
        match &e.kind {
            ExprKind::Var(name) => {
                self.use_var(name, e.span);
                let v = self.lookup(name);
                if v.borrowed {
                    self.diags.push(
                        Diag::new(e.span, t!("borrowed_move", name = name)).help(t!("borrowed_move_help", name = name)),
                    );
                } else if let Some(s) = self.state.as_mut() {
                    s.insert(v.id, e.span);
                    self.info.moves[e.id] = true;
                }
            }
            ExprKind::Field(..) => {
                self.borrow(e);
                let place = &self.src[e.span.start..e.span.end];
                self.diags.push(Diag::new(e.span, t!("field_move", place = place)).help(t!("field_move_help", place = place)));
            }
            ExprKind::Method { receiver, name, .. } if self.info.types[receiver.id].is_field(&name.text) => {
                self.borrow(e);
                let place = &self.src[e.span.start..e.span.end];
                self.diags.push(
                    Diag::new(e.span, t!("struct_field_move", place = place)).help(t!("field_move_help", place = place)),
                );
            }
            _ => self.borrow(e),
        }
    }

    fn contract(&mut self, c: &Contract) {
        self.borrow(&c.cond);
        self.parts(c.message.iter().flatten());
    }

    fn parts<'p>(&mut self, parts: impl Iterator<Item = &'p Part>) {
        for p in parts {
            if let Part::Code(e) = p {
                self.borrow(e);
            }
        }
    }

    /// `rex.sound`: how the struct method receives `rex` (`None`: not a struct method).
    fn receiver(&self, receiver: &Expr, name: &Name) -> Option<Receiver> {
        let Type::Struct(d) = &self.info.types[receiver.id] else { return None };
        (*self.receivers.get(&method_name(&d.name, &name.text))?).or(Some(Receiver::Read))
    }

    /// `rex.rename(rex.name)`: the method changes `rex` while reading one of its Strings
    /// (rule 2.7.5). A copied value (`rex.age`) is read before the call: allowed.
    fn exclusive(&mut self, receiver: &Expr, method: &Name, args: &[Expr]) {
        let Some((root, _)) = place_root(receiver, &self.info.types) else { return };
        for a in args {
            let same = place_root(a, &self.info.types).is_some_and(|(n, _)| n == root);
            if same && !is_copy(&self.info.types[a.id]) {
                let arg = &self.src[a.span.start..a.span.end];
                self.diags.push(
                    Diag::new(a.span, t!("exclusive_use", name = root, method = method.text))
                        .help(t!("exclusive_use_help", arg = arg)),
                );
            }
        }
    }

    /// `e` is only read.
    fn borrow(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Var(name) => self.use_var(name, e.span),
            ExprKind::Call { args, .. } | ExprKind::Static { args, .. } => args.iter().for_each(|a| self.borrow(a)),
            ExprKind::Method { receiver, name, args } if self.receiver(receiver, name).is_some() => {
                let r = self.receiver(receiver, name).unwrap();
                if r == Receiver::Move {
                    self.give(receiver);
                } else {
                    self.borrow(receiver);
                }
                args.iter().for_each(|a| self.borrow(a));
                if r == Receiver::Var {
                    self.exclusive(receiver, name, args);
                }
            }
            ExprKind::Method { receiver, args, .. } => {
                self.borrow(receiver);
                args.iter().for_each(|a| self.borrow(a));
            }
            ExprKind::Unary(_, x) | ExprKind::Field(x, _) => self.borrow(x),
            ExprKind::Binary(BinOp::And | BinOp::Or, a, b) => {
                // `b` is computed only sometimes: what it moves is "maybe moved" afterwards.
                self.borrow(a);
                let start = self.state.clone();
                self.borrow(b);
                self.state = join(start, self.state.take());
            }
            ExprKind::Binary(_, a, b) => {
                self.borrow(a);
                self.borrow(b);
            }
            // A new tuple or struct owns its elements.
            ExprKind::Tuple(elems) => elems.iter().for_each(|x| self.give(x)),
            ExprKind::StructLit { fields, .. } => fields.iter().for_each(|(_, x)| self.give(x)),
            ExprKind::If { cond, then, otherwise } => self.if_value(cond, then, otherwise),
            ExprKind::Interp(parts) => self.parts(parts.iter()),
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Char(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::InMode { .. }
            | ExprKind::Result => {}
        }
    }
}
