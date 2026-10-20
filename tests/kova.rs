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
        let run = Command::new(KOVA).arg("run").arg(&f).output().unwrap();
        check_output(&name, "run", &run, &expected, &mut failures);

        // Build mode: compile, then run the produced executable.
        let exe = tmp.join(name.trim_end_matches(".kova"));
        let out = Command::new(KOVA).arg("build").arg(&f).arg("-o").arg(&exe).output().unwrap();
        if !out.status.success() {
            failures.push(format!("{name} [build]: compilation failed\n  {}", text(&out.stderr).trim()));
            continue;
        }
        let out = Command::new(&exe).output().unwrap();
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

        let out = Command::new(KOVA).arg("run").arg(&f).output().unwrap();
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
    let out = Command::new(KOVA).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(text(&out.stdout).trim(), format!("kova {}", env!("CARGO_PKG_VERSION")));

    let out = Command::new(KOVA).arg("whatever").output().unwrap();
    assert_eq!(out.status.code(), Some(2), "unknown command → usage + code 2");

    let out = Command::new(KOVA).args(["run", "file-that-does-not-exist.kova"]).output().unwrap();
    assert!(text(&out.stderr).contains("impossible de lire"));

    // `--boost` removes contracts: the failed `require` gives way to the division by zero.
    let exe = std::env::temp_dir().join(format!("kova-boost-{}", std::process::id()));
    let out = Command::new(KOVA).args(["build", "tests/ok/contract_require.kova", "--boost", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = Command::new(&exe).output().unwrap();
    let _ = std::fs::remove_file(&exe);
    assert_eq!(out.status.code(), Some(101));
    assert!(text(&out.stderr).starts_with("erreur: division par zéro"), "{}", text(&out.stderr));

    // `kova test`: every test runs, even after a failure; code 1 if any failed.
    let out = Command::new(KOVA).args(["test", "tests/ok/test_blocks.kova"]).output().unwrap();
    let stdout = text(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    for piece in [
        "✅ carré de 3\n❌ carré d'un négatif\n   erreur: un carré est toujours positif\n",
        "     check carre(n) = -16\n     valeurs : n = -4\n❌ division\n   erreur: division par zéro\n",
        "\n1 réussi(s), 2 raté(s) sur 3\n",
    ] {
        assert!(stdout.contains(piece), "« {piece} » missing from:\n{stdout}");
    }

    // `header`: `mode := build` → `kova f.kova` compiles; `lock := true` refuses the other command.
    let dir = std::env::temp_dir().join(format!("kova-header-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("prog.kova");
    std::fs::write(&source, "header {\n    mode := build\n    lock := true\n}\nfn main() {\n    print(7)\n}\n").unwrap();
    let out = Command::new(KOVA).arg("run").arg(&source).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stderr), "erreur: ce programme est réservé à `kova build` (header ligne 3)\n");
    let out = Command::new(KOVA).arg(&source).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&Command::new(dir.join("prog")).output().unwrap().stdout), "7\n");
    std::fs::write(&source, "header {\n    lock := true\n}\nfn main() {\n    print(8)\n}\n").unwrap();
    let out = Command::new(KOVA).arg("build").arg(&source).output().unwrap();
    assert_eq!(text(&out.stderr), "erreur: ce programme est réservé à `kova run` (header ligne 2)\n");
    assert_eq!(text(&Command::new(KOVA).arg(&source).output().unwrap().stdout), "8\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(expectations("// expect: 3\nx\n  // expect:  a b\n// error: z", "expect"), ["3", " a b"]);
}

/// Talks to `kova lsp` like the editor does: opens a wrong file, receives the error,
/// asks for completion, then shuts the server down cleanly.
#[test]
fn language_server() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let mut server = Command::new(KOVA).arg("lsp").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
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

    let text = "fn main() {\n    let a: Int := 1\n    a := 2\n}\n";
    send(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": uri, "languageId": "kova", "version": 1, "text": text}}}));
    let diag = receive();
    assert_eq!(diag["method"], "textDocument/publishDiagnostics");
    let error = &diag["params"]["diagnostics"][0];
    assert!(error["message"].as_str().unwrap().contains("`a` est déclarée avec let"), "{diag}");
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
