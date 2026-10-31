// Debugger (`kova debug`): the editor talks to it with the DAP protocol (Debug Adapter Protocol,
// the debugging twin of LSP), on stdin/stdout. It runs the program in the interpreter, which
// asks it at each statement whether to stop (`interp::line`).
//
//   editor ──"stop at line 12"──► kova debug ──runs──► interp ── line 12 ──► stopped
//          ◄──"stopped" + variables─────────────────────────────┘
//
// The KIR is translated with `watching` on: an `Inst::Line` before each statement says where it
// is and which variables are visible; `kova run` and `kova build` never see them.
//
// While stopped, the console evaluates what is typed (`rex.age + 1`): `evaluation` adds to the
// program a function taking the visible variables, borrowed, that prints it.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, channel};

use serde_json::{Value, json};

use crate::kir::Loc;
use crate::locale::t;
use crate::{analyze, diag, interp, kir};

/// A function being run: its name, the statement it is at (function, block, instruction, place)
/// and the values of the variables visible there, copied at that statement or before a call.
pub struct Frame {
    pub name: String,
    pub at: Option<(usize, usize, usize, Loc)>,
    pub values: Vec<i64>,
}

/// What the editor asks while the program is stopped, about a frame.
pub enum Ask {
    Variables(usize),
    /// What the console shows for this expression.
    Evaluate(usize, String),
}

/// The function `evaluation` adds; its name cannot clash with a program's own.
const EVAL: &str = "kova_debug_console";

/// Where to stop next, besides the breakpoints.
#[derive(Clone, Copy, PartialEq, Default)]
enum Step {
    #[default]
    Run,
    Pause,
    /// "Step over": the next statement of this function or of a caller (at most this many frames).
    Over(usize),
    Into,
    /// "Step out": the next statement of a caller (fewer frames than this).
    Out(usize),
}

pub struct Debugger {
    requests: Receiver<Value>,
    /// File (as `canonical` gives it) → the lines to stop at.
    breakpoints: HashMap<String, Vec<usize>>,
    step: Step,
    pub frames: Vec<Frame>,
    /// A runtime error was already shown (the frames it goes up through don't stop again).
    errored: bool,
    /// The `variables` request being answered.
    asked: Option<Value>,
    /// Ctrl+F5 ("run without debugging"): never stops.
    quiet: bool,
    /// The program run (its path and text), for `evaluation`.
    path: String,
    source: String,
}

/// What the debugger follows for each thread: only the running one's is in `Debugger`.
#[derive(Default)]
pub struct Stack {
    frames: Vec<Frame>,
    step: Step,
}

impl Debugger {
    /// Another thread runs: a step stays in the thread it was asked in.
    pub fn swap(&mut self, stack: &mut Stack) {
        std::mem::swap(&mut self.frames, &mut stack.frames);
        std::mem::swap(&mut self.step, &mut stack.step);
    }
}

pub fn serve() -> i32 {
    let (sender, requests) = channel();
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        while let Some(msg) = receive(&mut input) {
            if sender.send(msg).is_err() {
                return;
            }
        }
        // The editor is gone.
        let _ = sender.send(json!({ "type": "request", "seq": 0, "command": "disconnect" }));
    });
    let mut d = Debugger {
        requests,
        breakpoints: HashMap::new(),
        step: Step::Run,
        frames: vec![],
        errored: false,
        asked: None,
        quiet: false,
        path: String::new(),
        source: String::new(),
    };

    // Until the editor has said what to run and where to stop.
    let (mut launch, mut ready) = (None, false);
    while launch.is_none() || !ready {
        let req = d.next();
        match req["command"].as_str().unwrap_or("") {
            "initialize" => {
                respond(
                    &req,
                    json!({ "supportsConfigurationDoneRequest": true, "supportsTerminateRequest": true, "supportsEvaluateForHovers": true }),
                );
                event("initialized", json!({}));
            }
            "launch" => {
                let a = &req["arguments"];
                let args: Vec<String> = a["args"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
                d.quiet = a["noDebug"].as_bool().unwrap_or(false);
                launch = Some((a["program"].as_str().unwrap_or("").to_string(), args));
                respond(&req, json!({}));
            }
            "configurationDone" => {
                ready = true;
                respond(&req, json!({}));
            }
            _ => d.handle(&req),
        }
    }
    let (path, args) = launch.unwrap();
    let code = match program(&path) {
        Ok((program, files)) => {
            (d.path, d.source) = (path.clone(), std::fs::read_to_string(&path).unwrap_or_default());
            let (code, back) = interp::debug(&program, &files, &args, d);
            d = *back;
            code
        }
        Err(message) => {
            output("stderr", &message);
            1
        }
    };
    event("exited", json!({ "exitCode": code }));
    event("terminated", json!({}));
    // The editor still asks a few things, then says goodbye (`disconnect` exits).
    loop {
        let req = d.next();
        d.handle(&req);
    }
}

