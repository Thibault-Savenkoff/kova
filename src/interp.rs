// Interpreter (`kova run`): executes the KIR directly, instruction by instruction.

use std::collections::VecDeque;
use std::io::{BufRead, BufWriter, Write};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::debug::{Ask, Debugger, Stack};

use crate::kir::*;
use crate::locale::{self, t};

/// What a String or List slot points to.
enum Object {
    Str(String),
    /// `length` elements, their slots one after the other (`width` slots each). A Map or a Set
    /// also has an `index` (a List, made by `kir.rs` `find`; 0 = none), like in runtime/kova.rs.
    List { width: usize, length: usize, slots: Vec<i64>, index: i64, small: Option<Vec<u8>> },
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
    /// The state of `random` (splitmix64, like runtime/kova.rs), once started.
    chance: Option<u64>,
    /// Under `kova debug`: what stops the program and shows its variables.
    debugger: Option<Box<Debugger>>,
    /// While the debugger shows a variable: a String or a List already given away (slot 0) or
    /// freed was read (`broken`), as an empty one (`nothing`).
    looking: bool,
    broken: std::cell::Cell<bool>,
    nothing: (usize, Vec<i64>),
    /// `serve`'s sockets: a listener's number is its place here, a connection's too.
    listeners: Vec<crate::http::Server<crate::web::System>>,
    /// Each question being answered: its connection, then who asked, its method and its URL if
    /// it is written when answered.
    links: Vec<Option<(std::net::TcpStream, Option<([u8; 4], String, String)>)>>,
    /// `log.level(l)`, once called; `KOVA_LOG`'s level, once read.
    log_level: Option<i64>,
    log_env: Option<Option<i64>>,
    /// `ui.typed`: what the field that changed last holds.
    ui_typed: String,
    /// `ui.draw`: the canvas being drawn.
    ui_drawing: String,
    /// `ui.act`: what the window must do.
    ui_acts: Vec<i64>,
    /// `kova debug`: its threads.
    threads: Threads,
}

// ---- `kova debug` follows threads (PLAN Part 4 "Threads under the debugger") ----

/// runtime/kova.rs `Pipe`: a queue of boxes.
struct Pipe {
    items: VecDeque<i64>,
    limit: i64,
    closed: bool,
    refs: i64,
    free: usize,
}

/// Each Kova thread is a thread of `kova debug`, but one runs at a time, until it waits or hands
/// over: all share the `Heap`, and a stop shows one place in the program.
#[derive(Default)]
struct Threads {
    /// A pipe's handle is its place here + 1.
    pipes: Vec<Pipe>,
    me: usize,
    /// `None`: ended. What the debugger follows for the threads that do not run.
    stacks: Vec<Option<Stack>>,
    /// How many times in a row a thread found it had to wait: when each did, nothing moves.
    idle: usize,
    /// One of those has a time limit.
    timed: bool,
    waited_at: Loc,
    /// How the program stopped, for the first thread.
    stop: Option<Stop>,
}

impl Threads {
    fn pipe(&mut self, h: i64) -> &mut Pipe {
        &mut self.pipes[h as usize - 1]
    }
}

/// Whose turn it is.
static TURN: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

fn site(packed: i64, col: i64) -> Loc {
    Loc { file: (packed >> 32) as usize, line: (packed & 0xffff_ffff) as usize, col: col as usize }
}

fn hand_over(to: usize) {
    *TURN.0.lock().unwrap() = to;
    TURN.1.notify_all();
}

fn wait_for_turn(me: usize) {
    let mut turn = TURN.0.lock().unwrap();
    while *turn != me {
        turn = TURN.1.wait(turn).unwrap();
    }
}

/// The running thread gives its turn to the next one; back here at its next turn.
fn next_turn(heap: &mut Heap) -> Result<(), Stop> {
    let t = &mut heap.threads;
    let (me, n) = (t.me, t.stacks.len());
    let Some(next) = (1..n).map(|k| (me + k) % n).find(|&k| t.stacks[k].is_some()) else { return Ok(()) };
    switch(heap, next);
    hand_over(next);
    wait_for_turn(me);
    match heap.threads.stop.take() {
        Some(stop) if me == 0 => Err(stop),
        // Stopped by another: the program ends without this thread.
        Some(_) => loop {
            std::thread::park();
        },
        None => Ok(()),
    }
}

/// The debugger now follows thread `next`.
fn switch(heap: &mut Heap, next: usize) {
    let t = &mut heap.threads;
    if let Some(d) = &mut heap.debugger {
        t.stacks[t.me].iter_mut().for_each(|s| d.swap(s));
        t.stacks[next].iter_mut().for_each(|s| d.swap(s));
    }
    t.me = next;
}

/// The running thread cannot go on before another has done something.
fn wait(timed: bool, at: Loc, out: &mut dyn Write, files: &[String], heap: &mut Heap) -> Result<(), Stop> {
    let _ = out.flush();
    let t = &mut heap.threads;
    t.timed |= timed;
    t.waited_at = at;
    t.idle += 1;
    if t.idle > t.stacks.iter().flatten().count() {
        if !t.timed || heap.looking {
            return Err(Stop::Error(fault_text(Fault::Deadlock, files, at)));
        }
        (t.idle, t.timed) = (0, false);
        std::thread::sleep(Duration::from_millis(1));
    }
    next_turn(heap)
}

/// The end of `main`: a program ends when all its threads have.
fn every_thread_ended(out: &mut dyn Write, files: &[String], heap: &mut Heap) -> Result<(), Stop> {
    while heap.threads.stacks.iter().flatten().count() > 1 {
        wait(false, heap.threads.waited_at, out, files, heap)?;
    }
    Ok(())
}

