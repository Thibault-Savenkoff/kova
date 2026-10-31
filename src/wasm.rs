// `kova build --target web` (PLAN Part 4 "WebAssembly"): a page holding one WebAssembly module.
// The runtime is already a module (runtime/web.rs, linked by build.rs while `kova` itself is
// built); the program's functions are added after its own, so that no runtime function moves, and
// they call it by its exported names. Nothing but `kova` is needed on the user's machine.
//
// Each KIR local becomes a wasm local. WebAssembly has no `goto`: the KIR's blocks become `block`,
// `loop` and `if` again (`structured`, after Ramsey's "Beyond Relooper").
//
// A page must not block, yet `sleep` and `read_line` wait. A function that may wait (`waiting`) is
// written otherwise (`resumable`): a loop around a `br_table`, one case per block, so that it can
// be left at a call, its locals saved, and entered there again later.

use std::collections::HashMap;

use crate::codegen::{runtime_text, str_op};
use crate::kir::*;
use crate::locale::{self, t};

const I32: u8 = 0x7f;
const I64: u8 = 0x7e;
const F64: u8 = 0x7c;

const SEC_TYPE: u8 = 1;
const SEC_IMPORT: u8 = 2;
const SEC_FUNCTION: u8 = 3;
const SEC_TABLE: u8 = 4;
const SEC_MEMORY: u8 = 5;
const SEC_EXPORT: u8 = 7;
const SEC_ELEMENT: u8 = 9;
const SEC_CODE: u8 = 10;
const SEC_DATA: u8 = 11;
const SEC_DATA_COUNT: u8 = 12;

const PAGE: u32 = 1 << 16;

fn leb(o: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        o.push(if v != 0 { byte | 0x80 } else { byte });
        if v == 0 {
            return;
        }
    }
}

fn sleb(o: &mut Vec<u8>, mut v: i64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0);
        o.push(if done { byte } else { byte | 0x80 });
        if done {
            return;
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, at: 0 }
    }
    fn byte(&mut self) -> u8 {
        self.at += 1;
        self.b[self.at - 1]
    }
    fn number(&mut self) -> u32 {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let c = self.byte();
            v |= ((c & 0x7f) as u64) << shift;
            shift += 7;
            if c & 0x80 == 0 {
                return v as u32;
            }
        }
    }
    fn bytes(&mut self) -> &'a [u8] {
        let n = self.number() as usize;
        self.at += n;
        &self.b[self.at - n..self.at]
    }
    fn more(&self) -> bool {
        self.at < self.b.len()
    }
}

type FuncType = (Vec<u8>, Vec<u8>);

/// What is needed of the runtime's module.
struct Base<'a> {
    sections: Vec<(u8, &'a [u8])>,
    types: Vec<FuncType>,
    imported: u32,
    /// The type of each function it defines.
    functions: Vec<u32>,
    exports: HashMap<String, u32>,
    /// A function's number in the table, for those already there.
    table: HashMap<u32, u32>,
    table_size: u32,
    /// Its memory's first size, in pages; the program's texts go right after.
    pages: u32,
}