/// The program `path`, translated for the debugger, and the paths of its files; or its errors.
fn program(path: &str) -> Result<(kir::Program, Vec<String>), String> {
    let source = std::fs::read_to_string(path).map_err(|e| t!("cannot_read", file = path, error = e) + "\n")?;
    let (tree, info, diags, sources) = analyze(path, &source);
    let info = match info {
        Some(info) if diags.is_empty() => info,
        _ => return Err(diags.iter().map(|d| diag::render(d, &sources) + "\n").collect()),
    };
    if !tree.functions.iter().any(|f| f.name.text == "main") {
        return Err(diag::render(&diag::Diag::new(diag::Span::new(0, 0), t!("main_missing")), &sources));
    }
    let program = kir::translate(&tree, &info, &sources.text, true, false, false, true);
    Ok((program, sources.files.iter().map(|(p, _)| p.clone()).collect()))
}

impl Debugger {
    /// The next request of the editor (waits for it).
    fn next(&mut self) -> Value {
        self.requests.recv().unwrap_or_else(|_| std::process::exit(0))
    }

    pub fn enter(&mut self, name: &str) {
        self.frames.push(Frame { name: name.to_string(), at: None, values: vec![] });
    }

    pub fn leave(&mut self) {
        self.frames.pop();
    }

    /// True the first time a runtime error goes up: the debugger stops there.
    pub fn first_error(&mut self) -> bool {
        !std::mem::replace(&mut self.errored, true) && !self.quiet
    }