/// What a new thread uses of what `kova debug` runs: it stays until the program ends, and only the
/// thread whose turn it is touches it.
struct Lent {
    program: *const Program,
    out: *mut (dyn Write + 'static),
    files: *const [String],
    heap: *mut Heap,
}
unsafe impl Send for Lent {}

fn start_thread(p: &Program, function: usize, args: [i64; 2], out: &mut dyn Write, files: &[String], heap: &mut Heap) {
    let out: *mut (dyn Write + '_) = out;
    let lent = Lent { program: p, out: unsafe { std::mem::transmute(out) }, files, heap };
    heap.threads.stacks.push(Some(Stack::default()));
    heap.threads.idle = 0;
    let me = heap.threads.stacks.len() - 1;
    let body = move || {
        let lent = lent;
        wait_for_turn(me);
        let (p, out, files, heap) = unsafe { (&*lent.program, &mut *lent.out, &*lent.files, &mut *lent.heap) };
        let result = call(p, function, &args, out, files, heap);
        let t = &mut heap.threads;
        t.stacks[me] = None;
        t.idle = 0;
        if let Err(stop) = result {
            t.stop = Some(stop);
            hand_over(0);
            return;
        }
        let n = t.stacks.len();
        let next = (1..n).map(|k| (me + k) % n).find(|&k| t.stacks[k].is_some()).expect("the first thread waits for the others");
        switch(heap, next);
        hand_over(next);
    };
    // Like the interpreter's own stack (src/main.rs), smaller: there may be many.
    std::thread::Builder::new().stack_size(1 << 27).spawn(body).expect("a thread for the debugger");
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
            Some(Object::List { width, length, slots, small, .. }) => {
                if let Some(b) = small.take() {
                    *length = b.len();
                    *slots = b.iter().map(|&b| b as i64).collect();
                }
                (*width, length, slots)
            }
            _ => panic!("List used after being freed"),
        }
    }
    /// The bytes of the List `h`, while it is held that way (`runtime/kova.rs` `List::small`).
    fn small(&mut self, h: i64) -> Option<&mut Vec<u8>> {
        match self.objects.get_mut(h as usize) {
            Some(Some(Object::List { small, .. })) => small.as_mut(),
            _ => None,
        }
    }
    fn length(&mut self, h: i64) -> usize {
        match self.small(h) {
            Some(b) => b.len(),
            None => *self.list(h).1,
        }
    }
    fn bytes(&mut self, h: i64) -> Vec<u8> {
        match self.small(h) {
            Some(b) => b.clone(),
            None => self.list(h).2.iter().map(|&x| x as u8).collect(),
        }
    }
    fn byte_list(&mut self, b: Vec<u8>) -> i64 {
        self.add(Object::List { width: 1, length: 0, slots: vec![], index: 0, small: Some(b) })
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

/// `random(from..to)`: the state is started at the first call.
/// Fills `b` with the system's unguessable chance (`secure.random`), like `sys::chance` in the
/// runtime.
#[cfg(unix)]
fn chance(b: &mut [u8]) {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(b)).expect("/dev/urandom");
}

/// Whether `s.red` paints (`shared::colors`); on Windows, asks the console to read the colors.
fn colors() -> bool {
    use std::io::IsTerminal;
    let terminal = std::io::stdout().is_terminal();
    #[cfg(windows)]
    if terminal {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetStdHandle(n: u32) -> isize;
            fn GetConsoleMode(h: isize, mode: *mut u32) -> i32;
            fn SetConsoleMode(h: isize, mode: u32) -> i32;
        }
        // STD_OUTPUT_HANDLE, ENABLE_VIRTUAL_TERMINAL_PROCESSING.
        unsafe {
            let (h, mut mode) = (GetStdHandle(-11i32 as u32), 0);
            GetConsoleMode(h, &mut mode);
            SetConsoleMode(h, mode | 4);
        }
    }
    crate::shared::colors(terminal, |k| std::env::var(k).ok())
}

#[cfg(windows)]
fn chance(b: &mut [u8]) {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: isize, b: *mut u8, n: u32, flags: u32) -> i32;
    }
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG
    assert_eq!(unsafe { BCryptGenRandom(0, b.as_mut_ptr(), b.len() as u32, 2) }, 0, "BCryptGenRandom");
}

/// `List<Book>.from_csv(text)` (src/csv.rs): a new List, or the problem's code.
fn csv_read(heap: &mut Heap, text: i64, spec: i64) -> i64 {
    let (text, spec) = (heap.get(text).to_string(), heap.get(spec).to_string());
    let strings = |heap: &mut Heap, parts: Vec<String>| {
        let slots: Vec<i64> = parts.into_iter().map(|p| heap.make(p)).collect();
        heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0, small: None })
    };
    if spec == "*" {
        return match crate::csv::rows(&text) {
            Ok((rows, _)) => {
                let rows: Vec<i64> = rows.into_iter().map(|(_, r)| strings(heap, r)).collect();
                heap.add(Object::List { width: 1, length: rows.len(), slots: rows, index: 0, small: None })
            }
            Err(p) => p.code(),
        };
    }
    match crate::csv::records(&text, &spec) {
        Ok(cells) => {
            let width: usize = crate::csv::fields(&spec).iter().map(|f| 1 + f.optional as usize).sum();
            let slots: Vec<i64> = cells.into_iter().map(|c| match c {
                crate::csv::Slot::Int(n) => n,
                crate::csv::Slot::Text(t) => heap.make(t),
            }).collect();
            heap.add(Object::List { width, length: slots.len() / width.max(1), slots, index: 0, small: None })
        }
        Err(p) => p.code(),
    }
}

/// `books.to_csv separator(c)`: a new String.
fn csv_write(heap: &mut Heap, l: i64, spec: i64, sep: i64) -> i64 {
    let spec = heap.get(spec).to_string();
    let sep = char::from_u32(sep as u32).unwrap_or(',');
    let (width, length, slots) = heap.list(l);
    let slots = slots[..width * *length].to_vec();
    let text = if spec == "*" {
        let rows: Vec<Vec<String>> = slots.iter().map(|&r| {
            let cells = heap.list(r).2.clone();
            cells.iter().map(|&c| heap.get(c).to_string()).collect()
        }).collect();
        crate::csv::write_rows(&rows, sep)
    } else {
        crate::csv::write_records(&spec, &slots, &|s| heap.get(s).to_string(), sep)
    };
    heap.make(text)
}

/// A pattern of `s.matches(…)`, `s.find_all(…)`…: the checker made sure it is one.
fn pattern(p: &str) -> crate::regex::Regex {
    crate::regex::compile(p).unwrap_or_else(|_| unreachable!("pattern checked by the type checker"))
}

fn random(heap: &mut Heap, from: i64, to: i64) -> i64 {
    let state = heap.chance.get_or_insert_with(|| {
        crate::shared::seed(std::env::var("KOVA_SEED").ok().as_deref()).unwrap_or_else(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            (t.as_secs().wrapping_mul(1_000_000_000).wrapping_add(t.subsec_nanos() as u64)) ^ (std::process::id() as u64) << 32
        })
    });
    crate::shared::random(state, from, to)
}

/// A failure of the system as an `IoError` variant + 1, like `failure` in the runtime.
fn failure(e: &std::io::Error) -> i64 {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound | NotADirectory => 1,
        PermissionDenied | ReadOnlyFilesystem => 2,
        IsADirectory => 3,
        _ => 5,
    }
}

