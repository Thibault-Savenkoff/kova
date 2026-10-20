// Kova test runner (see docs/PLAN.md, Part 3, "Tests").
//
// tests/ok/*.kova    : must work in `run` AND in `build`, with the same output.
//                      Expected output: one `// expect: ...` line per printed line.
//                      Runtime error: `// code: 101` and `// stderr: ...` (a piece of the message).
//                      stderr must be identical in both modes.
// tests/error/*.kova : must be rejected; each `// error: ...` must appear in the message.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const KOVA: &str = env!("CARGO_BIN_EXE_kova");

/// `kova` (or a built executable) with its language fixed: the tests must not depend
/// on the machine's `LANG`.
const LANG: &str = "en";

fn kova(lang: &str) -> Command {
    program(KOVA, lang)
}
fn program(path: impl AsRef<std::ffi::OsStr>, lang: &str) -> Command {
    let mut c = Command::new(path);
    c.env("KOVA_LANG", lang).env_remove("LC_ALL").env_remove("LC_MESSAGES").env_remove("LANG");
    c
}

fn files(dir: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let mut v: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "kova"))
        .collect();
    v.sort();
    v
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
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&f).unwrap();
        let expected = Expected {
            output: expectations(&source, "expect"),
            code: expectations(&source, "code").first().map_or(0, |c| c.parse().unwrap()),
            stderr: expectations(&source, "stderr"),
        };

        // Run mode
        let run = kova(LANG).arg("run").arg(&f).output().unwrap();
        check_output(&name, "run", &run, &expected, &mut failures);

        // Build mode: compile, then run the produced executable.
        let exe = tmp.join(name.trim_end_matches(".kova"));
        let out = kova(LANG).arg("build").arg(&f).arg("-o").arg(&exe).output().unwrap();
        if !out.status.success() {
            failures.push(format!("{name} [build]: compilation failed\n  {}", text(&out.stderr).trim()));
            continue;
        }
        let out = program(&exe, LANG).output().unwrap();
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
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
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
    let exe = std::env::temp_dir().join(format!("kova-boost-{}", std::process::id()));
    let out = kova(LANG).args(["build", "tests/ok/contract_require.kova", "--boost", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = program(&exe, LANG).output().unwrap();
    let _ = std::fs::remove_file(&exe);
    assert_eq!(out.status.code(), Some(101));
    assert!(text(&out.stderr).starts_with("error: division by zero"), "{}", text(&out.stderr));

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
    let _ = std::fs::remove_dir_all(&dir);

    languages();
    modes();

    assert_eq!(expectations("// expect: 3\nx\n  // expect:  a b\n// error: z", "expect"), ["3", " a b"]);
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

    send(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown"}));
    assert_eq!(receive()["id"], 3);
    send(serde_json::json!({"jsonrpc": "2.0", "method": "exit"}));
    assert!(server.wait().unwrap().success());
}