impl<'a> Base<'a> {
    fn read(module: &'a [u8]) -> Base<'a> {
        let mut r = Reader::new(&module[8..]);
        let mut base = Base { sections: vec![], types: vec![], imported: 0, functions: vec![], exports: HashMap::new(), table: HashMap::new(), table_size: 0, pages: 0 };
        while r.more() {
            let id = r.byte();
            let body = r.bytes();
            if id == 0 {
                continue; // names and debug information
            }
            base.sections.push((id, body));
            let mut s = Reader::new(body);
            match id {
                SEC_TYPE => {
                    for _ in 0..s.number() {
                        assert_eq!(s.byte(), 0x60);
                        let params = s.bytes().to_vec();
                        base.types.push((params, s.bytes().to_vec()));
                    }
                }
                SEC_IMPORT => {
                    for _ in 0..s.number() {
                        s.bytes();
                        s.bytes();
                        assert_eq!(s.byte(), 0, "the runtime only imports functions");
                        s.number();
                        base.imported += 1;
                    }
                }
                SEC_FUNCTION => base.functions = (0..s.number()).map(|_| s.number()).collect(),
                SEC_TABLE => {
                    s.number();
                    s.byte();
                    let flags = s.byte();
                    base.table_size = s.number();
                    let _ = flags;
                }
                SEC_MEMORY => {
                    s.number();
                    s.byte();
                    base.pages = s.number();
                }
                SEC_EXPORT => {
                    for _ in 0..s.number() {
                        let name = String::from_utf8_lossy(s.bytes()).into_owned();
                        let kind = s.byte();
                        let number = s.number();
                        if kind == 0 {
                            base.exports.insert(name, number);
                        }
                    }
                }
                SEC_ELEMENT => {
                    for _ in 0..s.number() {
                        assert_eq!(s.byte(), 0, "the runtime's table has one plain segment");
                        assert_eq!(s.byte(), 0x41);
                        let mut at = s.number();
                        assert_eq!(s.byte(), 0x0b);
                        for _ in 0..s.number() {
                            base.table.insert(s.number(), at);
                            at += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        base
    }

    fn section(&self, id: u8) -> &'a [u8] {
        self.sections.iter().find(|(i, _)| *i == id).map_or(&[], |(_, b)| b)
    }

    /// The function number and the type of an exported runtime function.
    fn runtime(&self, name: &str) -> Option<(u32, &FuncType)> {
        let f = *self.exports.get(name)?;
        Some((f, &self.types[self.functions[(f - self.imported) as usize] as usize]))
    }

    fn type_number(&mut self, t: FuncType) -> u32 {
        match self.types.iter().position(|x| *x == t) {
            Some(k) => k as u32,
            None => {
                self.types.push(t);
                self.types.len() as u32 - 1
            }
        }
    }
}

/// The program's data, placed after the runtime's memory.
struct Data {
    at: u32,
    bytes: Vec<u8>,
    texts: HashMap<Vec<u8>, u32>,
}

impl Data {
    fn add(&mut self, b: &[u8], align: u32) -> u32 {
        while (self.at as usize + self.bytes.len()) % align as usize != 0 {
            self.bytes.push(0);
        }
        let at = self.at + self.bytes.len() as u32;
        self.bytes.extend_from_slice(b);
        at
    }

    /// A text followed by a 0 byte, each text once.
    fn text(&mut self, t: &str) -> u32 {
        if let Some(&at) = self.texts.get(t.as_bytes()) {
            return at;
        }
        let at = self.add(&[t.as_bytes(), &[0]].concat(), 1);
        self.texts.insert(t.as_bytes().to_vec(), at);
        at
    }

    /// A table of addresses (32 bits here) of these texts.
    fn table(&mut self, items: &[String]) -> u32 {
        let addresses: Vec<u32> = items.iter().map(|t| self.text(t)).collect();
        let bytes: Vec<u8> = addresses.iter().flat_map(|a| a.to_le_bytes()).collect();
        self.add(&bytes, 4)
    }
}

/// What a built-in needs that a page does not have: the program cannot be built.
fn refused(op: StrOp) -> Option<&'static str> {
    Some(match op {
        StrOp::Run => "system.run",
        StrOp::Listen | StrOp::Accept | StrOp::Reply => "web.serve",
        StrOp::Net | StrOp::NetBytes => "net",
        StrOp::Workers | StrOp::Parallel | StrOp::Spawn | StrOp::PipeNew => "threads",
        _ => return None,
    })
}

/// The functions reached from `main`, in order.
fn reached(p: &Program, main: usize) -> Vec<usize> {
    let mut seen = vec![false; p.functions.len()];
    let mut todo = vec![main];
    seen[main] = true;
    while let Some(f) = todo.pop() {
        for inst in p.functions[f].blocks.iter().flat_map(|b| &b.insts) {
            if let Inst::Call(_, g, _) | Inst::FuncRef(_, g) = inst
                && !seen[*g]
            {
                seen[*g] = true;
                todo.push(*g);
            }
        }
    }
    (0..p.functions.len()).filter(|&f| seen[f]).collect()
}

/// A built-in that may leave the program waiting (runtime/web.rs `Suspended`).
fn waits(op: StrOp) -> bool {
    matches!(op, StrOp::Sleep | StrOp::ReadLine | StrOp::UiShow | StrOp::UiDialog | StrOp::Sound | StrOp::Web)
}

/// The functions that may wait: those with a waiting built-in, their callers, and whoever calls a
/// block when one of the functions passed as blocks waits.
fn waiting(p: &Program, order: &[usize]) -> Vec<bool> {
    let mut found = vec![false; p.functions.len()];
    let mut blocks = false;
    loop {
        let mut changed = false;
        for &f in order {
            let may = p.functions[f].blocks.iter().flat_map(|b| &b.insts).any(|i| may_wait(i, &found, blocks));
            if may && !found[f] {
                found[f] = true;
                changed = true;
            }
        }
        let any = order.iter().flat_map(|&f| &p.functions[f].blocks).flat_map(|b| &b.insts).any(|i| matches!(i, Inst::FuncRef(_, g) if found[*g]));
        if any && !blocks {
            blocks = true;
            changed = true;
        }
        if !changed {
            return found;
        }
    }
}

fn may_wait(inst: &Inst, found: &[bool], blocks: bool) -> bool {
    match inst {
        Inst::Str(_, op, _) => waits(*op),
        Inst::Call(_, g, _) => found[*g],
        Inst::CallIndirect(..) => blocks,
        _ => false,
    }
}

/// Each instruction that may wait gets a block of its own: the function is entered there again.
fn split(f: &Function, found: &[bool], blocks: bool) -> Vec<KBlock> {
    let mut out: Vec<KBlock> = vec![];
    // Blocks keep their numbers; the pieces after the first get new ones at the end.
    let mut pieces: Vec<KBlock> = vec![];
    let mut next = f.blocks.len();
    for b in &f.blocks {
        let empty = || KBlock { insts: vec![], end: End::Unreachable };
        let mut current = empty();
        let mut finished: Vec<KBlock> = vec![];
        for inst in &b.insts {
            if may_wait(inst, found, blocks) {
                if !current.insts.is_empty() {
                    current.end = End::Jump(next);
                    finished.push(std::mem::replace(&mut current, empty()));
                    next += 1;
                }
                current.insts.push(inst.clone());
                current.end = End::Jump(next);
                finished.push(std::mem::replace(&mut current, empty()));
                next += 1;
            } else {
                current.insts.push(inst.clone());
            }
        }
        current.end = b.end.clone();
        finished.push(current);
        let mut finished = finished.into_iter();
        out.push(finished.next().unwrap());
        pieces.extend(finished);
    }
    out.extend(pieces);
    out
}

/// The shape of a function's jumps, to write them as `block`, `loop` and `if`.
struct Shape {
    /// Blocks in reverse postorder, and each block's place in it.
    order: Vec<usize>,
    place: Vec<usize>,
    /// Reached by two forward jumps or more.
    merge: Vec<bool>,
    /// The target of a jump backwards.
    header: Vec<bool>,
    /// The merge blocks whose immediate dominator this block is, the last in the order first.
    merges: Vec<Vec<usize>>,
}

fn targets(e: &End) -> Vec<usize> {
    match e {
        End::Jump(a) => vec![*a],
        End::Branch(_, a, b) => vec![*a, *b],
        End::Return(_) | End::Unreachable => vec![],
    }
}

/// `None` if the jumps do not nest (Kova's control flow always does: a new one would fall back on
/// `resumable`, slower but right).
fn shape(f: &Function) -> Option<Shape> {
    let n = f.blocks.len();
    let mut order = vec![];
    let mut seen = vec![false; n];
    // Depth first, without recursion: a function may have thousands of blocks.
    let mut stack: Vec<(usize, Vec<usize>)> = vec![(0, targets(&f.blocks[0].end))];
    seen[0] = true;
    while let Some((b, next)) = stack.last_mut() {
        match next.pop() {
            Some(t) if !seen[t] => {
                seen[t] = true;
                let more = targets(&f.blocks[t].end);
                stack.push((t, more));
            }
            Some(_) => {}
            None => {
                order.push(*b);
                stack.pop();
            }
        }
    }
    order.reverse();
    let mut place = vec![usize::MAX; n];
    for (i, &b) in order.iter().enumerate() {
        place[b] = i;
    }
    let mut preds: Vec<Vec<usize>> = vec![vec![]; n];
    for &b in &order {
        for t in targets(&f.blocks[b].end) {
            preds[t].push(b);
        }
    }
    // Immediate dominators (Cooper, Harvey, Kennedy).
    let mut idom = vec![usize::MAX; n];
    idom[0] = 0;
    let meet = |idom: &[usize], mut a: usize, mut b: usize| {
        while a != b {
            while place[a] > place[b] {
                a = idom[a];
            }
            while place[b] > place[a] {
                b = idom[b];
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &order[1..] {
            let mut new = usize::MAX;
            for &p in &preds[b] {
                if idom[p] != usize::MAX {
                    new = if new == usize::MAX { p } else { meet(&idom, p, new) };
                }
            }
            if idom[b] != new {
                idom[b] = new;
                changed = true;
            }
        }
    }
    let dominates = |a: usize, mut b: usize| {
        while b != a && b != 0 {
            b = idom[b];
        }
        b == a
    };
    let (mut merge, mut header) = (vec![false; n], vec![false; n]);
    for &b in &order {
        let forward = preds[b].iter().filter(|&&p| place[p] < place[b]).count();
        merge[b] = forward >= 2;
        for &p in &preds[b] {
            if place[p] >= place[b] {
                if !dominates(b, p) {
                    return None;
                }
                header[b] = true;
            }
        }
    }
    let mut merges = vec![vec![]; n];
    for &b in order.iter().rev() {
        if merge[b] {
            merges[idom[b]].push(b);
        }
    }
    Some(Shape { order, place, merge, header, merges })
}

/// What a `br` can reach, innermost last.
#[derive(Clone, Copy, PartialEq)]
enum Label {
    If,
    Block(usize),
    Loop(usize),
}

fn scalar(t: Scalar) -> u8 {
    match t {
        Scalar::Int => I64,
        Scalar::Float => F64,
    }
}

/// A value given to a runtime function.
#[derive(Clone, Copy)]
enum Arg {
    Slot(Local),
    /// A Float's bits in an integer.
    Bits(Local),
    Number(i64),
}

/// The program as a WebAssembly module: the runtime's, with the program's functions added.
pub fn module(p: &Program, files: &[String], lang: Option<usize>, runtime: &[u8]) -> Result<Vec<u8>, String> {
    let Some(main) = p.main else { return Err(t!("codegen_failed", error = "no main")) };
    let mut base = Base::read(runtime);
    let order = reached(p, main);
    if !p.externs.is_empty() {
        return Err(t!("web_refuses", what = "extern"));
    }
    for inst in order.iter().flat_map(|&f| &p.functions[f].blocks).flat_map(|b| &b.insts) {
        match inst {
            Inst::Str(_, op, _) => {
                if let Some(what) = refused(*op) {
                    return Err(t!("web_refuses", what = what));
                }
            }
            Inst::Threads(_) => return Err(t!("web_refuses", what = "threads")),
            _ => {}
        }
    }
    let may = waiting(p, &order);
    let blocks = order.iter().flat_map(|&f| &p.functions[f].blocks).flat_map(|b| &b.insts).any(|i| matches!(i, Inst::FuncRef(_, g) if may[*g]));
    let first = base.imported + base.functions.len() as u32;
    let number: HashMap<usize, u32> = order.iter().enumerate().map(|(i, &f)| (f, first + i as u32)).collect();
    // The table: the runtime's own, then the functions the program takes the address of.
    let mut table_extra: Vec<u32> = vec![];
    let mut in_table: HashMap<u32, u32> = base.table.clone();
    let mut size = base.table_size;
    let mut enter = |f: u32| {
        *in_table.entry(f).or_insert_with(|| {
            table_extra.push(f);
            size += 1;
            size - 1
        })
    };
    let shows = ["int", "bool", "float", "char", "string", "site"].map(|name| base.exports.get(&format!("kova_show_{name}")).map_or(0, |&f| enter(f)));
    let mut functions: HashMap<usize, u32> = HashMap::new();
    for inst in order.iter().flat_map(|&f| &p.functions[f].blocks).flat_map(|b| &b.insts) {
        if let Inst::FuncRef(_, g) = inst {
            functions.insert(*g, enter(number[g]));
        }
    }
    let main_entry = enter(number[&main]);

    let mut data = Data { at: base.pages * PAGE, bytes: vec![], texts: HashMap::new() };
    let mut sigs = vec![];
    let mut bodies = vec![];
    for &f in &order {
        let func = &p.functions[f];
        let sig = (func.locals[..func.nb_params].iter().map(|&t| scalar(t)).collect(), func.returns.iter().map(|&t| scalar(t)).collect());
        sigs.push(base.type_number(sig));
        let mut g = Generator { base: &mut base, data: &mut data, o: vec![], kinds: func.locals.clone(), number: &number, functions: &functions, shows, files, pc: func.locals.len() as u32, depth: 0, labels: vec![] };
        bodies.push(if may[f] {
            g.resumable(func, &split(func, &may, blocks), |i| may_wait(i, &may, blocks))?
        } else {
            match shape(func) {
                Some(shape) => g.structured(func, &shape)?,
                None => g.resumable(func, &func.blocks, |_| false)?,
            }
        });
    }

    // What the runtime reads (`Program` in runtime/kova.rs), then the entry point.
    let langs = locale::LANGUAGES.len();
    let kept = |k: &str| locale::RUNTIME_HOLES.iter().find(|(key, _)| *key == k).map_or(vec![], |(_, h)| h.iter().map(|h| (*h, format!("{{{h}}}"))).collect());
    let texts: Vec<String> = (0..langs)
        .flat_map(|l| {
            locale::RUNTIME.iter().map(move |k| {
                let holes = kept(k);
                let args: Vec<(&str, &dyn std::fmt::Display)> = holes.iter().map(|(h, v)| (*h, v as &dyn std::fmt::Display)).collect();
                locale::text_in(l, k, &args)
            })
        })
        .collect();
    let codes: Vec<String> = locale::LANGUAGES.iter().map(|(code, _)| code.to_string()).collect();
    let tables = [data.table(&texts), data.table(&codes), data.table(files), data.add(&0u64.to_le_bytes(), 8)];
    let counts = [langs as i32, locale::RUNTIME.len() as i32, lang.map_or(-1, |l| l as i32)];
    let header: Vec<u8> = tables.iter().flat_map(|a| a.to_le_bytes()).chain(counts.iter().flat_map(|c| c.to_le_bytes())).collect();
    let program = data.add(&header, 4);

    let start = first + order.len() as u32;
    sigs.push(base.type_number((vec![], vec![])));
    let mut body = vec![0];
    body.push(0x41);
    sleb(&mut body, program as i64);
    body.push(0x41);
    sleb(&mut body, main_entry as i64);
    body.push(0x10);
    leb(&mut body, base.exports["kova_web_start"] as u64);
    body.push(0x0b);
    bodies.push(body);

    // The module: the runtime's sections, some with the program added at the end.
    let mut out = runtime[..8].to_vec();
    let mut write = |id: u8, body: &[u8]| {
        out.push(id);
        leb(&mut out, body.len() as u64);
        out.extend_from_slice(body);
    };
    let appended = |old: &[u8], added: u32, new: &[u8]| {
        let mut r = Reader::new(old);
        let n = if old.is_empty() { 0 } else { r.number() };
        let mut s = vec![];
        leb(&mut s, (n + added) as u64);
        s.extend_from_slice(&old[r.at..]);
        s.extend_from_slice(new);
        s
    };
    let mut memory_pages = (data.at + data.bytes.len() as u32).div_ceil(PAGE);
    memory_pages = memory_pages.max(base.pages);
    let old_types = {
        let mut r = Reader::new(base.section(SEC_TYPE));
        r.number()
    } as usize;
    let mut new_types = vec![];
    for (params, results) in &base.types[old_types..] {
        new_types.push(0x60);
        leb(&mut new_types, params.len() as u64);
        new_types.extend_from_slice(params);
        leb(&mut new_types, results.len() as u64);
        new_types.extend_from_slice(results);
    }
    let mut new_functions = vec![];
    for s in &sigs {
        leb(&mut new_functions, *s as u64);
    }
    let mut new_code = vec![];
    for b in &bodies {
        leb(&mut new_code, b.len() as u64);
        new_code.extend_from_slice(b);
    }
    let mut new_element = vec![0, 0x41];
    sleb(&mut new_element, (size - table_extra.len() as u32) as i64);
    new_element.push(0x0b);
    leb(&mut new_element, table_extra.len() as u64);
    for f in &table_extra {
        leb(&mut new_element, *f as u64);
    }
    let mut new_data = vec![0, 0x41];
    sleb(&mut new_data, data.at as i64);
    new_data.push(0x0b);
    leb(&mut new_data, data.bytes.len() as u64);
    new_data.extend_from_slice(&data.bytes);
    for &(id, body) in &base.sections {
        match id {
            SEC_TYPE => write(id, &appended(body, (base.types.len() - old_types) as u32, &new_types)),
            SEC_FUNCTION => write(id, &appended(body, sigs.len() as u32, &new_functions)),
            SEC_TABLE => {
                let mut s = vec![1, 0x70, 1];
                leb(&mut s, size as u64);
                leb(&mut s, size as u64);
                write(id, &s);
            }
            SEC_MEMORY => {
                let mut s = vec![1, 0];
                leb(&mut s, memory_pages as u64);
                write(id, &s);
            }
            SEC_EXPORT => {
                let mut s = vec![3];
                for (name, kind, number) in [("memory", 2u8, 0u32), ("kova_start", 0, start), ("kova_web_stack_full", 0, base.exports["kova_web_stack_full"])] {
                    leb(&mut s, name.len() as u64);
                    s.extend_from_slice(name.as_bytes());
                    s.push(kind);
                    leb(&mut s, number as u64);
                }
                write(id, &s);
            }
            SEC_ELEMENT => write(id, &appended(body, 1, &new_element)),
            SEC_CODE => write(id, &appended(body, bodies.len() as u32, &new_code)),
            SEC_DATA => write(id, &appended(body, 1, &new_data)),
            SEC_DATA_COUNT => {
                let mut r = Reader::new(body);
                let mut s = vec![];
                leb(&mut s, r.number() as u64 + 1);
                write(id, &s);
            }
            _ => write(id, body),
        }
    }
    Ok(shaken(&out))
}

/// Walks one instruction at `r`; a `call` or `ref.func` is reported with where its number starts.
fn walk(r: &mut Reader, mut called: impl FnMut(u32, usize, usize)) {
    let code = r.byte();
    match code {
        0x02..=0x04 => {
            // A block type: one byte, or a signed number.
            let first = r.byte();
            if first & 0x80 != 0 {
                r.number();
            }
        }
        0x0c | 0x0d | 0x20..=0x26 => {
            r.number();
        }
        0x0e => {
            for _ in 0..=r.number() {
                r.number();
            }
        }
        0x10 | 0xd2 => {
            let start = r.at;
            let f = r.number();
            called(f, start, r.at);
        }
        0x11 => {
            r.number();
            r.number();
        }
        0x1c => {
            for _ in 0..r.number() {
                r.byte();
            }
        }
        0x28..=0x3e => {
            r.number();
            r.number();
        }
        0x3f | 0x40 | 0xd0 => {
            r.byte();
        }
        0x41 | 0x42 => while r.byte() & 0x80 != 0 {},
        0x43 => r.at += 4,
        0x44 => r.at += 8,
        0xfc => match r.number() {
            0..=7 => {}
            8 | 10 | 12 | 14 => {
                r.number();
                r.number();
            }
            _ => {
                r.number();
            }
        },
        _ => {}
    }
}

/// Each function body: where its instructions start and end, in the code section.
fn bodies_of(code: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut r = Reader::new(code);
    (0..r.number())
        .map(|_| {
            let size = r.number() as usize;
            let (start, end) = (r.at, r.at + size);
            for _ in 0..r.number() {
                r.number();
                r.byte();
            }
            let first = r.at;
            r.at = end;
            (start, first, end)
        })
        .collect()
}

/// The module without the functions nothing can reach from its exports and its table: the
/// runtime holds all of Kova's library, a program uses little of it.
fn shaken(module: &[u8]) -> Vec<u8> {
    let base = Base::read(module);
    let (imported, code) = (base.imported as usize, base.section(SEC_CODE));
    let bodies = bodies_of(code);
    let mut kept = vec![false; imported + bodies.len()];
    let mut todo: Vec<usize> = base.table.keys().map(|&f| f as usize).collect();
    let mut r = Reader::new(base.section(SEC_EXPORT));
    for _ in 0..r.number() {
        r.bytes();
        let kind = r.byte();
        let n = r.number() as usize;
        if kind == 0 {
            todo.push(n);
        }
    }
    while let Some(f) = todo.pop() {
        if std::mem::replace(&mut kept[f], true) || f < imported {
            continue;
        }
        let (_, first, end) = bodies[f - imported];
        let mut r = Reader::new(&code[..end]);
        r.at = first;
        while r.at < end {
            walk(&mut r, |g, _, _| todo.push(g as usize));
        }
    }
    let mut number = vec![0u32; kept.len()];
    let mut next = 0;
    for (f, k) in kept.iter().enumerate() {
        number[f] = next;
        next += (*k || f < imported) as u32;
    }

    let mut out = module[..8].to_vec();
    let mut write = |id: u8, body: &[u8]| {
        out.push(id);
        leb(&mut out, body.len() as u64);
        out.extend_from_slice(body);
    };
    let live = |k: usize| kept[imported + k];
    for &(id, body) in &base.sections {
        match id {
            SEC_FUNCTION => {
                let mut s = vec![];
                leb(&mut s, base.functions.iter().enumerate().filter(|(k, _)| live(*k)).count() as u64);
                for (_, t) in base.functions.iter().enumerate().filter(|(k, _)| live(*k)) {
                    leb(&mut s, *t as u64);
                }
                write(id, &s);
            }
            SEC_CODE => {
                let mut s = vec![];
                leb(&mut s, bodies.iter().enumerate().filter(|(k, _)| live(*k)).count() as u64);
                for (k, &(start, first, end)) in bodies.iter().enumerate() {
                    if !live(k) {
                        continue;
                    }
                    let mut f = code[start..first].to_vec();
                    let mut r = Reader::new(&code[..end]);
                    r.at = first;
                    let mut copied = first;
                    while r.at < end {
                        let mut seen = None;
                        walk(&mut r, |g, a, b| seen = Some((g, a, b)));
                        if let Some((g, a, b)) = seen {
                            f.extend_from_slice(&code[copied..a]);
                            leb(&mut f, number[g as usize] as u64);
                            copied = b;
                        }
                    }
                    f.extend_from_slice(&code[copied..end]);
                    leb(&mut s, f.len() as u64);
                    s.append(&mut f);
                }
                write(id, &s);
            }
            SEC_ELEMENT => {
                let mut r = Reader::new(body);
                let mut s = vec![];
                let n = r.number();
                leb(&mut s, n as u64);
                for _ in 0..n {
                    let start = r.at;
                    r.byte();
                    r.byte();
                    r.number();
                    r.byte();
                    s.extend_from_slice(&body[start..r.at]);
                    let items = r.number();
                    leb(&mut s, items as u64);
                    for _ in 0..items {
                        leb(&mut s, number[r.number() as usize] as u64);
                    }
                }
                write(id, &s);
            }
            SEC_EXPORT => {
                let mut r = Reader::new(body);
                let mut s = vec![];
                let n = r.number();
                leb(&mut s, n as u64);
                for _ in 0..n {
                    let start = r.at;
                    r.bytes();
                    let kind = r.byte();
                    s.extend_from_slice(&body[start..r.at]);
                    let k = r.number();
                    leb(&mut s, if kind == 0 { number[k as usize] } else { k } as u64);
                }
                write(id, &s);
            }
            _ => write(id, body),
        }
    }
    out
}

/// The page: the module in base 64 and runtime/web.js, in one file that needs no server.
pub fn page(module: &[u8], title: &str) -> Vec<u8> {
    let module = crate::crypto::base64(module, false);
    let title = title.replace('&', "&amp;").replace('<', "&lt;");
    include_str!("../runtime/web.html").replace("{title}", &title).replace("{script}", include_str!("../runtime/web.js")).replace("{module}", &module).into_bytes()
}

struct Generator<'g, 'b> {
    base: &'g mut Base<'b>,
    data: &'g mut Data,
    o: Vec<u8>,
    kinds: Vec<Scalar>,
    number: &'g HashMap<usize, u32>,
    /// The table numbers of the functions the program takes the address of.
    functions: &'g HashMap<usize, u32>,
    shows: [u32; 6],
    files: &'g [String],
    /// The local holding the next block's number; then scratch: an Int, a Float, two addresses.
    pc: u32,
    /// The `br` number of the loop around the cases, from the case being written.
    depth: u32,
    /// The `block`s, `loop`s and `if`s open, in `structured`.
    labels: Vec<Label>,
}

impl Generator<'_, '_> {
    fn op(&mut self, code: u8) {
        self.o.push(code);
    }
    fn get(&mut self, l: u32) {
        self.o.push(0x20);
        leb(&mut self.o, l as u64);
    }
    fn set(&mut self, l: u32) {
        self.o.push(0x21);
        leb(&mut self.o, l as u64);
    }
    fn int(&mut self, v: i64) {
        self.o.push(0x42);
        sleb(&mut self.o, v);
    }
    fn small(&mut self, v: i32) {
        self.o.push(0x41);
        sleb(&mut self.o, v as i64);
    }
    fn float(&mut self, v: f64) {
        self.o.push(0x44);
        self.o.extend_from_slice(&v.to_le_bytes());
    }
    fn begin_if(&mut self) {
        self.o.extend_from_slice(&[0x04, 0x40]);
    }
    fn end(&mut self) {
        self.o.push(0x0b);
    }
    fn br(&mut self, label: u32) {
        self.o.push(0x0c);
        leb(&mut self.o, label as u64);
    }
    fn take(&mut self, d: Local) {
        self.set(d as u32);
    }
    fn zero(&mut self, d: Local) {
        self.int(0);
        self.take(d);
    }

    /// A load or a store: its code, the power of two of its alignment, its offset.
    fn memory(&mut self, code: u8, align: u64, offset: u64) {
        self.o.push(code);
        leb(&mut self.o, align);
        leb(&mut self.o, offset);
    }

    /// Leaves on the stack whether the List's slots are words (else they are in `small`), and in
    /// local `pc + 4` the address of slot `k` of element `i`.
    fn list_slot(&mut self, l: Local, i: Local, k: usize) {
        let (list, at) = (self.pc + 3, self.pc + 4);
        self.get(l as u32);
        self.op(0xa7);
        self.set(list);
        self.get(list);
        self.memory(0x28, 2, 4);
        self.set(at);
        self.get(at);
        self.get(i as u32);
        self.op(0xa7);
        self.get(list);
        self.memory(0x28, 2, 8);
        self.op(0x6c);
        self.small(k as i32);
        self.op(0x6a);
        self.small(3);
        self.op(0x74);
        self.op(0x6a);
        self.set(at);
        self.get(list);
        self.memory(0x28, 2, 4);
    }

    /// Calls a runtime function; its result, if any, stays on the stack, as an Int or a Float.
    fn call(&mut self, name: &str, args: &[Arg]) -> Result<bool, String> {
        let Some((f, (params, results))) = self.base.runtime(name).map(|(f, t)| (f, t.clone())) else { return Err(t!("codegen_failed", error = format!("no {name} in the runtime"))) };
        if params.len() != args.len() {
            return Err(t!("codegen_failed", error = format!("{name} takes {} arguments, not {}", params.len(), args.len())));
        }
        for (a, &kind) in args.iter().zip(&params) {
            let given = match *a {
                Arg::Slot(l) => {
                    self.get(l as u32);
                    scalar(self.kinds[l])
                }
                Arg::Bits(l) => {
                    self.get(l as u32);
                    self.op(0xbd);
                    I64
                }
                Arg::Number(v) => {
                    self.int(v);
                    I64
                }
            };
            match (given, kind) {
                (I64, I32) => self.op(0xa7), // an address: 32 bits here
                (x, y) if x == y => {}
                (I64, F64) => self.op(0xbf),
                (F64, I64) => self.op(0xbd),
                _ => return Err(t!("codegen_failed", error = format!("{name}: an argument's type"))),
            }
        }
        self.o.push(0x10);
        leb(&mut self.o, f as u64);
        if results.first() == Some(&I32) {
            self.op(0xad);
        }
        Ok(!results.is_empty())
    }

    /// With the condition (32 bits) on the stack: if true, stops with this fault.
    fn fault_if(&mut self, fault: Fault, loc: Loc) -> Result<(), String> {
        self.begin_if();
        self.fault(fault, loc)?;
        self.end();
        Ok(())
    }

    fn fault(&mut self, fault: Fault, loc: Loc) -> Result<(), String> {
        self.call("kova_fault", &[Arg::Number(runtime_text(fault.key())), Arg::Number(loc.packed()), Arg::Number(loc.col as i64)])?;
        self.op(0x00);
        Ok(())
    }

    /// The declaration of the function's locals, then scratch locals.
    fn locals(&self, f: &Function, extra: usize) -> Vec<u8> {
        let mut head = vec![];
        let declared: Vec<u8> = f.locals[f.nb_params..].iter().map(|&t| scalar(t)).chain([I32, I64, F64, I32, I32]).chain(vec![I32; extra]).collect();
        let mut groups: Vec<(u32, u8)> = vec![];
        for t in declared {
            match groups.last_mut() {
                Some(g) if g.1 == t => g.0 += 1,
                _ => groups.push((1, t)),
            }
        }
        leb(&mut head, groups.len() as u64);
        for (n, t) in groups {
            leb(&mut head, n as u64);
            head.push(t);
        }
        head
    }

    /// The usual form: the blocks nested as the jumps between them ask.
    fn structured(&mut self, f: &Function, shape: &Shape) -> Result<Vec<u8>, String> {
        let mut head = self.locals(f, 0);
        self.tree(f, shape, shape.order[0])?;
        self.op(0x00); // never reached: every path returns
        self.end();
        head.append(&mut self.o);
        Ok(head)
    }

    fn tree(&mut self, f: &Function, s: &Shape, b: usize) -> Result<(), String> {
        if s.header[b] {
            self.o.extend_from_slice(&[0x03, 0x40]);
            self.labels.push(Label::Loop(b));
            self.within(f, s, b, 0)?;
            self.labels.pop();
            self.end();
            Ok(())
        } else {
            self.within(f, s, b, 0)
        }
    }

    /// Block `b`, with `block`s around it for the merge blocks from number `k` on: a jump to one
    /// is a `br` out of its `block`, right before its code.
    fn within(&mut self, f: &Function, s: &Shape, b: usize, k: usize) -> Result<(), String> {
        if let Some(&merge) = s.merges[b].get(k) {
            self.o.extend_from_slice(&[0x02, 0x40]);
            self.labels.push(Label::Block(merge));
            self.within(f, s, b, k + 1)?;
            self.labels.pop();
            self.end();
            return self.tree(f, s, merge);
        }
        for inst in &f.blocks[b].insts {
            self.inst(inst)?;
        }
        match &f.blocks[b].end {
            End::Jump(t) => self.go(f, s, b, *t),
            End::Branch(c, yes, no) => {
                self.get(*c as u32);
                self.int(0);
                self.op(0x52);
                self.begin_if();
                self.labels.push(Label::If);
                self.go(f, s, b, *yes)?;
                self.op(0x05);
                self.go(f, s, b, *no)?;
                self.labels.pop();
                self.end();
                Ok(())
            }
            End::Return(v) => {
                for &l in v {
                    self.get(l as u32);
                }
                self.op(0x0f);
                Ok(())
            }
            End::Unreachable => {
                self.op(0x00);
                Ok(())
            }
        }
    }

    fn go(&mut self, f: &Function, s: &Shape, from: usize, to: usize) -> Result<(), String> {
        let label = if s.place[to] <= s.place[from] {
            Label::Loop(to)
        } else if s.merge[to] {
            Label::Block(to)
        } else {
            return self.tree(f, s, to);
        };
        let depth = self.labels.iter().rev().position(|l| *l == label).expect("a label open around its jump");
        self.br(depth as u32);
        Ok(())
    }

    /// The form that can be left and entered again: a loop around a `br_table` on `pc`, one case
    /// per block, and a last case that saves the locals. `waits` tells the instructions after
    /// which the program may be waiting (each alone in its block, see `split`).
    fn resumable(&mut self, f: &Function, blocks: &[KBlock], waits: impl Fn(&Inst) -> bool) -> Result<Vec<u8>, String> {
        let saved = self.pc + 5;
        let mut head = self.locals(f, 1);
        let n = blocks.len() as u32;
        // Entered again: the block it was left at, its locals.
        self.call("kova_web_rewinding", &[])?;
        self.op(0xa7);
        self.begin_if();
        self.call("kova_web_pop", &[])?;
        self.op(0xa7);
        self.set(self.pc);
        for l in (0..f.locals.len()).rev() {
            self.call("kova_web_pop", &[])?;
            if f.locals[l] == Scalar::Float {
                self.op(0xbf);
            }
            self.set(l as u32);
        }
        self.end();
        self.o.extend_from_slice(&[0x03, 0x40]);
        for _ in 0..=n {
            self.o.extend_from_slice(&[0x02, 0x40]);
        }
        self.get(self.pc);
        self.op(0x0e);
        leb(&mut self.o, n as u64);
        for k in 0..=n {
            leb(&mut self.o, k as u64);
        }
        for (k, block) in blocks.iter().enumerate() {
            self.end();
            self.depth = n - k as u32;
            for inst in &block.insts {
                self.inst(inst)?;
                if waits(inst) {
                    self.call("kova_web_unwinding", &[])?;
                    self.op(0xa7);
                    self.begin_if();
                    self.depth += 1;
                    self.small(k as i32);
                    self.set(saved);
                    self.jump(n as usize);
                    self.depth -= 1;
                    self.end();
                }
            }
            match &block.end {
                End::Jump(s) => self.jump(*s),
                End::Branch(c, yes, no) => {
                    self.get(*c as u32);
                    self.int(0);
                    self.op(0x52);
                    self.begin_if();
                    self.depth += 1;
                    self.jump(*yes);
                    self.depth -= 1;
                    self.end();
                    self.jump(*no);
                }
                End::Return(v) => {
                    for &l in v {
                        self.get(l as u32);
                    }
                    self.op(0x0f);
                }
                End::Unreachable => self.op(0x00),
            }
        }
        // Waiting: the locals saved, then the block; the caller does the same.
        self.end();
        for l in 0..f.locals.len() {
            let arg = if f.locals[l] == Scalar::Float { Arg::Bits(l) } else { Arg::Slot(l) };
            self.call("kova_web_push", &[arg])?;
        }
        self.get(saved);
        self.op(0xad);
        self.set(self.pc + 1);
        self.get(self.pc + 1);
        let (push, _) = self.base.runtime("kova_web_push").map(|(f, t)| (f, t.clone())).expect("kova_web_push");
        self.o.push(0x10);
        leb(&mut self.o, push as u64);
        for &t in &f.returns {
            match t {
                Scalar::Int => self.int(0),
                Scalar::Float => self.float(0.0),
            }
        }
        self.op(0x0f);
        self.end();
        self.op(0x00); // never reached: the loop's end has no value to give
        self.end();
        head.append(&mut self.o);
        Ok(head)
    }

    fn jump(&mut self, to: usize) {
        self.small(to as i32);
        self.set(self.pc);
        self.br(self.depth);
    }

    fn inst(&mut self, inst: &Inst) -> Result<(), String> {
        match inst {
            Inst::Const(d, v) => {
                match self.kinds[*d] {
                    Scalar::Int => self.int(*v),
                    Scalar::Float => self.float(f64::from_bits(*v as u64)),
                }
                self.take(*d);
            }
            Inst::Copy(d, s) => {
                self.get(*s as u32);
                self.take(*d);
            }
            Inst::Binary(d, op, a, b, loc) => self.binary(*d, *op, *a as u32, *b as u32, *loc)?,
            Inst::Neg(d, a, loc) => {
                self.get(*a as u32);
                self.int(i64::MIN);
                self.op(0x51);
                self.fault_if(Fault::Overflow, *loc)?;
                self.int(0);
                self.get(*a as u32);
                self.op(0x7d);
                self.take(*d);
            }
            Inst::Not(d, a) => {
                self.get(*a as u32);
                self.int(1);
                self.op(0x85);
                self.take(*d);
            }
            Inst::FBinary(d, op, a, b, loc) => {
                let (a, b) = (*a as u32, *b as u32);
                if *op == Op::Div {
                    self.get(b);
                    self.float(0.0);
                    self.op(0x61);
                    self.fault_if(Fault::DivisionByZero, *loc)?;
                }
                self.get(a);
                self.get(b);
                let (code, compare) = match op {
                    Op::Plus => (0xa0, false),
                    Op::Minus => (0xa1, false),
                    Op::Times => (0xa2, false),
                    Op::Div => (0xa3, false),
                    Op::Eq => (0x61, true),
                    Op::Ne => (0x62, true),
                    Op::Lt => (0x63, true),
                    Op::Gt => (0x64, true),
                    Op::Le => (0x65, true),
                    Op::Ge => (0x66, true),
                    _ => unreachable!("refused on Floats by the type checker"),
                };
                self.op(code);
                if compare {
                    self.op(0xad);
                }
                self.take(*d);
            }
            Inst::FNeg(d, a) => {
                self.get(*a as u32);
                self.op(0x9a);
                self.take(*d);
            }
            Inst::FUnary(d, op, a) => {
                let a = *a as u32;
                self.get(a);
                match op {
                    FloatOp::Abs => self.op(0x99),
                    FloatOp::Sqrt => self.op(0x9f),
                    FloatOp::Floor => self.op(0x9c),
                    FloatOp::Ceil => self.op(0x9b),
                    // As in codegen.rs: the whole part, one further when the rest is at least a half.
                    FloatOp::Round => {
                        let t = self.pc + 2;
                        self.op(0x9d);
                        self.set(t);
                        self.get(t);
                        self.float(1.0);
                        self.get(a);
                        self.op(0xa6);
                        self.op(0xa0);
                        self.get(t);
                        self.get(a);
                        self.get(t);
                        self.op(0xa1);
                        self.op(0x99);
                        self.float(0.5);
                        self.op(0x66);
                        self.op(0x1b);
                    }
                }
                self.take(*d);
            }
            Inst::Math(d, m, x, y) => {
                self.call("kova_math", &[Arg::Number(*m as i64), Arg::Slot(*x), Arg::Slot(*y)])?;
                self.take(*d);
            }
            Inst::Convert(d, c, a, loc) => {
                let a = *a as u32;
                match c {
                    Conversion::IntToFloat => {
                        self.get(a);
                        self.op(0xb9);
                    }
                    Conversion::FromBits => {
                        self.get(a);
                        self.op(0xbf);
                    }
                    Conversion::ToBits => {
                        self.get(a);
                        self.op(0xbd);
                    }
                    Conversion::FloatToInt => {
                        self.get(a);
                        self.float(-9223372036854775808.0);
                        self.op(0x66);
                        self.get(a);
                        self.float(9223372036854775808.0);
                        self.op(0x63);
                        self.op(0x71);
                        self.op(0x45);
                        self.fault_if(Fault::BadConversion, *loc)?;
                        self.get(a);
                        self.op(0xb0);
                    }
                }
                self.take(*d);
            }
            Inst::Call(d, f, args) => {
                for &a in args {
                    self.get(a as u32);
                }
                self.o.push(0x10);
                leb(&mut self.o, self.number[f] as u64);
                for &r in d.iter().rev() {
                    self.take(r);
                }
            }
            Inst::FuncRef(d, f) => {
                self.int(self.functions[f] as i64);
                self.take(*d);
            }
            Inst::CallIndirect(d, f, args) => {
                for &a in args {
                    self.get(a as u32);
                }
                self.get(*f as u32);
                self.op(0xa7);
                let t = (args.iter().map(|&l| scalar(self.kinds[l])).collect(), d.iter().map(|&l| scalar(self.kinds[l])).collect());
                let t = self.base.type_number(t);
                self.o.push(0x11);
                leb(&mut self.o, t as u64);
                self.o.push(0);
                for &r in d.iter().rev() {
                    self.take(r);
                }
            }
            Inst::Text(d, t) => {
                let at = self.data.text(t);
                self.call("kova_str_new", &[Arg::Number(at as i64), Arg::Number(t.len() as i64)])?;
                self.take(*d);
            }
            Inst::Str(d, op, args) => {
                let given: Vec<Arg> = match op {
                    StrOp::Format(show) => {
                        let v = if *show == Show::Float { Arg::Bits(args[0]) } else { Arg::Slot(args[0]) };
                        vec![v, Arg::Number(self.shows[*show as usize] as i64)]
                    }
                    _ => args.iter().map(|&l| Arg::Slot(l)).collect(),
                };
                if !self.call(str_op(*op).1, &given)? {
                    self.int(0);
                }
                self.take(*d);
            }
            Inst::Free(l) => {
                self.call("kova_str_free", &[Arg::Slot(*l)])?;
                self.zero(*l);
            }
            Inst::ListNew(d, width) => {
                self.call("kova_list_new", &[Arg::Number(*width as i64)])?;
                self.take(*d);
            }
            // A List's first fields are read here, as in codegen.rs (32 bits each: `List` in
            // runtime/kova.rs). 0 = a List emptied by a move.
            Inst::ListLength(d, l) => {
                self.get(*l as u32);
                self.op(0x50);
                self.o.extend_from_slice(&[0x04, I64]);
                self.int(0);
                self.op(0x05);
                self.get(*l as u32);
                self.op(0xa7);
                self.memory(0x35, 2, 0);
                self.end();
                self.take(*d);
            }
            Inst::ListCheck(l, i, loc) => {
                self.get(*i as u32);
                self.get(*l as u32);
                self.op(0xa7);
                self.memory(0x35, 2, 0);
                self.op(0x5a); // unsigned: a negative index is a huge number
                self.begin_if();
                self.call("kova_list_check", &[Arg::Slot(*l), Arg::Slot(*i), Arg::Number(loc.packed()), Arg::Number(loc.col as i64)])?;
                self.end();
            }
            Inst::FaultIf(c, f, loc) => {
                self.get(*c as u32);
                self.int(0);
                self.op(0x52);
                self.fault_if(*f, *loc)?;
            }
            Inst::ListLoad(d, l, i, k) => {
                self.list_slot(*l, *i, *k);
                self.o.extend_from_slice(&[0x04, I64]);
                self.get(self.pc + 4);
                self.memory(0x29, 3, 0);
                self.op(0x05);
                self.call("kova_list_load", &[Arg::Slot(*l), Arg::Slot(*i), Arg::Number(*k as i64)])?;
                self.end();
                if self.kinds[*d] == Scalar::Float {
                    self.op(0xbf);
                }
                self.take(*d);
            }
            Inst::ListStore(l, i, k, value) => {
                let v = if self.kinds[*value] == Scalar::Float { Arg::Bits(*value) } else { Arg::Slot(*value) };
                self.list_slot(*l, *i, *k);
                self.begin_if();
                self.get(self.pc + 4);
                self.get(*value as u32);
                if self.kinds[*value] == Scalar::Float {
                    self.op(0xbd);
                }
                self.memory(0x37, 3, 0);
                self.op(0x05);
                self.call("kova_list_store", &[Arg::Slot(*l), Arg::Slot(*i), Arg::Number(*k as i64), v])?;
                self.end();
            }
            Inst::ListGrow(d, l) => {
                self.call("kova_list_grow", &[Arg::Slot(*l)])?;
                self.take(*d);
            }
            Inst::ListRemove(l, i) => {
                self.call("kova_list_remove", &[Arg::Slot(*l), Arg::Slot(*i)])?;
            }
            Inst::ListIndex(d, l) => {
                self.call("kova_list_index", &[Arg::Slot(*l)])?;
                self.take(*d);
            }
            Inst::ListSetIndex(l, v) => {
                self.call("kova_list_set_index", &[Arg::Slot(*l), Arg::Slot(*v)])?;
            }
            Inst::ListFree(l) => {
                self.call("kova_list_free", &[Arg::Slot(*l)])?;
                self.zero(*l);
            }
            // Like `interp::mix`.
            Inst::Mix(d, a, b) => {
                let t = self.pc + 1;
                self.get(*a as u32);
                self.get(*b as u32);
                self.op(0x85);
                self.set(t);
                for (shift, times) in [(30, 0xbf58476d1ce4e5b9u64 as i64), (27, 0x94d049bb133111ebu64 as i64)] {
                    self.get(t);
                    self.get(t);
                    self.int(shift);
                    self.op(0x88);
                    self.op(0x85);
                    self.int(times);
                    self.op(0x7e);
                    self.set(t);
                }
                self.get(t);
                self.get(t);
                self.int(31);
                self.op(0x88);
                self.op(0x85);
                self.int(1);
                self.op(0x88);
                self.take(*d);
            }
            Inst::Print { value, show, newline } => {
                let v = if *show == Show::Float { Arg::Bits(*value) } else { Arg::Slot(*value) };
                self.call("kova_print", &[v, Arg::Number(self.shows[*show as usize] as i64), Arg::Number(*newline as i64)])?;
            }
            Inst::Exit(code) => {
                self.call("kova_exit", &[Arg::Slot(*code)])?;
            }
            Inst::Fail(pieces) => {
                for m in pieces {
                    let t = match m {
                        Piece::Value(l, show) => {
                            let v = if *show == Show::Float { Arg::Bits(*l) } else { Arg::Slot(*l) };
                            self.call("kova_write_value", &[v, Arg::Number(self.shows[*show as usize] as i64)])?;
                            continue;
                        }
                        Piece::Label(k) => {
                            self.call("kova_write_label", &[Arg::Number(runtime_text(k))])?;
                            continue;
                        }
                        Piece::Text(t) => t.as_str(),
                        Piece::File(i) => self.files[*i].as_str(),
                    };
                    let at = self.data.text(t);
                    self.call("kova_write_text", &[Arg::Number(at as i64), Arg::Number(t.len() as i64)])?;
                }
                self.call("kova_fail", &[])?;
            }
            Inst::Line(..) | Inst::Threads(_) => {}
            Inst::Extern(..) | Inst::Callback(..) => return Err(t!("web_refuses", what = "extern")),
        }
        Ok(())
    }

    /// Same rules as `interp::compute`.
    fn binary(&mut self, d: Local, op: Op, a: u32, b: u32, loc: Loc) -> Result<(), String> {
        let t = self.pc + 1;
        let plain = |g: &mut Self, code: u8, wide: bool| {
            g.get(a);
            g.get(b);
            g.op(code);
            if wide {
                g.op(0xad);
            }
            g.take(d);
        };
        match op {
            // No overflow flag here: the sign of the result says.
            Op::Plus | Op::Minus => {
                self.get(a);
                self.get(b);
                self.op(if op == Op::Plus { 0x7c } else { 0x7d });
                self.set(t);
                self.get(a);
                self.get(if op == Op::Plus { t } else { b });
                self.op(0x85);
                self.get(if op == Op::Plus { b } else { a });
                self.get(t);
                self.op(0x85);
                self.op(0x83);
                self.int(0);
                self.op(0x53);
                self.fault_if(Fault::Overflow, loc)?;
                self.get(t);
                self.take(d);
            }
            // Two numbers of 32 bits cannot overflow; the others are asked of the runtime.
            Op::Times => {
                for x in [a, b] {
                    self.get(x);
                    self.int(1 << 31);
                    self.op(0x7c);
                }
                self.op(0x84);
                self.int(32);
                self.op(0x88);
                self.int(0);
                self.op(0x52);
                self.begin_if();
                self.call("kova_web_times_overflows", &[Arg::Slot(a as usize), Arg::Slot(b as usize)])?;
                self.int(0);
                self.op(0x52);
                self.fault_if(Fault::Overflow, loc)?;
                self.end();
                plain(self, 0x7e, false);
            }
            Op::Div | Op::Rem => {
                self.get(b);
                self.op(0x50);
                self.fault_if(Fault::DivisionByZero, loc)?;
                if op == Op::Div {
                    self.get(a);
                    self.int(i64::MIN);
                    self.op(0x51);
                    self.get(b);
                    self.int(-1);
                    self.op(0x51);
                    self.op(0x71);
                    self.fault_if(Fault::Overflow, loc)?;
                    plain(self, 0x7f, false);
                } else {
                    // x % -1 = 0; MIN % -1 would trap.
                    self.get(a);
                    self.int(1);
                    self.get(b);
                    self.get(b);
                    self.int(-1);
                    self.op(0x51);
                    self.op(0x1b);
                    self.op(0x81);
                    self.take(d);
                }
            }
            Op::BitOr => plain(self, 0x84, false),
            Op::BitXor => plain(self, 0x85, false),
            Op::BitAnd => plain(self, 0x83, false),
            Op::ShiftLeft | Op::ShiftRight => {
                self.get(b);
                self.int(63);
                self.op(0x56);
                self.fault_if(Fault::Shift, loc)?;
                plain(self, if op == Op::ShiftLeft { 0x86 } else { 0x87 }, false);
            }
            Op::Eq => plain(self, 0x51, true),
            Op::Ne => plain(self, 0x52, true),
            Op::Lt => plain(self, 0x53, true),
            Op::Le => plain(self, 0x57, true),
            Op::Gt => plain(self, 0x55, true),
            Op::Ge => plain(self, 0x59, true),
        }
        Ok(())
    }
}