/// `system.run`, as runtime/kova.rs `kova_run`: the program found the same way, its input empty.
fn run_program(name: &str, words: &[String]) -> Result<(String, String, i64), i64> {
    if name.contains('\0') || words.iter().any(|w| w.contains('\0')) {
        return Err(3);
    }
    let path = std::env::var_os("PATH").map(|p| p.to_string_lossy().into_owned());
    let is_file = |p: &str| std::fs::metadata(p).is_ok_and(|m| !m.is_dir());
    let program = crate::shared::find_program(name, path.as_deref(), cfg!(windows), &is_file)?;
    let ran = std::process::Command::new(&program).args(words).stdin(std::process::Stdio::null()).output();
    let ran = ran.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => 1,
        std::io::ErrorKind::PermissionDenied => 2,
        _ => 3,
    })?;
    #[cfg(unix)]
    let code = {
        use std::os::unix::process::ExitStatusExt;
        crate::shared::exit_code(ran.status.into_raw())
    };
    #[cfg(windows)]
    let code = ran.status.code().unwrap_or(0) as u32 as i64;
    Ok((String::from_utf8_lossy(&ran.stdout).into_owned(), String::from_utf8_lossy(&ran.stderr).into_owned(), code))
}

/// `files.copy(a) to(b)`, as runtime/kova.rs `copy_file`: a failure past 16 is `b`'s.
fn copy_file(a: &str, b: &str) -> i64 {
    if std::path::Path::new(a).is_dir() {
        return -3;
    }
    if std::path::Path::new(b).is_dir() {
        return -(3 + 16);
    }
    match std::fs::read(a) {
        Ok(bytes) => std::fs::write(b, bytes).map_or_else(|e| -(failure(&e) + 16), |()| 0),
        Err(e) => -failure(&e),
    }
}

/// `files.move(a) to(b)`, as runtime/kova.rs `kova_move_file`.
fn move_file(a: &str, b: &str) -> i64 {
    if let Err(e) = std::fs::metadata(a) {
        return -failure(&e);
    }
    if std::path::Path::new(b).is_dir() {
        return -(3 + 16);
    }
    // EXDEV, ERROR_NOT_SAME_DEVICE: on another disk.
    let elsewhere = if cfg!(windows) { 17 } else { 18 };
    match std::fs::rename(a, b) {
        Ok(()) => 0,
        Err(e) if e.raw_os_error() == Some(elsewhere) => match copy_file(a, b) {
            0 => std::fs::remove_file(a).map_or_else(|e| -failure(&e), |()| 0),
            f => f,
        },
        Err(e) => -(failure(&e) + 16),
    }
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
        // What is still alive is not counted as a leak (`kova_exit` in runtime/kova.rs).
        Err(Stop::Exit(code)) => {
            alive(heap.live);
            code
        }
    }
}

/// `KOVA_LEAKS=live`: `exit` writes how many Strings and Lists are still alive, the same at each
/// `exit` of a server that frees what each question made (test `server`). Like runtime/kova.rs.
fn alive(live: usize) {
    if std::env::var("KOVA_LEAKS").is_ok_and(|m| m == "live") {
        eprintln!("alive: {live}");
    }
}