    /// At a statement: why to stop here (`breakpoint`, `step`, `pause`), or `None`. Meanwhile,
    /// answers what the editor asked while the program was running.
    pub fn stops(&mut self, loc: Loc, files: &[String]) -> Option<&'static str> {
        while let Ok(req) = self.requests.try_recv() {
            self.handle(&req);
        }
        if self.quiet {
            return None;
        }
        let depth = self.frames.len();
        // `files` are the canonical paths (`debug::files`): nothing to ask the system here.
        if self.breakpoints.get(&files[loc.file]).is_some_and(|lines| lines.contains(&loc.line)) {
            return Some("breakpoint");
        }
        match self.step {
            Step::Pause => Some("pause"),
            Step::Into => Some("step"),
            Step::Over(n) if depth <= n => Some("step"),
            Step::Out(n) if depth < n => Some("step"),
            _ => None,
        }
    }

    /// Tells the editor the program stopped (`text`: the message of a runtime error).
    pub fn stopped(&mut self, reason: &str, text: Option<&str>) {
        self.step = Step::Run;
        event("stopped", json!({ "reason": reason, "threadId": 1, "allThreadsStopped": true, "text": text, "description": text }));
    }

    /// While stopped: answers the editor until it asks something about a function being run
    /// (`Some`) or to go on (`None`).
    pub fn asked(&mut self) -> Option<Ask> {
        loop {
            let req = self.next();
            let depth = self.frames.len();
            let step = match req["command"].as_str().unwrap_or("") {
                "variables" => {
                    let frame = req["arguments"]["variablesReference"].as_u64().unwrap_or(1) as usize - 1;
                    self.asked = Some(req);
                    return Some(Ask::Variables(frame));
                }
                // Without a frame (the console's first line): the innermost function.
                "evaluate" => {
                    let frame = req["arguments"]["frameId"].as_u64().map_or(depth.saturating_sub(1), |f| f as usize);
                    let expression = req["arguments"]["expression"].as_str().unwrap_or("").to_string();
                    self.asked = Some(req);
                    return Some(Ask::Evaluate(frame, expression));
                }
                "continue" => Step::Run,
                "next" => Step::Over(depth),
                "stepIn" => Step::Into,
                "stepOut" => Step::Out(depth),
                _ => {
                    self.handle(&req);
                    continue;
                }
            };
            self.step = step;
            respond(&req, json!({ "allThreadsContinued": true }));
            return None;
        }
    }

    /// The program with a function `EVAL` printing `expression`, taking the variables `visible`
    /// ((name, type) of each), and its number; or why it cannot be.
    // ponytail: the whole program is checked again at each evaluation (fine for programs of a
    // few thousand lines).
    pub fn evaluation(&self, visible: &[(String, String, usize)], expression: &str) -> Result<(kir::Program, usize), String> {
        // `self` cannot name a parameter of a plain function: it becomes `kova_self`, in the
        // expression too (texts included: `"{self.name}"`).
        let rename = |n: &str| if n == "self" { "kova_self".to_string() } else { n.to_string() };
        // A function kept in a variable (1 slot) stays one: written directly, its type would be a
        // block's (2 slots).
        let mut aliases = String::new();
        let mut typed = |(i, (n, t, slots)): (usize, &(String, String, usize))| {
            if t.starts_with('{') && *slots == 1 {
                aliases += &format!("type KovaDebug{i} := {t}\n");
                return format!("{}: KovaDebug{i}", rename(n));
            }
            format!("{}: {t}", rename(n))
        };
        let params: Vec<String> = visible.iter().enumerate().map(&mut typed).collect();
        let params = if params.is_empty() { String::new() } else { format!("({})", params.join(", ")) };
        let mut code = String::new();
        let mut rest = expression.trim();
        while let Some(at) = rest.find("self") {
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let alone = !word(rest[..at].chars().next_back()) && !word(rest[at + 4..].chars().next());
            code += &rest[..at];
            code += if alone { "kova_self" } else { "self" };
            rest = &rest[at + 4..];
        }
        code += rest;
        let source = format!("{}\n{aliases}fn {EVAL}{params} {{\n    print({code})\n}}\n", self.source);
        let (tree, info, diags, _) = analyze(&self.path, &source);
        let info = match (info, diags.first()) {
            (Some(info), None) => info,
            (_, Some(d)) => return Err(d.message.clone()),
            (None, None) => return Err(t!("debug_not_yet")),
        };
        let program = kir::translate(&tree, &info, &source, true, false, false, false);
        let num = program.functions.iter().position(|f| f.name == EVAL).expect("just added");
        Ok((program, num))
    }

    /// The answer to `Ask::Evaluate`: what the console shows, or the error.
    pub fn evaluated(&mut self, result: Result<String, String>) {
        let Some(req) = self.asked.take() else { return };
        match result {
            Ok(text) => respond(&req, json!({ "result": text, "variablesReference": 0 })),
            Err(message) => fail(&req, &message),
        }
    }

    /// The answer to `Ask::Variables`: (name, type, value as text) of each variable.
    pub fn variables(&mut self, shown: Vec<(String, String, String)>) {
        let variables: Vec<Value> =
            shown.into_iter().map(|(name, ty, value)| json!({ "name": name, "value": value, "type": ty, "variablesReference": 0 })).collect();
        if let Some(req) = self.asked.take() {
            respond(&req, json!({ "variables": variables }));
        }
    }

    /// The requests answered the same way whether the program runs or not.
    fn handle(&mut self, req: &Value) {
        let a = &req["arguments"];
        match req["command"].as_str().unwrap_or("") {
            "threads" => respond(req, json!({ "threads": [{ "id": 1, "name": "main" }] })),
            "setBreakpoints" => {
                let lines: Vec<usize> = a["breakpoints"].as_array().into_iter().flatten().filter_map(|b| b["line"].as_u64()).map(|l| l as usize).collect();
                let verified: Vec<Value> = lines.iter().map(|l| json!({ "verified": true, "line": l })).collect();
                self.breakpoints.insert(canonical(a["source"]["path"].as_str().unwrap_or("")), lines);
                respond(req, json!({ "breakpoints": verified }));
            }
            "setExceptionBreakpoints" | "setFunctionBreakpoints" => respond(req, json!({ "breakpoints": [] })),
            "pause" => {
                self.step = Step::Pause;
                respond(req, json!({}));
            }
            // The innermost function first; one that hasn't reached its first statement isn't shown.
            "stackTrace" => {
                let frames: Vec<Value> = (self.frames.iter().enumerate().rev())
                    .filter_map(|(i, f)| Some((i, f, f.at?.3)))
                    .map(|(i, f, loc)| json!({ "id": i, "name": f.name, "line": loc.line, "column": loc.col, "source": source(loc) }))
                    .collect();
                respond(req, json!({ "stackFrames": frames, "totalFrames": frames.len() }));
            }
            "scopes" => {
                let frame = a["frameId"].as_u64().unwrap_or(0);
                respond(req, json!({ "scopes": [{ "name": t!("debug_variables"), "variablesReference": frame + 1, "expensive": false }] }));
            }
            "disconnect" | "terminate" => {
                respond(req, json!({}));
                std::process::exit(0);
            }
            // Not answered while running: the editor asks again once stopped.
            "variables" | "continue" | "next" | "stepIn" | "stepOut" => respond(req, json!({ "variables": [] })),
            "evaluate" => fail(req, &t!("debug_evaluate_running")),
            _ => fail(req, &t!("debug_not_yet")),
        }
    }
}

