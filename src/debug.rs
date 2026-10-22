// Debugger (`kova debug`): the editor talks to it with the DAP protocol (Debug Adapter Protocol,
// the debugging twin of LSP), on stdin/stdout. It runs the program in the interpreter, which
// asks it at each statement whether to stop (`interp::line`).
//
//   editor ──"stop at line 12"──► kova debug ──runs──► interp ── line 12 ──► stopped
//          ◄──"stopped" + variables─────────────────────────────┘
//
// The KIR is translated with `watching` on: an `Inst::Line` before each statement says where it
// is and which variables are visible; `kova run` and `kova build` never see them.

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

/// Where to stop next, besides the breakpoints.
#[derive(Clone, Copy, PartialEq)]
enum Step {
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
    let mut d = Debugger { requests, breakpoints: HashMap::new(), step: Step::Run, frames: vec![], errored: false, asked: None, quiet: false };

    // Until the editor has said what to run and where to stop.
    let (mut launch, mut ready) = (None, false);
    while launch.is_none() || !ready {
        let req = d.next();
        match req["command"].as_str().unwrap_or("") {
            "initialize" => {
                respond(&req, json!({ "supportsConfigurationDoneRequest": true, "supportsTerminateRequest": true }));
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

    /// While stopped: answers the editor until it asks for a function's variables (its frame
    /// number, `Some`) or to go on (`None`).
    pub fn variables_asked(&mut self) -> Option<usize> {
        loop {
            let req = self.next();
            let depth = self.frames.len();
            let step = match req["command"].as_str().unwrap_or("") {
                "variables" => {
                    let frame = req["arguments"]["variablesReference"].as_u64().unwrap_or(1) as usize - 1;
                    self.asked = Some(req);
                    return Some(frame);
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

    /// The answer to `variables_asked`: (name, type, value as text) of each variable.
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