/// `kova debug`: runs `main` with the debugger (`debug.rs`); the program's output goes to it.
/// Returns the exit code (101 after a runtime error), and the debugger back.
pub fn debug(p: &Program, files: &[String], args: &[String], debugger: Debugger) -> (i32, Box<Debugger>) {
    let files = &crate::debug::files(files)[..];
    let mut out = crate::debug::Console::default();
    let mut heap = Heap { args: args.to_vec(), debugger: Some(Box::new(debugger)), ..Heap::default() };
    heap.threads.stacks.push(Some(Stack::default()));
    let result = call(p, p.main.expect("checked by debug.rs"), &[], &mut out, files, &mut heap);
    let result = result.and_then(|r| every_thread_ended(&mut out, files, &mut heap).map(|_| r));
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
    while let Some(ask) = d.asked() {
        let frame = match &ask {
            Ask::Variables(f) | Ask::Evaluate(f, _) => *f,
        };
        let at = d.frames.get(frame).and_then(|f| f.at);
        let values = d.frames.get(frame).map_or(vec![], |f| f.values.clone());
        if let Ask::Evaluate(_, expression) = &ask {
            let result = evaluate(p, at, &values, expression, &d, files, heap);
            d.evaluated(result);
            continue;
        }
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

/// What the console shows for `expression`, with the values of the variables of a frame
/// (stopped at `at`): what `print` would write.
fn evaluate(
    p: &Program,
    at: Option<(usize, usize, usize, Loc)>,
    values: &[i64],
    expression: &str,
    d: &Debugger,
    files: &[String],
    heap: &mut Heap,
) -> Result<String, String> {
    // A function held in a variable is `p`'s number: the console's own functions come after `p`'s.
    let named: Vec<_> = watches(p, at).iter().map(|w| (w.name.clone(), w.ty.clone(), w.slots.len())).collect();
    let (mut q, num) = d.evaluation(&named, expression)?;
    let num = q.after(p) + num;
    let mut printed = vec![];
    heap.looking = true;
    heap.broken.set(false);
    let result = call(&q, num, values, &mut printed, files, heap);
    heap.looking = false;
    if heap.broken.get() {
        return Ok(t!("debug_given_away"));
    }
    match result {
        Ok(_) => Ok(String::from_utf8_lossy(&printed).trim_end_matches('\n').to_string()),
        // Its first line only: the place after it is in the function `evaluation` added.
        Err(Stop::Error(message)) => Err(message.lines().next().unwrap_or("").to_string()),
        Err(Stop::Exit(code)) => Err(format!("exit({code})")),
    }
}

/// Same check as `leaks` in `runtime/kova.rs`.
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

// ponytail: Rust recursion = the interpreter's stack (1 GB, src/main.rs); an infinite recursion
// crashes without a nice message, as in native (code 134 here, 139 there). To handle with stack errors.
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
                // Only before an `Inst::Extern`, which stops the interpreter.
                Inst::Callback(d, ..) => locals[*d] = 0,
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
                Inst::Math(d, m, x, y) => locals[*d] = bits(crate::shared::math(*m as i64, float(locals[*x]), float(locals[*y]))),
                Inst::FUnary(d, op, a) => {
                    let x = float(locals[*a]);
                    locals[*d] = bits(match op {
                        FloatOp::Abs => x.abs(),
                        FloatOp::Sqrt => x.sqrt(),
                        FloatOp::Floor => x.floor(),
                        FloatOp::Ceil => x.ceil(),
                        FloatOp::Round => x.round(),
                    })
                }
                Inst::Convert(d, c, a, loc) => {
                    locals[*d] = match c {
                        Conversion::IntToFloat => bits(locals[*a] as f64),
                        Conversion::FromBits | Conversion::ToBits => locals[*a],
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
                        StrOp::Hash => crate::shared::hash(s(0)),
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
                        StrOp::ReadBytes if std::path::Path::new(s(0)).is_dir() => -3,
                        StrOp::ReadBytes => match std::fs::read(s(0)) {
                            Ok(b) => heap.byte_list(b),
                            Err(e) => -failure(&e),
                        },
                        StrOp::WriteBytes if std::path::Path::new(s(0)).is_dir() => -3,
                        StrOp::WriteBytes => {
                            let path = s(0).to_string();
                            let bytes = heap.bytes(locals[a[1]]);
                            let mut file = std::fs::OpenOptions::new();
                            file.create(true);
                            if locals[a[2]] != 0 { file.append(true) } else { file.write(true).truncate(true) };
                            match file.open(path).and_then(|mut f| f.write_all(&bytes)) {
                                Ok(()) => 0,
                                Err(e) => -failure(&e),
                            }
                        }
                        StrOp::NotBytes => (heap.small(locals[a[0]]).is_none() && heap.list(locals[a[0]]).2.iter().any(|x| !(0..256).contains(x))) as i64,
                        StrOp::BytesText => {
                            let bytes = heap.bytes(locals[a[0]]);
                            crate::shared::bytes_text(locals[a[1]], &bytes).map_or(0, |t| heap.make(t))
                        }
                        StrOp::TextBytes => match crate::shared::text_bytes(locals[a[1]], s(0)) {
                            Some(b) => heap.byte_list(b),
                            None => 0,
                        },
                        StrOp::WriteFile | StrOp::AppendFile if std::path::Path::new(s(0)).is_dir() => -3,
                        StrOp::WriteFile | StrOp::AppendFile => {
                            let mut file = std::fs::OpenOptions::new();
                            file.create(true);
                            if matches!(op, StrOp::AppendFile) { file.append(true) } else { file.write(true).truncate(true) };
                            match file.open(s(0)).and_then(|mut f| f.write_all(s(1).as_bytes())) {
                                Ok(()) => 0,
                                Err(e) => -failure(&e),
                            }
                        }
                        StrOp::Exists => (!s(0).contains('\0') && std::path::Path::new(s(0)).exists()) as i64,
                        StrOp::IsFolder => (!s(0).contains('\0') && std::path::Path::new(s(0)).is_dir()) as i64,
                        StrOp::CopyFile => copy_file(s(0), s(1)),
                        StrOp::MoveFile => move_file(s(0), s(1)),
                        StrOp::FileSize => match std::fs::metadata(s(0)) {
                            Ok(m) if m.is_dir() => -3,
                            Ok(m) => m.len() as i64,
                            Err(e) => -failure(&e),
                        },
                        StrOp::Modified => match std::fs::metadata(s(0)).and_then(|m| m.modified()) {
                            Ok(t) => {
                                let utc = t.duration_since(std::time::UNIX_EPOCH).map_or_else(|e| -(e.duration().as_secs() as i64), |d| d.as_secs() as i64);
                                heap.add(Object::List { width: 1, length: 1, slots: vec![local_at(utc)], index: 0, small: None })
                            }
                            Err(e) => -failure(&e),
                        },
                        StrOp::Matching => {
                            let list = |d: &str| std::fs::read_dir(d).ok()?.map(|e| e.ok()?.file_name().into_string().ok()).collect::<Option<Vec<String>>>();
                            let kind = |p: &str| {
                                let m = std::fs::symlink_metadata(p).ok()?;
                                Some((if m.is_symlink() { std::path::Path::new(p).is_dir() } else { m.is_dir() }, m.is_symlink()))
                            };
                            match crate::shared::matching(s(0), &list, &kind) {
                                Ok(found) => {
                                    let slots: Vec<i64> = found.into_iter().map(|n| heap.make(n)).collect();
                                    heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0, small: None })
                                }
                                Err(f) => -f,
                            }
                        }
                        StrOp::Run => {
                            let name = s(0).to_string();
                            let words: Vec<String> = match locals[a[1]] {
                                0 => vec![],
                                l => heap.list(l).2.clone().into_iter().map(|w| heap.get(w).to_string()).collect(),
                            };
                            match run_program(&name, &words) {
                                Ok((output, errors, code)) => {
                                    let slots = vec![heap.make(output), heap.make(errors), code];
                                    heap.add(Object::List { width: 3, length: 1, slots, index: 0, small: None })
                                }
                                Err(f) => -f,
                            }
                        }
                        StrOp::Env if s(0).is_empty() || s(0).contains(['=', '\0']) => 0,
                        StrOp::Env => match std::env::var_os(s(0)) {
                            Some(v) => heap.make(v.to_string_lossy().into_owned()),
                            None => 0,
                        },
                        StrOp::ListDir => match std::fs::read_dir(s(0)) {
                            Err(e) => -failure(&e),
                            Ok(entries) => {
                                let names: Result<Vec<String>, _> = entries.map(|e| e.map_err(|_| -5).and_then(|e| e.file_name().into_string().map_err(|_| -4))).collect();
                                match names {
                                    Err(code) => code,
                                    Ok(mut names) => {
                                        names.sort();
                                        let slots: Vec<i64> = names.into_iter().map(|n| heap.make(n)).collect();
                                        heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0, small: None })
                                    }
                                }
                            }
                        },
                        StrOp::JsonParse => match crate::json::parse(s(0)) {
                            Err(line) => -line,
                            Ok(nodes) => {
                                let slots = crate::json::slots(nodes, |t| heap.make(t));
                                heap.add(Object::List { width: 4, length: slots.len() / 4, slots, index: 0, small: None })
                            }
                        },
                        StrOp::Listen => {
                            use crate::http::Serve;
                            let (everyone, log) = (locals[a[1]] != 0, locals[a[3]]);
                            match u16::try_from(locals[a[0]]).ok().filter(|&p| p > 0).and_then(|p| crate::web::System.listen(p, everyone).map(|l| (p, l))) {
                                None => -1,
                                Some((port, l)) => {
                                    heap.listeners.push(crate::http::Server::new(l, log == 0));
                                    let title = match locals[a[2]] {
                                        0 => t!("serving", port = port),
                                        f => t!("serving_files", folder = crate::http::shown_folder(heap.get(f)), port = port),
                                    };
                                    let (computer, network, stop) = (if everyone { t!("this_computer") } else { t!("this_computer_only") }, t!("this_network"), t!("stop_with_ctrl_c"));
                                    let said = crate::http::Said { title: &title, this_computer: &computer, this_network: &network, stop: &stop };
                                    let addresses = if everyone { crate::web::System.addresses() } else { vec![] };
                                    if log < 2 {
                                        out.flush().unwrap();
                                        eprint!("{}", crate::http::banner(port, everyone, &addresses, &said));
                                    }
                                    heap.listeners.len() as i64 - 1
                                }
                            }
                        }
                        StrOp::Accept => {
                            out.flush().unwrap();
                            let server = &mut heap.listeners[locals[a[0]] as usize];
                            let (link, q, from) = crate::http::next_question(&mut crate::web::System, server);
                            let asked = server.logs.then(|| (from, q.method.clone(), q.url.clone()));
                            heap.links.push(Some((link, asked)));
                            let conn = heap.links.len() as i64 - 1;
                            let mut headers = vec![];
                            for (n, v) in q.headers {
                                headers.push(heap.make(n));
                                headers.push(heap.make(v));
                            }
                            let length = headers.len() / 2;
                            let headers = heap.add(Object::List { width: 2, length, slots: headers, index: 0, small: None });
                            let slots = vec![heap.make(q.method), heap.make(q.url), headers, heap.make(q.body), 0, 0, conn];
                            heap.add(Object::List { width: 7, length: 1, slots, index: 0, small: None })
                        }
                        StrOp::Reply => {
                            let slots = heap.list(locals[a[2]]).2.clone();
                            let headers: Vec<(String, String)> = slots.chunks(2).map(|p| (heap.get(p[0]).to_string(), heap.get(p[1]).to_string())).collect();
                            let headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
                            let (link, asked) = heap.links[locals[a[0]] as usize].take().expect("answered once");
                            let (body, file) = (heap.get(locals[a[3]]), heap.get(locals[a[4]]));
                            let status = crate::http::reply(&mut crate::web::System, link, locals[a[1]], &headers, body, file, |p| std::fs::read(p).ok());
                            if let Some((from, method, url)) = asked {
                                out.flush().unwrap();
                                eprint!("{}", crate::http::logged(local_now(), from, &method, &url, status));
                            }
                            0
                        }
                        StrOp::UrlPath => {
                            let t = crate::http::url_path(s(0));
                            heap.make(t)
                        }
                        StrOp::UrlQuery => match crate::http::url_query(s(0), s(1)) {
                            None => 0,
                            Some(t) => heap.make(t),
                        },
                        StrOp::StatusText => heap.make(crate::http::status_text(locals[a[0]])),
                        StrOp::FileToServe => match crate::http::file_to_serve(s(0), |p| std::fs::metadata(p).ok().map(|m| m.is_dir())) {
                            None => 0,
                            Some(t) => heap.make(t),
                        },
                        StrOp::PipeNew => {
                            let (limit, free) = (locals[a[0]], locals[a[1]] as usize);
                            heap.threads.pipes.push(Pipe { items: VecDeque::new(), limit, closed: false, refs: 1, free });
                            heap.threads.pipes.len() as i64
                        }
                        StrOp::PipeRetain => {
                            if locals[a[0]] != 0 {
                                heap.threads.pipe(locals[a[0]]).refs += 1;
                            }
                            locals[a[0]]
                        }
                        StrOp::PipeRelease => {
                            let h = locals[a[0]];
                            if h != 0 {
                                let pipe = heap.threads.pipe(h);
                                pipe.refs -= 1;
                                if pipe.refs == 0 {
                                    let free = pipe.free;
                                    for b in std::mem::take(&mut pipe.items) {
                                        call(p, free, &[b], out, files, heap)?;
                                    }
                                }
                            }
                            0
                        }
                        StrOp::PipePut => {
                            let (h, b, at) = (locals[a[0]], locals[a[1]], site(locals[a[2]], locals[a[3]]));
                            loop {
                                let pipe = heap.threads.pipe(h);
                                if pipe.closed {
                                    return Err(Stop::Error(fault_text(Fault::Closed, files, at)));
                                }
                                if pipe.limit == 0 || (pipe.items.len() as i64) < pipe.limit {
                                    break;
                                }
                                wait(false, at, out, files, heap)?;
                            }
                            heap.threads.pipe(h).items.push_back(b);
                            heap.threads.idle = 0;
                            // A thread that loops on `s.read` leaves the others a turn.
                            next_turn(heap)?;
                            0
                        }
                        StrOp::PipeTake => {
                            let (h, at) = (locals[a[0]], site(locals[a[1]], locals[a[2]]));
                            loop {
                                let pipe = heap.threads.pipe(h);
                                if let Some(b) = pipe.items.pop_front() {
                                    heap.threads.idle = 0;
                                    break b;
                                }
                                if pipe.closed {
                                    break 0;
                                }
                                wait(false, at, out, files, heap)?;
                            }
                        }
                        StrOp::PipeClose => {
                            heap.threads.pipe(locals[a[0]]).closed = true;
                            heap.threads.idle = 0;
                            0
                        }
                        StrOp::PipeSelect => {
                            let (l, at) = (locals[a[0]], site(locals[a[2]], locals[a[3]]));
                            let time = f64::from_bits(locals[a[1]] as u64);
                            let end = (time >= 0.0).then(|| Instant::now() + Duration::from_secs_f64(time.min(1e9)));
                            let pipes = heap.list(l).2.clone();
                            loop {
                                if let Some((k, b)) = pipes.iter().enumerate().find_map(|(k, &h)| Some((k, heap.threads.pipe(h).items.pop_front()?))) {
                                    heap.list(l).2[0] = b;
                                    heap.threads.idle = 0;
                                    break k as i64;
                                }
                                if pipes.iter().all(|&h| heap.threads.pipe(h).closed) {
                                    break pipes.len() as i64 + 1;
                                }
                                if end.is_some_and(|end| Instant::now() >= end) {
                                    break pipes.len() as i64;
                                }
                                wait(end.is_some(), at, out, files, heap)?;
                            }
                        }
                        StrOp::CharTest => crate::shared::char_test(locals[a[0]], locals[a[1]]),
                        StrOp::Cores => std::thread::available_parallelism().map_or(1, |n| n.get() as i64),
                        StrOp::Spawn => {
                            start_thread(p, locals[a[0]] as usize, [locals[a[1]], locals[a[2]]], out, files, heap);
                            0
                        }
                        StrOp::Workers => unreachable!("built only: `kova run` answers one question at a time"),
                        StrOp::Parallel | StrOp::Next => unreachable!("built only: `kova run` does one element at a time"),
                        StrOp::JsonQuote => {
                            let t = crate::json::quote(s(0));
                            heap.make(t)
                        }
                        StrOp::Web => {
                            let (url, method, body) = (s(0).to_string(), s(1).to_string(), s(2).to_string());
                            let mut headers = vec![];
                            if locals[a[4]] != 0 {
                                let slots = heap.list(locals[a[4]]).2.clone();
                                headers = slots.chunks(2).map(|p| (heap.get(p[0]).to_string(), heap.get(p[1]).to_string())).collect();
                            }
                            let r = crate::http::Request {
                                method: &method,
                                url: &url,
                                body: &body,
                                headers: headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect(),
                                follow: locals[a[3]] != 0,
                                ms: crate::http::milliseconds(f64::from_bits(locals[a[5]] as u64)),
                            };
                            match crate::http::exchange(&mut crate::web::network(), &r) {
                                Err(f) => -(f.number() + 1),
                                Ok(answer) => {
                                    // As the runtime makes it: a List of one `Response`.
                                    let mut headers = vec![];
                                    for (n, v) in answer.headers {
                                        headers.push(heap.make(n));
                                        headers.push(heap.make(v));
                                    }
                                    let length = headers.len() / 2;
                                    let headers = heap.add(Object::List { width: 2, length, slots: headers, index: 0, small: None });
                                    let slots = vec![answer.status, heap.make(answer.url), heap.make(answer.body), headers];
                                    heap.add(Object::List { width: 4, length: 1, slots, index: 0, small: None })
                                }
                            }
                        }
                        StrOp::MakeDir if s(0).is_empty() => -1,
                        StrOp::MakeDir => match std::fs::create_dir_all(s(0)) {
                            Ok(()) => 0,
                            Err(_) if std::path::Path::new(s(0)).exists() => -5,
                            Err(e) => -failure(&e),
                        },
                        StrOp::RemoveFile if std::path::Path::new(s(0)).is_dir() => -3,
                        StrOp::RemoveFile => match std::fs::remove_file(s(0)) {
                            Ok(()) => 0,
                            Err(e) => -failure(&e),
                        },
                        // Windows answers "denied" for a directory: asked first, like `kova_read_file` in runtime/kova.rs.
                        StrOp::ReadFile if std::path::Path::new(s(0)).is_dir() => -3,
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
                        // Spaces = the same 5 characters as `kova_str_trim` in runtime/kova.rs.
                        StrOp::Trim => {
                            let t = s(0).trim_matches(|c: char| c.is_ascii_whitespace()).to_string();
                            heap.make(t)
                        }
                        StrOp::ToBigInt => match crate::bigint::parse(s(0)) {
                            Some(t) => heap.make(t),
                            None => 0,
                        },
                        StrOp::Reversed => {
                            let t: String = s(0).chars().rev().collect();
                            heap.make(t)
                        }
                        StrOp::ToBool => match s(0) {
                            "true" => 1,
                            "false" => 0,
                            _ => 2,
                        },
                        StrOp::Contains => s(0).contains(s(1)) as i64,
                        StrOp::StartsWith => s(0).starts_with(s(1)) as i64,
                        StrOp::EndsWith => s(0).ends_with(s(1)) as i64,
                        StrOp::Replace => {
                            let t = s(0).replace(s(1), s(2));
                            heap.make(t)
                        }
                        StrOp::Fixed => {
                            let t = crate::shared::fixed(f64::from_bits(locals[a[0]] as u64), locals[a[1]]);
                            heap.make(t)
                        }
                        StrOp::RoundTo => crate::shared::round_to(f64::from_bits(locals[a[0]] as u64), locals[a[1]]).to_bits() as i64,
                        StrOp::Split | StrOp::Words | StrOp::Lines => {
                            let parts: Vec<String> = match op {
                                StrOp::Split => s(0).split(s(1)).map(String::from).collect(),
                                StrOp::Lines => s(0).lines().map(String::from).collect(),
                                _ => s(0).split_ascii_whitespace().map(String::from).collect(),
                            };
                            let slots: Vec<i64> = parts.into_iter().map(|p| heap.make(p)).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0, small: None })
                        }
                        StrOp::Args => {
                            let slots: Vec<i64> = heap.args.clone().into_iter().map(|a| heap.make(a)).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0, small: None })
                        }
                        StrOp::ToUpper | StrOp::ToLower => {
                            let t: String = s(0).chars().map(|c| crate::kir::case_of(c, matches!(op, StrOp::ToUpper))).collect();
                            heap.make(t)
                        }
                        StrOp::Find => s(0).find(s(1)).map_or(-1, |i| s(0)[..i].chars().count() as i64),
                        StrOp::Slice => {
                            let (from, to) = (locals[a[1]] as usize, locals[a[2]] as usize);
                            let t: String = s(0).chars().skip(from).take(to - from).collect();
                            heap.make(t)
                        }
                        StrOp::Wrap => {
                            let t = crate::shared::wrapped(s(0), locals[a[1]]);
                            heap.make(t)
                        }
                        StrOp::Matches => crate::regex::matches(&pattern(s(1)), s(0)) as i64,
                        StrOp::ReplaceMatches => {
                            let t = crate::regex::replace_all(&pattern(s(1)), s(0), s(2));
                            heap.make(t)
                        }
                        StrOp::AllMatches | StrOp::Groups => {
                            let found = match op {
                                StrOp::AllMatches => Some(crate::regex::find_all(&pattern(s(1)), s(0))),
                                _ => crate::regex::groups(&pattern(s(1)), s(0)),
                            };
                            match found {
                                Some(parts) => {
                                    let slots: Vec<i64> = parts.into_iter().map(|p| heap.make(p)).collect();
                                    heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0, small: None })
                                }
                                None => 0,
                            }
                        }
                        StrOp::Mean | StrOp::Median => {
                            let (_, length, slots) = heap.list(locals[a[0]]);
                            let values = slots[..*length].to_vec();
                            let f = if matches!(op, StrOp::Mean) { crate::shared::mean } else { crate::shared::median };
                            f(&values, locals[a[1]] == 1).to_bits() as i64
                        }
                        StrOp::SecureRandom => crate::shared::secure_random(&mut |b| chance(b), locals[a[0]], locals[a[1]]),
                        StrOp::SecureToken => {
                            let mut b = vec![0; locals[a[0]] as usize];
                            chance(&mut b);
                            heap.make(crate::shared::hex(&b))
                        }
                        StrOp::Paint => {
                            static COLORS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                            let t = crate::shared::painted(s(0), locals[a[1]], *COLORS.get_or_init(colors));
                            heap.make(t)
                        }
                        // Only before an `Inst::Extern`, which stops the interpreter.
                        StrOp::CText | StrOp::CFree | StrOp::FromCText | StrOp::ExternAddress | StrOp::CWideText | StrOp::CWideFree | StrOp::FromCWideText | StrOp::CRead => 0,
                        // Only the tests reach it: `kova run` runs a `ui` program as native code.
                        StrOp::UiShow => {
                            writeln!(out, "[{}] {}", s(0), crate::shared::ui_describe(s(1))).unwrap();
                            let acts = std::mem::take(&mut heap.ui_acts);
                            for act in &acts {
                                writeln!(out, "({})", crate::shared::ui_act_name(*act)).unwrap();
                            }
                            out.flush().unwrap();
                            let mut line = String::new();
                            match std::env::var_os("KOVA_UI_SCRIPT").is_some() && heap.debugger.is_none() && !acts.contains(&1) {
                                true if std::io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) => {
                                    let (answer, typed) = crate::shared::ui_answer(&line);
                                    if let Some(t) = typed {
                                        heap.ui_typed = t;
                                    }
                                    answer
                                }
                                _ => -1,
                            }
                        }
                        StrOp::UiTyped => {
                            let t = heap.ui_typed.clone();
                            heap.make(t)
                        }
                        StrOp::UiAct => {
                            heap.ui_acts.push(locals[a[0]]);
                            0
                        }
                        // As the runtime before a window has told it anything.
                        StrOp::UiPlace => {
                            heap.ui_acts.push(6);
                            0
                        }
                        StrOp::UiRoom => match locals[a[0]] as usize {
                            which @ 0..=2 => [320, 200, 0][which],
                            which => crate::shared::UI_AREAS_GUESS[which - 3] as i64,
                        },
                        StrOp::UiDraw => {
                            let op = s(0).to_string();
                            heap.ui_drawing.push_str(&op);
                            0
                        }
                        StrOp::UiDrawn => {
                            let t = core::mem::take(&mut heap.ui_drawing);
                            heap.make(t)
                        }
                        StrOp::LogLevel => {
                            heap.log_level = Some(locals[a[0]]);
                            0
                        }
                        StrOp::Log => {
                            let text = s(1).to_string();
                            out.flush().unwrap();
                            let env = *heap.log_env.get_or_insert_with(|| {
                                let named = std::env::var("KOVA_LOG").ok();
                                crate::shared::level_named(named.as_deref()).unwrap_or_else(|()| {
                                    let said = t!("log_unknown_level", name = named.unwrap_or_default());
                                    eprint!("{}", crate::shared::log_line(local_now(), 2, &said));
                                    None
                                })
                            });
                            if locals[a[0]] >= env.or(heap.log_level).unwrap_or(1) {
                                eprint!("{}", crate::shared::log_line(local_now(), locals[a[0]], &text));
                            }
                            0
                        }
                        StrOp::Uuid => {
                            let mut b = [0; 16];
                            chance(&mut b);
                            heap.make(crate::shared::uuid(b))
                        }
                        StrOp::Sha256 | StrOp::Hmac | StrOp::ToBase64 | StrOp::ToBase64Url | StrOp::PasswordHash | StrOp::SecureKey => {
                            use crate::crypto;
                            let t = match op {
                                StrOp::Sha256 => crate::shared::hex(&crypto::sha256(s(0).as_bytes())),
                                StrOp::Hmac => crate::shared::hex(&crypto::hmac_sha256(s(1).as_bytes(), s(0).as_bytes())),
                                StrOp::ToBase64 => crypto::base64(s(0).as_bytes(), false),
                                StrOp::ToBase64Url => crypto::base64(s(0).as_bytes(), true),
                                StrOp::PasswordHash => {
                                    let mut salt = [0; 16];
                                    chance(&mut salt);
                                    crypto::password_hash(s(0), &salt, locals[a[1]] as u32, locals[a[2]] as u32)
                                }
                                _ => {
                                    let mut key = [0; 32];
                                    chance(&mut key);
                                    crypto::base64(&key, false)
                                }
                            };
                            heap.make(t)
                        }
                        StrOp::PasswordMatches => crate::crypto::password_matches(s(0), s(1)) as i64,
                        StrOp::FromBase64 | StrOp::FromBase64Url | StrOp::Encrypt | StrOp::Decrypt => {
                            use crate::crypto;
                            let t = match op {
                                StrOp::FromBase64 | StrOp::FromBase64Url => crypto::from_base64(s(0), matches!(op, StrOp::FromBase64Url)).and_then(|b| String::from_utf8(b).ok()),
                                StrOp::Encrypt => {
                                    let mut nonce = [0; 24];
                                    chance(&mut nonce);
                                    crypto::encrypt(s(0), s(1), &nonce)
                                }
                                _ => crypto::decrypt(s(0), s(1)),
                            };
                            t.map_or(0, |t| heap.make(t))
                        }
                        StrOp::FileSha256 => match std::fs::read(s(0)) {
                            _ if std::path::Path::new(s(0)).is_dir() => -3,
                            Ok(b) => heap.make(crate::shared::hex(&crate::crypto::sha256(&b))),
                            Err(e) => match e.kind() {
                                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => -1,
                                std::io::ErrorKind::PermissionDenied => -2,
                                std::io::ErrorKind::IsADirectory => -3,
                                _ => -5,
                            },
                        },
                        StrOp::CsvRead => csv_read(heap, locals[a[0]], locals[a[1]]),
                        StrOp::CsvWrite => csv_write(heap, locals[a[0]], locals[a[1]], locals[a[2]]),
                        StrOp::Padded | StrOp::PaddedLeft => {
                            let t = crate::shared::padded(s(0), locals[a[1]], matches!(op, StrOp::PaddedLeft));
                            heap.make(t)
                        }
                        StrOp::Repeat => {
                            let t = s(0).repeat(locals[a[1]].max(0) as usize);
                            heap.make(t)
                        }
                        StrOp::Join => {
                            let parts: Vec<String> = heap.list(locals[a[0]]).2.clone().into_iter().map(|w| heap.get(w).to_string()).collect();
                            let t = parts.join(heap.get(locals[a[1]]));
                            heap.make(t)
                        }
                        StrOp::LocalNow => local_now(),
                        StrOp::ToDate => crate::shared::parse_date(s(0), false).unwrap_or(i64::MIN),
                        StrOp::ToDateTime => crate::shared::parse_date(s(0), true).unwrap_or(i64::MIN),
                        StrOp::Now => {
                            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                            (t.as_secs() as f64 + t.subsec_nanos() as f64 / 1e9).to_bits() as i64
                        }
                        StrOp::Sleep => {
                            out.flush().unwrap();
                            let s = float(locals[a[0]]);
                            if s > 0.0 && s < 1e9 {
                                std::thread::sleep(std::time::Duration::from_secs_f64(s));
                            }
                            0
                        }
                        StrOp::Random => random(heap, locals[a[0]], locals[a[1]]),
                        StrOp::IsFloat => crate::kir::parse_float(s(0)).is_some() as i64,
                        StrOp::ToFloat => crate::kir::parse_float(s(0)).unwrap_or(0.0).to_bits() as i64,
                        StrOp::Chars => {
                            let slots: Vec<i64> = s(0).chars().map(|c| c as i64).collect();
                            heap.add(Object::List { width: 1, length: slots.len(), slots, index: 0, small: None })
                        }
                    }
                }
                Inst::Free(l) | Inst::ListFree(l) => {
                    heap.release(locals[*l]);
                    locals[*l] = 0;
                }
                Inst::ListNew(d, width) => {
                    locals[*d] = heap.add(Object::List { width: *width, length: 0, slots: vec![], index: 0, small: (*width == 1).then(Vec::new) });
                }
                // 0 = a List emptied by a move: its helper (`Drop`) still asks its length.
                Inst::ListLength(d, l) => locals[*d] = if locals[*l] == 0 { 0 } else { heap.length(locals[*l]) as i64 },
                Inst::FaultIf(c, f, loc) if locals[*c] != 0 => return Err(fault(*f, loc)),
                Inst::FaultIf(..) => {}
                // `kova run` runs such a program as native code (main.rs `native_run`).
                Inst::Extern(_, _, _, loc) => return Err(fault(Fault::ExternInterpreted, loc)),
                Inst::Threads(loc) if heap.looking => return Err(fault(Fault::ThreadsInterpreted, loc)),
                Inst::Threads(_) => {}
                Inst::ListCheck(l, i, loc) => {
                    let length = heap.length(locals[*l]) as i64;
                    if !(0..length).contains(&locals[*i]) {
                        return Err(fault(Fault::Index, loc));
                    }
                }
                Inst::ListLoad(d, l, i, k) => {
                    if let Some(b) = heap.small(locals[*l]) {
                        locals[*d] = b[locals[*i] as usize] as i64;
                        continue;
                    }
                    let (width, _, slots) = heap.list(locals[*l]);
                    locals[*d] = slots[locals[*i] as usize * width + k];
                }
                Inst::ListStore(l, i, k, v) => {
                    let value = locals[*v];
                    if (0..256).contains(&value)
                        && let Some(b) = heap.small(locals[*l])
                    {
                        b[locals[*i] as usize] = value as u8;
                        continue;
                    }
                    let (width, _, slots) = heap.list(locals[*l]);
                    slots[locals[*i] as usize * width + k] = value;
                }
                Inst::ListGrow(d, l) => {
                    if let Some(b) = heap.small(locals[*l]) {
                        b.push(0);
                        locals[*d] = b.len() as i64 - 1;
                        continue;
                    }
                    let (width, length, slots) = heap.list(locals[*l]);
                    slots.resize(slots.len() + width, 0);
                    *length += 1;
                    locals[*d] = *length as i64 - 1;
                }
                Inst::ListRemove(l, i) => {
                    let index = *heap.index(locals[*l]);
                    if index != 0 {
                        crate::shared::unindex(heap.list(index).2, locals[*i]);
                    }
                    let i = locals[*i] as usize;
                    if let Some(b) = heap.small(locals[*l]) {
                        b.remove(i);
                        continue;
                    }
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
        Op::Rem | Op::BitOr | Op::BitXor | Op::BitAnd | Op::ShiftLeft | Op::ShiftRight => unreachable!("refused on Floats by the type checker"),
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
        Op::BitOr => a | b,
        Op::BitXor => a ^ b,
        Op::BitAnd => a & b,
        Op::ShiftLeft | Op::ShiftRight if !(0..64).contains(&b) => return Err(Fault::Shift),
        Op::ShiftLeft => a << b,
        Op::ShiftRight => a >> b,
    })
}

