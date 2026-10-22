// Kova test runner (see docs/PLAN.md, Part 3, "Tests").
//
// tests/ok/*.kova    : must work in `run` AND in `build`, with the same output.
//                      Expected output: one `// expect: ...` line per printed line.
//                      Runtime error: `// code: 101` and `// stderr: ...` (a piece of the message).
//                      stderr must be identical in both modes.
//                      Input: one `// stdin: ...` line per line typed (none = empty input).
// tests/error/*.kova : must be rejected; each `// error: ...` must appear in the message.

use std::path::{Path, PathBuf};
use std::io::Write;
use std::process::{Command, Output, Stdio};

const KOVA: &str = env!("CARGO_BIN_EXE_kova");

/// `kova` (or a built executable) with its language fixed: the tests must not depend
/// on the machine's `LANG`.
const LANG: &str = "en";
/// `.exe` on Windows: what `kova build -o name` adds.
const EXE: &str = std::env::consts::EXE_SUFFIX;

fn kova(lang: &str) -> Command {
    program(KOVA, lang)
}
fn program(path: impl AsRef<std::ffi::OsStr>, lang: &str) -> Command {
    let mut c = Command::new(path);
    // KOVA_LEAKS: a String never freed makes the program fail (code 102), in run and in build.
    c.env("KOVA_LANG", lang).env("KOVA_LEAKS", "1").env_remove("LC_ALL").env_remove("LC_MESSAGES").env_remove("LANG");
    c
}

fn files(dir: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    // A program of several files: a folder whose `main.kova` imports the others.
    let mut v: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .map(|p| if p.is_dir() { p.join("main.kova") } else { p })
        .filter(|p| p.extension().is_some_and(|e| e == "kova") && p.exists())
        .collect();
    v.sort();
    v
}

/// `square.kova`, or `geometry/main.kova` for a folder.
fn test_name(f: &Path) -> String {
    let parent = f.parent().unwrap();
    match parent.file_name().unwrap().to_string_lossy() {
        p if p == "ok" || p == "error" => f.file_name().unwrap().to_string_lossy().into_owned(),
        p => format!("{p}/main.kova"),
    }
}

/// The values of the `// <key>: value` comments, in order.
fn expectations(source: &str, key: &str) -> Vec<String> {
    let prefix = format!("// {key}:");
    source
        .lines()
        .filter_map(|l| l.trim().strip_prefix(&prefix))
        .map(|v| v.strip_prefix(' ').unwrap_or(v).to_string())
        .collect()
}

/// Runs `c` with `input` as its standard input.
fn with_input(c: &mut Command, input: &str) -> Output {
    with_bytes(c, input.as_bytes())
}

/// Runs `c` with these bytes as its standard input.
fn with_bytes(c: &mut Command, input: &[u8]) -> Output {
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // A program may stop before reading all its input.
    let _ = child.stdin.take().unwrap().write_all(input);
    child.wait_with_output().unwrap()
}

fn text(o: &[u8]) -> String {
    String::from_utf8_lossy(o).into_owned()
}

struct Expected {
    output: Vec<String>,
    code: i32,
    stderr: Vec<String>,
}

fn check_output(name: &str, mode: &str, out: &Output, e: &Expected, failures: &mut Vec<String>) {
    let got: Vec<String> = text(&out.stdout).lines().map(String::from).collect();
    let stderr = text(&out.stderr);
    if out.status.code() != Some(e.code) || got != e.output || !e.stderr.iter().all(|m| stderr.contains(m.as_str())) {
        failures.push(format!(
            "{name} [{mode}]: code {:?} (expected {})\n  expected: {:?}\n  got:      {got:?}\n  stderr:   {}",
            out.status.code(),
            e.code,
            e.output,
            stderr.trim()
        ));
    }
}