/// The source of a place, for the editor: its file's path.
fn source(loc: Loc) -> Value {
    FILES.with(|f| {
        let path = f.borrow().get(loc.file).cloned().unwrap_or_default();
        json!({ "name": std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()), "path": path })
    })
}

thread_local! {
    /// The paths of the program's files, by number (`Loc::file`), as absolute paths.
    static FILES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(vec![]) };
}

/// The path as the editor writes it (absolute, links followed), to compare breakpoints' files.
fn canonical(path: &str) -> String {
    std::fs::canonicalize(path).map_or(path.to_string(), |p| p.to_string_lossy().into_owned())
}

/// Called by `interp::debug` before running: the files' paths as the editor writes them, for
/// the call stack and the breakpoints.
pub fn files(files: &[String]) -> Vec<String> {
    let paths: Vec<String> = files.iter().map(|p| canonical(p)).collect();
    FILES.with(|f| *f.borrow_mut() = paths.clone());
    paths
}

// ---- The protocol: JSON messages, each after a `Content-Length: n` line and an empty line ----

static SEQ: AtomicI64 = AtomicI64::new(1);

fn send(mut msg: Value) {
    msg["seq"] = json!(SEQ.fetch_add(1, Ordering::Relaxed));
    let text = msg.to_string();
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{text}", text.len());
    let _ = out.flush();
}

fn respond(req: &Value, body: Value) {
    send(json!({ "type": "response", "request_seq": req["seq"], "command": req["command"], "success": true, "body": body }));
}

fn fail(req: &Value, message: &str) {
    send(json!({ "type": "response", "request_seq": req["seq"], "command": req["command"], "success": false, "message": message }));
}

fn event(name: &str, body: Value) {
    send(json!({ "type": "event", "event": name, "body": body }));
}

/// Text in the editor's debug console: `stdout` (what the program prints) or `stderr`.
pub fn output(category: &str, text: &str) {
    event("output", json!({ "category": category, "output": text }));
}

fn receive(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() && length.is_some() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length:") {
            length = n.trim().parse().ok();
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// What the program prints, sent to the editor's debug console line by line.
#[derive(Default)]
pub struct Console {
    pending: Vec<u8>,
}

impl Write for Console {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(buf);
        if buf.contains(&b'\n') {
            self.flush()?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.pending.is_empty() {
            output("stdout", &String::from_utf8_lossy(&std::mem::take(&mut self.pending)));
        }
        Ok(())
    }
}