/// `DateTime.now`: the local time in seconds since 1970-01-01 00:00, as the runtime's
/// `kova_local_now` gives it.
#[cfg(not(windows))]
fn local_now() -> i64 {
    let utc = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_secs() as i64);
    local_at(utc)
}

/// The local time at `utc` (seconds since 1970), as runtime/kova.rs `local_at`.
#[cfg(not(windows))]
fn local_at(utc: i64) -> i64 {
    utc + crate::shared::local_offset(utc, std::env::var("TZ").ok().as_deref(), &|p| std::fs::read(p).ok())
}

/// By the rules of that day, as runtime/windows.rs `local_at`.
#[cfg(windows)]
fn local_at(utc: i64) -> i64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FileTimeToSystemTime(t: *const u64, s: *mut [u16; 8]) -> i32;
        fn SystemTimeToTzSpecificLocalTime(zone: *const u8, utc: *const [u16; 8], local: *mut [u16; 8]) -> i32;
        fn SystemTimeToFileTime(s: *const [u16; 8], t: *mut u64) -> i32;
    }
    let t = ((utc + 11_644_473_600) * 10_000_000) as u64;
    let (mut s, mut local, mut back) = ([0u16; 8], [0u16; 8], 0u64);
    unsafe {
        if FileTimeToSystemTime(&t, &mut s) == 0 || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &s, &mut local) == 0 || SystemTimeToFileTime(&local, &mut back) == 0 {
            return utc;
        }
    }
    (back / 10_000_000) as i64 - 11_644_473_600
}

/// By the time zone of Windows' settings.
#[cfg(windows)]
fn local_now() -> i64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemTimePreciseAsFileTime(t: *mut u64);
        fn FileTimeToLocalFileTime(utc: *const u64, local: *mut u64) -> i32;
    }
    let (mut utc, mut local) = (0u64, 0u64);
    unsafe {
        GetSystemTimePreciseAsFileTime(&mut utc);
        FileTimeToLocalFileTime(&utc, &mut local);
    }
    (local / 10_000_000) as i64 - 11_644_473_600
}