#[test]
fn ok_programs() {
    let tmp = std::env::temp_dir().join(format!("kova-tests-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut failures = vec![];

    for f in files("ok") {
        let name = test_name(&f);
        let source = std::fs::read_to_string(&f).unwrap();
        let expected = Expected {
            output: expectations(&source, "expect"),
            code: expectations(&source, "code").first().map_or(0, |c| c.parse().unwrap()),
            stderr: expectations(&source, "stderr"),
        };
        let input: String = expectations(&source, "stdin").iter().map(|l| format!("{l}\n")).collect();
        // `// args: a b`: the words given to the program (`args`).
        let args: Vec<String> = expectations(&source, "args").iter().flat_map(|l| l.split_whitespace().map(String::from)).collect();

        // Run mode
        let run = with_input(kova(LANG).arg("run").arg(&f).args(&args), &input);
        check_output(&name, "run", &run, &expected, &mut failures);

        // Build mode: compile, then run the produced executable.
        let exe = tmp.join(name.trim_end_matches(".kova").replace('/', "_") + EXE);
        let out = kova(LANG).arg("build").arg(&f).arg("-o").arg(&exe).output().unwrap();
        if !out.status.success() {
            failures.push(format!("{name} [build]: compilation failed\n  {}", text(&out.stderr).trim()));
            continue;
        }
        let out = with_input(program(&exe, LANG).args(&args), &input);
        check_output(&name, "build", &out, &expected, &mut failures);
        if out.stderr != run.stderr {
            failures.push(format!(
                "{name}: different stderr\n  run:   {}\n  build: {}",
                text(&run.stderr).trim(),
                text(&out.stderr).trim()
            ));
        }
    }

    let _ = std::fs::remove_dir_all(&tmp);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

#[test]
fn rejected_programs() {
    let mut failures = vec![];
    for f in files("error") {
        let name = test_name(&f);
        let expected = expectations(&std::fs::read_to_string(&f).unwrap(), "error");
        assert!(!expected.is_empty(), "{name}: missing `// error: ...`");

        let out = kova(LANG).arg("run").arg(&f).output().unwrap();
        let stderr = text(&out.stderr);
        if out.status.success() {
            failures.push(format!("{name}: accepted but should have been rejected"));
        } else if let Some(missing) = expected.iter().find(|e| !stderr.contains(e.as_str())) {
            failures.push(format!("{name}: expected message « {missing} »\n  got: {}", stderr.trim()));
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

#[test]
fn command_line() {
    let out = kova(LANG).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(text(&out.stdout).trim(), format!("kova {}", env!("CARGO_PKG_VERSION")));

    let out = kova(LANG).arg("whatever").output().unwrap();
    assert_eq!(out.status.code(), Some(2), "unknown command → usage + code 2");

    let out = kova(LANG).args(["run", "file-that-does-not-exist.kova"]).output().unwrap();
    assert!(text(&out.stderr).contains("cannot read"));

    // `--boost` removes contracts: the failed `require` gives way to the division by zero.
    let exe = std::env::temp_dir().join(format!("kova-boost-{}{EXE}", std::process::id()));
    let out = kova(LANG).args(["build", "tests/ok/contract_require.kova", "--boost", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = program(&exe, LANG).output().unwrap();
    let _ = std::fs::remove_file(&exe);
    assert_eq!(out.status.code(), Some(101));
    assert!(text(&out.stderr).starts_with("error: division by zero"), "{}", text(&out.stderr));

    // A module alone: no `main` needed for `kova test`; its own tests run (not when imported).
    let out = kova(LANG).args(["test", "tests/ok/modules/shapes.kova"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let out = kova(LANG).args(["test", "tests/ok/modules/geometry.kova"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("❌ run only by kova test on this file"), "{}", text(&out.stdout));

    // `kova test`: every test runs, even after a failure; code 1 if any failed.
    let out = kova(LANG).args(["test", "tests/ok/test_blocks.kova"]).output().unwrap();
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    for piece in [
        "✅ carré de 3\n❌ carré d'un négatif\n   error: un carré est toujours positif\n",
        "     check carre(n) = -16\n     values: n = -4\n❌ division\n   error: division by zero\n",
        "\n1 passed, 2 failed out of 3\n",
    ] {
        assert!(stdout.contains(piece), "« {piece} » missing from:\n{stdout}");
    }

    // `kova test f.kova "name"`: only that test (the editor's ▶ above a `test`).
    let out = kova(LANG).args(["test", "tests/ok/test_blocks.kova", "carré de 3"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "✅ carré de 3\n\n1 passed, 0 failed out of 1\n");
    let out = kova(LANG).args(["test", "tests/ok/test_blocks.kova", "nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stderr), "no test \"nope\" in tests/ok/test_blocks.kova\n");

    // `header`: `mode := build` → `kova f.kova` compiles; `lock := true` refuses the other command.
    let dir = std::env::temp_dir().join(format!("kova-header-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("prog.kova");
    std::fs::write(&source, "header {\n    mode := build\n    lock := true\n}\nfn main {\n    print(7)\n}\n").unwrap();
    let out = kova(LANG).arg("run").arg(&source).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stderr), "error: this program is reserved for `kova build` (header line 3)\n");
    let out = kova(LANG).arg(&source).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&program(dir.join("prog"), LANG).output().unwrap().stdout), "7\n");
    std::fs::write(&source, "header {\n    lock := true\n}\nfn main {\n    print(8)\n}\n").unwrap();
    let out = kova(LANG).arg("build").arg(&source).output().unwrap();
    assert_eq!(text(&out.stderr), "error: this program is reserved for `kova run` (header line 2)\n");
    assert_eq!(text(&kova(LANG).arg(&source).output().unwrap().stdout), "8\n");

    // `kova fmt`: lays the file out in place; a file that doesn't read stays as it is (code 1).
    std::fs::write(&source, "fn main {\nprint( 8 )\n}").unwrap();
    assert!(kova(LANG).arg("fmt").arg(&source).status().unwrap().success());
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "fn main {\n    print(8)\n}\n");
    std::fs::write(&source, "fn main {\nprint( \"8 )\n}").unwrap();
    let out = kova(LANG).arg("fmt").arg(&source).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("is not laid out"), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "fn main {\nprint( \"8 )\n}");
    let _ = std::fs::remove_dir_all(&dir);

    languages();
    modes();
    leak_counter();
    seeded();
    static_executable();

    assert_eq!(expectations("// expect: 3\nx\n  // expect:  a b\n// error: z", "expect"), ["3", " a b"]);
}

/// `kova build` makes a 100% static executable when musl is installed: no ELF program header
/// `PT_INTERP` (3), the loader of the system's C library.
fn static_executable() {
    if Command::new("musl-gcc").arg("--version").output().is_err() {
        return;
    }
    let exe = std::env::temp_dir().join(format!("kova-static-{}{EXE}", std::process::id()));
    let out = kova(LANG).args(["build", "tests/ok/strings.kova", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    let elf = std::fs::read(&exe).unwrap();
    let _ = std::fs::remove_file(&exe);
    let number = |at: usize, n: usize| elf[at..at + n].iter().rev().fold(0usize, |v, &b| v << 8 | b as usize);
    let (start, size, count) = (number(0x20, 8), number(0x36, 2), number(0x38, 2));
    assert!((0..count).all(|i| number(start + i * size, 4) != 3), "the executable needs the system's loader");
}

/// The leak check really counts: the same number of Strings is made in run and in build.
fn leak_counter() {
    let count = |c: &mut Command| {
        let out = c.env("KOVA_LEAKS", "count").output().unwrap();
        assert!(out.status.success(), "{}", text(&out.stderr));
        text(&out.stderr)
    };
    let run = count(kova(LANG).args(["run", "tests/ok/strings.kova"]));
    assert_eq!(run, "Strings and Lists made: 25\n");
    let exe = std::env::temp_dir().join(format!("kova-leaks-{}{EXE}", std::process::id()));
    assert!(kova(LANG).args(["build", "tests/ok/strings.kova", "-o"]).arg(&exe).status().unwrap().success());
    assert_eq!(count(&mut program(&exe, LANG)), run);
    let _ = std::fs::remove_file(&exe);
}

/// `KOVA_SEED`: the same random numbers again, and the same in run and build.
fn seeded() {
    let seeded = |c: &mut Command| text(&c.env("KOVA_SEED", "42").output().unwrap().stdout);
    let run = seeded(kova(LANG).args(["run", "tests/seed/seed.kova"]));
    assert_eq!(run, "14 92 59 65 51 6792609088808213255\n");
    let exe = std::env::temp_dir().join(format!("kova-seed-{}", std::process::id()));
    assert!(kova(LANG).args(["build", "tests/seed/seed.kova", "-o"]).arg(&exe).status().unwrap().success());
    assert_eq!(seeded(&mut program(&exe, LANG)), run);
    let _ = std::fs::remove_file(&exe);
}

/// `if mode = build`: each mode translates only its own branch.
fn modes() {
    let out = kova(LANG).args(["run", "tests/modes/mode.kova"]).output().unwrap();
    assert_eq!(text(&out.stdout), "11\n1\n3\n", "{}", text(&out.stderr));
    let exe = std::env::temp_dir().join(format!("kova-mode-{}", std::process::id()));
    let out = kova(LANG).args(["build", "tests/modes/mode.kova", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&program(&exe, LANG).output().unwrap().stdout), "20\n2\n3\n");
    let _ = std::fs::remove_file(&exe);
}

/// The language of the messages (PLAN 2.13): environment, `header`, translation files.
fn languages() {
    let dir = std::env::temp_dir().join(format!("kova-lang-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let stderr = |c: &mut Command| text(&c.output().unwrap().stderr);

    // A compile error: the first non-empty variable decides, even if unknown (then English).
    let source = dir.join("bad.kova");
    std::fs::write(&source, "fn main {\n    let a: Int := 1\n    a := 2\n}\n").unwrap();
    assert!(stderr(kova("fr").arg("run").arg(&source)).starts_with("erreur: `a` est déclarée avec let"));
    assert!(stderr(kova("").env("LANG", "fr_FR.UTF-8").arg("run").arg(&source)).starts_with("erreur: "));
    assert!(stderr(kova("C").env("LANG", "fr_FR.UTF-8").arg("run").arg(&source)).starts_with("error: "));
    assert!(stderr(kova("de_DE.UTF-8").arg("run").arg(&source)).starts_with("error: "));

    // One executable, every language; `header { lang := fr }` wins over the environment.
    let source = dir.join("fault.kova");
    let program_text = "fn main {\n    let z: Int := 0\n    print(1 / z)\n}\n";
    std::fs::write(&source, program_text).unwrap();
    let exe = dir.join("fault");
    assert!(kova("en").arg("build").arg(&source).arg("-o").arg(&exe).status().unwrap().success());
    assert!(stderr(&mut program(&exe, "en")).starts_with("error: division by zero"));
    assert!(stderr(&mut program(&exe, "fr")).starts_with("erreur: division par zéro"));
    std::fs::write(&source, format!("header {{\n    lang := fr\n}}\n{program_text}")).unwrap();
    assert!(stderr(kova("en").arg("run").arg(&source)).starts_with("erreur: division par zéro"));
    assert!(kova("en").arg("build").arg(&source).arg("-o").arg(&exe).status().unwrap().success());
    assert!(stderr(&mut program(&exe, "en")).starts_with("erreur: division par zéro"));

    // `kova run` on a translation file checks it against the others of its folder.
    let out = kova("en").args(["run", "locales/fr.kova"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
    assert!(text(&out.stdout).contains("complete compared with en.kova"));
    std::fs::write(dir.join("en.kova"), "lang en {\n    hello := \"Hello {name}\"\n    bye := \"Bye\"\n}\n").unwrap();
    std::fs::write(dir.join("fr.kova"), "lang fr {\n    hello := \"Bonjour {nom}\"\n}\n").unwrap();
    let out = kova("en").arg("run").arg(dir.join("fr.kova")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("1 key(s)\n  missing compared with en.kova: bye\n"), "{stdout}");
    assert!(stdout.contains("different `{…}` from en.kova: hello"), "{stdout}");
    assert_eq!(kova("en").arg("build").arg(dir.join("fr.kova")).output().unwrap().status.code(), Some(1));

    let _ = std::fs::remove_dir_all(&dir);
}

/// Talks to `kova lsp` like the editor does: opens a wrong file, receives the error,
/// asks for completion, then shuts the server down cleanly.
#[test]
fn language_server() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let mut server = kova(LANG).arg("lsp").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut input = server.stdin.take().unwrap();
    let mut output = BufReader::new(server.stdout.take().unwrap());
    let mut send = |msg: serde_json::Value| {
        let text = msg.to_string();
        write!(input, "Content-Length: {}\r\n\r\n{text}", text.len()).unwrap();
        input.flush().unwrap();
    };
    let mut receive = || -> serde_json::Value {
        let mut length = 0;
        loop {
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                length = n.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        output.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    };
    let uri = "file:///tmp/sample.kova";

    send(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}));
    assert!(receive()["result"]["capabilities"]["completionProvider"].is_object());
    send(serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));

    let text = "fn main {\n    let a: Int := 1\n    a := 2\n}\n";
    send(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": uri, "languageId": "kova", "version": 1, "text": text}}}));
    let diag = receive();
    assert_eq!(diag["method"], "textDocument/publishDiagnostics");
    let error = &diag["params"]["diagnostics"][0];
    assert!(error["message"].as_str().unwrap().contains("`a` is declared with let"), "{diag}");
    assert_eq!(error["range"]["start"], serde_json::json!({"line": 2, "character": 4}));

    send(serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/completion",
        "params": {"textDocument": {"uri": uri}, "position": {"line": 2, "character": 4}}}));
    let response = receive();
    let labels: Vec<&str> = response["result"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"a") && labels.contains(&"main") && labels.contains(&"print"), "{labels:?}");

    // An error in an imported file: shown on the `import` line, with its real place.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/error/import_error");
    let main = format!("file://{}", dir.join("main.kova").display());
    let text = std::fs::read_to_string(dir.join("main.kova")).unwrap();
    send(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": main, "languageId": "kova", "version": 1, "text": text}}}));
    let diag = receive();
    let error = &diag["params"]["diagnostics"][0];
    assert!(error["message"].as_str().unwrap().contains("broken.kova:4:16: different types"), "{diag}");
    assert_eq!(error["range"]["start"], serde_json::json!({"line": 0, "character": 7}));

    // Imported through another file: shown on the `import` of the file that leads to it.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/error/import_deep");
    let main = format!("file://{}", dir.join("main.kova").display());
    let text = std::fs::read_to_string(dir.join("main.kova")).unwrap();
    send(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": main, "languageId": "kova", "version": 1, "text": text}}}));
    let diag = receive();
    let error = &diag["params"]["diagnostics"][0];
    assert!(error["message"].as_str().unwrap().contains("deep.kova:2:"), "{diag}");
    assert_eq!(error["range"]["start"], serde_json::json!({"line": 1, "character": 7}));

    // E4: the type on hover, Ctrl+click, the types in grey.
    let text = "fn main {\n    let l: List<Int> := [1]\n    for x in l {\n        print(x)\n    }\n}\n";
    send(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": text}]}}));
    receive();
    let at = serde_json::json!({"textDocument": {"uri": uri}, "position": {"line": 3, "character": 14}});
    send(serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "textDocument/hover", "params": at}));
    assert_eq!(receive()["result"]["contents"]["value"], "```kova\nx: Int\n```");
    send(serde_json::json!({"jsonrpc": "2.0", "id": 5, "method": "textDocument/definition", "params": at}));
    assert_eq!(receive()["result"]["range"]["start"], serde_json::json!({"line": 2, "character": 8}));
    let range = serde_json::json!({"start": {"line": 0, "character": 0}, "end": {"line": 6, "character": 0}});
    send(serde_json::json!({"jsonrpc": "2.0", "id": 6, "method": "textDocument/inlayHint",
        "params": {"textDocument": {"uri": uri}, "range": range}}));
    assert_eq!(receive()["result"][0]["label"], ": Int");

    send(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown"}));
    assert_eq!(receive()["id"], 3);
    send(serde_json::json!({"jsonrpc": "2.0", "method": "exit"}));
    assert!(server.wait().unwrap().success());
}

/// `kova debug`: the editor's side of the DAP protocol, on `tests/debug/dogs.kova`.
#[test]
fn debugger() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{ChildStdin, ChildStdout, Stdio};
    use serde_json::{Value, json};

    struct Editor {
        input: ChildStdin,
        output: BufReader<ChildStdout>,
        seq: u64,
        /// What the program printed.
        printed: String,
    }
    impl Editor {
        fn send(&mut self, command: &str, arguments: Value) {
            self.seq += 1;
            let text = json!({"seq": self.seq, "type": "request", "command": command, "arguments": arguments}).to_string();
            write!(self.input, "Content-Length: {}\r\n\r\n{text}", text.len()).unwrap();
            self.input.flush().unwrap();
        }
        /// Reads until a message `wanted` accepts.
        fn until(&mut self, wanted: impl Fn(&Value) -> bool) -> Value {
            loop {
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    self.output.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.strip_prefix("Content-Length: ") {
                        length = n.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                self.output.read_exact(&mut body).unwrap();
                let msg: Value = serde_json::from_slice(&body).unwrap();
                if msg["event"] == "output" {
                    self.printed += msg["body"]["output"].as_str().unwrap();
                }
                if wanted(&msg) {
                    return msg;
                }
            }
        }
        fn ask(&mut self, command: &'static str, arguments: Value) -> Value {
            self.send(command, arguments);
            self.until(|m| m["type"] == "response" && m["command"] == command)
        }
        /// Goes on (`continue`, `next`…) until the next stop: its reason.
        fn go(&mut self, command: &'static str) -> Value {
            self.send(command, json!({"threadId": 1}));
            self.until(|m| m["event"] == "stopped")["body"]["reason"].clone()
        }
        /// The variables of a function stopped in (frame 0 = `main`, 1 = the function it called).
        fn variables(&mut self, frame: u64) -> Vec<String> {
            let r = self.ask("variables", json!({"variablesReference": frame + 1}));
            let shown = |v: &Value| format!("{} = {}", v["name"].as_str().unwrap(), v["value"].as_str().unwrap());
            r["body"]["variables"].as_array().unwrap().iter().map(shown).collect()
        }
        fn stack(&mut self) -> Vec<(String, u64)> {
            let r = self.ask("stackTrace", json!({"threadId": 1}));
            let frame = |f: &Value| (f["name"].as_str().unwrap().to_string(), f["line"].as_u64().unwrap());
            r["body"]["stackFrames"].as_array().unwrap().iter().map(frame).collect()
        }
    }

    let mut adapter = kova(LANG).arg("debug").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut e = Editor { input: adapter.stdin.take().unwrap(), output: BufReader::new(adapter.stdout.take().unwrap()), seq: 0, printed: String::new() };
    let path = std::fs::canonicalize("tests/debug/dogs.kova").unwrap().display().to_string();

    e.send("initialize", json!({"adapterID": "kova"}));
    e.until(|m| m["event"] == "initialized");
    e.ask("launch", json!({"program": path}));
    let set = e.ask("setBreakpoints", json!({"source": {"path": path}, "breakpoints": [{"line": 15}, {"line": 18}]}));
    assert_eq!(set["body"]["breakpoints"][0]["verified"], true);
    e.send("configurationDone", json!({}));

    assert_eq!(e.until(|m| m["event"] == "stopped")["body"]["reason"], "breakpoint");
    assert_eq!(e.variables(0), ["rex = Dog { name: \"Rex\", age: 3 }", "names = [\"a\", \"b\"]"]);
    assert_eq!(e.go("stepIn"), "step");
    assert_eq!(e.stack(), [("older".to_string(), 8), ("main".to_string(), 15)]);
    e.go("next");
    assert_eq!(e.variables(1), ["d = Dog { name: \"Rex\", age: 3 }", "years = 4"]);
    e.go("stepOut");
    assert_eq!(e.stack(), [("main".to_string(), 16)]);
    assert_eq!(e.go("continue"), "breakpoint");
    assert_eq!(e.variables(0), ["rex = (given away)", "names = [\"a\", \"b\"]", "next = 4", "gone = Dog { name: \"Rex\", age: 3 }"]);
    e.send("continue", json!({"threadId": 1}));
    assert_eq!(e.until(|m| m["event"] == "exited")["body"]["exitCode"], 0);
    assert_eq!(e.printed, "4\n3 2\n");
    e.send("disconnect", json!({}));
    assert!(adapter.wait().unwrap().success());

    // A runtime error: stops where it happens, with its message, then the program ends (101).
    let mut adapter = kova(LANG).arg("debug").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut e = Editor { input: adapter.stdin.take().unwrap(), output: BufReader::new(adapter.stdout.take().unwrap()), seq: 0, printed: String::new() };
    e.send("initialize", json!({"adapterID": "kova"}));
    e.ask("launch", json!({"program": "tests/debug/fault.kova"}));
    e.send("configurationDone", json!({}));
    let stop = e.until(|m| m["event"] == "stopped");
    assert_eq!(stop["body"]["reason"], "exception");
    assert!(stop["body"]["text"].as_str().unwrap().starts_with("error: division by zero"), "{stop}");
    assert_eq!(e.stack()[0], ("divide".to_string(), 3));
    assert_eq!(e.variables(1), ["a = 6", "b = 0"]);
    e.send("continue", json!({"threadId": 1}));
    assert_eq!(e.until(|m| m["event"] == "exited")["body"]["exitCode"], 101);
    assert!(e.printed.contains("division by zero"), "{}", e.printed);
    e.send("disconnect", json!({}));
    assert!(adapter.wait().unwrap().success());
}

/// A line that is not UTF-8 stops `read_line` loudly (not a silent `none`), in run and in build.
#[test]
fn input_not_text() {
    let source = "tests/ok/read_line.kova";
    let input = b"12\n\xff\n30\n";
    let run = with_bytes(kova(LANG).arg("run").arg(source), input);
    let exe = std::env::temp_dir().join(format!("kova-not-text-{}", std::process::id()));
    assert!(kova(LANG).arg("build").arg(source).arg("-o").arg(&exe).status().unwrap().success());
    let build = with_bytes(&mut program(&exe, LANG), input);
    let _ = std::fs::remove_file(&exe);
    for out in [&run, &build] {
        assert_eq!(out.status.code(), Some(101), "{}", text(&out.stderr));
        assert!(text(&out.stderr).contains("the line typed is not text (invalid UTF-8)"), "{}", text(&out.stderr));
    }
    assert_eq!(run.stderr, build.stderr);
}
