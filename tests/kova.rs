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
    let failures = in_parallel(&files("ok"), |f, failures| check_program(f, &tmp, failures));
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

/// The examples of the README and of docs/guide.md: a ```kova block followed by a ```text block
/// prints exactly that. A block starting with `// geometry.kova` is a file of the program before it.
#[test]
fn guide_examples() {
    let tmp = std::env::temp_dir().join(format!("kova-guide-{}", std::process::id()));
    let mut failures = vec![];
    for doc in ["README.md", "docs/guide.md"] {
        let page = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(doc)).unwrap();
        // (language, text) of each fenced block.
        let mut blocks: Vec<(String, String)> = vec![];
        let mut open: Option<(String, String)> = None;
        for line in page.lines() {
            match (line.strip_prefix("```"), open.take()) {
                (Some(lang), None) => open = Some((lang.to_string(), String::new())),
                (Some(_), Some(block)) => blocks.push(block),
                (None, Some((lang, body))) => open = Some((lang, body + line + "\n")),
                (None, None) => {}
            }
        }
        let mut files: Vec<(String, String)> = vec![];
        for (lang, body) in blocks {
            let named = body.lines().next().and_then(|l| l.strip_prefix("// ")).and_then(|l| l.split_whitespace().next()).map(|n| n.trim_end_matches(',')).filter(|n| n.ends_with(".kova"));
            match (lang.as_str(), named) {
                ("kova", Some(name)) => files.push((name.to_string(), body)),
                ("kova", None) => files = vec![("main.kova".to_string(), body)],
                ("text", _) if !files.is_empty() => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    std::fs::create_dir_all(&tmp).unwrap();
                    for (name, code) in &files {
                        std::fs::write(tmp.join(name), code).unwrap();
                    }
                    let out = kova(LANG).arg("run").arg(&files[0].0).current_dir(&tmp).output().unwrap();
                    if text(&out.stdout) != body || !out.status.success() {
                        failures.push(format!("{doc}, {}:\n{}{}", files[0].1.lines().next().unwrap_or(""), text(&out.stdout), text(&out.stderr)));
                    }
                    files.clear();
                }
                _ => {}
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

/// `check` on every file, one per core at a time (most of the time goes to starting programs,
/// slow on Windows, and to the C compiler of `kova build`); the failures in the files' order.
fn in_parallel(files: &[PathBuf], check: impl Fn(&PathBuf, &mut Vec<String>) + Sync) -> Vec<String> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let found = std::sync::Mutex::new(vec![]);
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..cores {
            s.spawn(|| {
                while let Some(f) = files.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed)) {
                    let mut failures = vec![];
                    check(f, &mut failures);
                    found.lock().unwrap().extend(failures.into_iter().map(|m| (f.clone(), m)));
                }
            });
        }
    });
    let mut failures = found.into_inner().unwrap();
    failures.sort();
    failures.into_iter().map(|(_, m)| m).collect()
}

/// `f` under `kova run`, then built and run: the same output, as its `// expect:` lines say.
fn check_program(f: &PathBuf, tmp: &Path, failures: &mut Vec<String>) {
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
    check_output(&name, "run", &run, &expected, failures);

    // Build mode: compile, then run the produced executable.
    let exe = tmp.join(name.trim_end_matches(".kova").replace('/', "_") + EXE);
    let out = kova(LANG).arg("build").arg(&f).arg("-o").arg(&exe).output().unwrap();
    if !out.status.success() {
        failures.push(format!("{name} [build]: compilation failed\n  {}", text(&out.stderr).trim()));
        return;
    }
    let out = with_input(program(&exe, LANG).args(&args), &input);
    check_output(&name, "build", &out, &expected, failures);
    if out.stderr != run.stderr {
        failures.push(format!(
            "{name}: different stderr\n  run:   {}\n  build: {}",
            text(&run.stderr).trim(),
            text(&out.stderr).trim()
        ));
    }
}

#[test]
fn rejected_programs() {
    let failures = in_parallel(&files("error"), |f, failures| {
        let name = test_name(f);
        let expected = expectations(&std::fs::read_to_string(f).unwrap(), "error");
        assert!(!expected.is_empty(), "{name}: missing `// error: ...`");

        let out = kova(LANG).arg("run").arg(f).output().unwrap();
        let stderr = text(&out.stderr);
        if out.status.success() {
            failures.push(format!("{name}: accepted but should have been rejected"));
        } else if let Some(missing) = expected.iter().find(|e| !stderr.contains(e.as_str())) {
            failures.push(format!("{name}: expected message « {missing} »\n  got: {}", stderr.trim()));
        }
    });
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
    cross_build();

    assert_eq!(expectations("// expect: 3\nx\n  // expect:  a b\n// error: z", "expect"), ["3", " a b"]);
}

/// On Linux x86-64, `kova build` makes a 100% static executable (Kova's own linker, src/link.rs):
/// no ELF program header `PT_INTERP` (3), the loader of the system's C library.
fn static_executable() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let exe = std::env::temp_dir().join(format!("kova-static-{}{EXE}", std::process::id()));
    let out = kova(LANG).args(["build", "tests/ok/strings.kova", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success() && text(&out.stderr).starts_with("✓ "), "{}", text(&out.stderr));
    let elf = std::fs::read(&exe).unwrap();
    let _ = std::fs::remove_file(&exe);
    let number = |at: usize, n: usize| elf[at..at + n].iter().rev().fold(0usize, |v, &b| v << 8 | b as usize);
    let (start, size, count) = (number(0x20, 8), number(0x36, 2), number(0x38, 2));
    assert!((0..count).all(|i| number(start + i * size, 4) != 3), "the executable needs the system's loader");
}

/// `kova build --target`: an executable for each system, from any machine (PLAN Part 4
/// "Cross-compilation"), told by the start of its file; `.exe` added for Windows.
fn cross_build() {
    let dir = std::env::temp_dir().join(format!("kova-cross-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("hello.kova");
    std::fs::copy("tests/ok/strings.kova", &source).unwrap();
    // The start of the file, then the processor's number at `at`.
    let systems: [(&str, &str, &[u8], usize, u8); 7] = [
        ("linux", "hello", b"\x7fELF", 0x12, 62),
        ("linux-arm", "hello", b"\x7fELF", 0x12, 183),
        ("windows", "hello.exe", b"MZ", 0x45, 0x86),
        ("windows-arm", "hello.exe", b"MZ", 0x45, 0xAA),
        // Both Macs' executables, the Intel one first.
        ("mac", "hello", b"\xca\xfe\xba\xbe\0\0\0\x02", 11, 0x07),
        ("mac-arm", "hello", b"\xcf\xfa\xed\xfe", 4, 0x0C),
        ("mac-intel", "hello", b"\xcf\xfa\xed\xfe", 4, 0x07),
    ];
    for (target, file, start, at, cpu) in systems {
        let out = kova(LANG).arg("build").arg(&source).args(["--target", target]).output().unwrap();
        // A `kova` built without Rust's library for that system says what to install (build.rs).
        if text(&out.stderr).contains("rustup target add") {
            continue;
        }
        // The report: the file, the system, its size and the time taken.
        let report = text(&out.stderr);
        let title = [("linux", "Linux"), ("linux-arm", "Linux arm"), ("windows", "Windows"), ("windows-arm", "Windows arm"), ("mac", "Mac"), ("mac-arm", "Mac arm"), ("mac-intel", "Mac Intel")].iter().find(|(n, _)| *n == target).unwrap().1;
        assert!(out.status.success() && report.starts_with(&format!("✓ {}", dir.join(file).display())), "{target}: {report}");
        assert!(report.contains(&format!(" ({title}, ")) && report.contains(" KB) in ") && report.lines().count() == 1, "{report}");
        let exe = std::fs::read(dir.join(file)).unwrap();
        assert!(exe.starts_with(start) && exe[at] == cpu, "{target}");
        std::fs::remove_file(dir.join(file)).unwrap();
    }
    std::fs::remove_dir_all(&dir).unwrap();
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

/// `mode build else …` under a signature (PLAN 2.14): what a call does in each mode.
fn modes() {
    let exe = std::env::temp_dir().join(format!("kova-mode-{}", std::process::id()));
    // `mode build else …` under a signature: in the other mode, the `else` acts, at the call.
    let out = kova(LANG).args(["run", "tests/modes/only.kova"]).env("KOVA_LEAKS", "1").output().unwrap();
    assert_eq!(text(&out.stdout), "42\n4\n[1, 2, 3, 4]\nnone\n");
    assert_eq!(out.status.code(), Some(101));
    let stop = text(&out.stderr);
    assert!(stop.starts_with("error: needs kova build, n = 5\n"), "{stop}");
    assert!(stop.contains("only.kova:33:5\n  mode build\n  called from ") && stop.ends_with("only.kova:46:11\n"), "{stop}");
    let out = kova(LANG).args(["build", "tests/modes/only.kova", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = program(&exe, LANG).env("KOVA_LEAKS", "1").output().unwrap();
    assert_eq!(text(&out.stdout), "42\nbuilt: rex\n0\n[1, 2, 3]\n\"build 7\"\n5\nend\n", "{}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0));

    let out = kova(LANG).args(["run", "tests/modes/main_only.kova"]).output().unwrap();
    assert_eq!((text(&out.stdout).as_str(), out.status.code()), ("", Some(101)));
    let stop = text(&out.stderr);
    assert!(stop.starts_with("error: this program needs kova build\n") && stop.ends_with("main_only.kova:3:5\n  mode build\n"), "{stop}");
    let out = kova(LANG).args(["build", "tests/modes/main_only.kova", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&program(&exe, LANG).output().unwrap().stdout), "built\n");

    // Built with `--boost` (no contracts), a stop still says where each kind of call came from.
    let out = kova(LANG).args(["build", "tests/modes/boosted.kova", "--boost", "-o"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    for (call, message, clause, site) in [("any", "sound", "11:9", "33:28"), ("generic", "first", "24:5", "34:28"), ("method", "age", "17:9", "35:24")] {
        let out = program(&exe, LANG).arg(call).output().unwrap();
        let stop = text(&out.stderr);
        assert_eq!(out.status.code(), Some(101), "{stop}");
        assert!(stop.starts_with(&format!("error: no {message} when built\n")), "{stop}");
        assert!(stop.contains(&format!("boosted.kova:{clause}\n  mode run\n  called from ")) && stop.ends_with(&format!("boosted.kova:{site}\n")), "{stop}");
    }
    let out = kova(LANG).args(["run", "tests/modes/boosted.kova", "any"]).output().unwrap();
    assert_eq!(text(&out.stdout), "woof\n");

    // A discreet `mode build` function: `kova run` runs native code, translated for `kova run`;
    // without the `mode build` line, the same program is interpreted, many times slower.
    let timed = |file: &Path| {
        let start = std::time::Instant::now();
        let out = kova(LANG).arg("run").arg(file).output().unwrap();
        assert_eq!(text(&out.stdout), "196418\nrun\n", "{}", text(&out.stderr));
        start.elapsed()
    };
    let plain = std::env::temp_dir().join(format!("kova-plain-{}.kova", std::process::id()));
    std::fs::write(&plain, std::fs::read_to_string("tests/modes/discreet.kova").unwrap().replace("    mode build\n", "")).unwrap();
    let (native, interpreted) = (timed(Path::new("tests/modes/discreet.kova")), timed(&plain));
    let _ = std::fs::remove_file(&plain);
    // Building takes its share on a slow machine (an Intel Mac of the CI: 0.5 s against 1.4 s).
    assert!(native * 2 < interpreted, "native {native:?}, interpreted {interpreted:?}");
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

/// The editor's address of a file: `file:///home/me/x.kova`, `file:///C:/Users/me/x.kova`.
fn file_uri(path: &Path) -> String {
    let path = path.display().to_string().replace('\\', "/");
    format!("file://{}{path}", if path.starts_with('/') { "" } else { "/" })
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
            assert!(output.read_line(&mut line).unwrap() > 0, "kova lsp stopped");
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
    let capabilities = receive()["result"]["capabilities"].clone();
    assert_eq!(capabilities["completionProvider"]["triggerCharacters"], serde_json::json!([".", " "]));
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

    // A space asks for completion: after `import ` it offers the modules, elsewhere nothing.
    let spaced = |id: u32, line: u32, character: u32| serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "textDocument/completion",
        "params": {"textDocument": {"uri": uri}, "position": {"line": line, "character": character},
            "context": {"triggerKind": 2, "triggerCharacter": " "}}});
    send(spaced(3, 1, 8));
    assert!(receive()["result"].is_null());
    send(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": format!("import \n{text}")}]}}));
    receive();
    send(spaced(4, 0, 7));
    let modules = receive()["result"].clone();
    let labels: Vec<&str> = modules.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.starts_with(&["web", "files"]), "{labels:?}");

    // An error in an imported file: shown on the `import` line, with its real place.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/error/import_error");
    let main = file_uri(&dir.join("main.kova"));
    let text = std::fs::read_to_string(dir.join("main.kova")).unwrap();
    send(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": main, "languageId": "kova", "version": 1, "text": text}}}));
    let diag = receive();
    let error = &diag["params"]["diagnostics"][0];
    assert!(error["message"].as_str().unwrap().contains("broken.kova:4:16: different types"), "{diag}");
    assert_eq!(error["range"]["start"], serde_json::json!({"line": 0, "character": 7}));

    // Imported through another file: shown on the `import` of the file that leads to it.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/error/import_deep");
    let main = file_uri(&dir.join("main.kova"));
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
                    assert!(self.output.read_line(&mut line).unwrap() > 0, "kova debug stopped");
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
        /// What the console shows for `expression` (`frame`: as `stackTrace` numbers them), or its error.
        fn evaluate(&mut self, expression: &str, frame: Option<u64>) -> String {
            let r = self.ask("evaluate", json!({"expression": expression, "frameId": frame, "context": "repl"}));
            if r["success"] == true { r["body"]["result"].as_str().unwrap().to_string() } else { format!("error: {}", r["message"].as_str().unwrap()) }
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
    // The console: what `print` would write; the variables are only borrowed.
    assert_eq!(e.evaluate("rex.age + 1", None), "4");
    assert_eq!(e.evaluate("\"{rex.name} has {names.length} names\"", Some(0)), "Rex has 2 names");
    assert_eq!(e.evaluate("names", None), "[\"a\", \"b\"]");
    assert_eq!(e.evaluate("older(rex)", None), "4");
    assert_eq!(e.evaluate("nope + 1", None), "error: unknown variable `nope`");
    assert!(e.evaluate("names[5]", None).starts_with("error: error: index out of bounds"));
    assert!(e.evaluate("names.add(\"c\")", None).starts_with("error: "));
    assert_eq!(e.go("stepIn"), "step");
    assert_eq!(e.stack(), [("older".to_string(), 8), ("main".to_string(), 15)]);
    e.go("next");
    assert_eq!(e.variables(1), ["d = Dog { name: \"Rex\", age: 3 }", "years = 4"]);
    assert_eq!(e.evaluate("years * d.age", Some(1)), "12");
    assert_eq!(e.evaluate("names.length", Some(0)), "2");
    e.go("stepOut");
    assert_eq!(e.stack(), [("main".to_string(), 16)]);
    assert_eq!(e.go("continue"), "breakpoint");
    assert_eq!(e.variables(0), ["rex = (given away)", "names = [\"a\", \"b\"]", "next = 4", "gone = Dog { name: \"Rex\", age: 3 }"]);
    assert_eq!(e.evaluate("rex.name", None), "(given away)");
    assert_eq!(e.evaluate("gone.name", None), "Rex");
    e.send("continue", json!({"threadId": 1}));
    assert_eq!(e.until(|m| m["event"] == "exited")["body"]["exitCode"], 0);
    assert_eq!(e.printed, "4\n3 2\n");
    e.send("disconnect", json!({}));
    assert!(adapter.wait().unwrap().success());

    // In a method: `self` too.
    let mut adapter = kova(LANG).arg("debug").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut e = Editor { input: adapter.stdin.take().unwrap(), output: BufReader::new(adapter.stdout.take().unwrap()), seq: 0, printed: String::new() };
    let path = std::fs::canonicalize("tests/debug/method.kova").unwrap().display().to_string();
    e.send("initialize", json!({"adapterID": "kova"}));
    e.ask("launch", json!({"program": path}));
    e.ask("setBreakpoints", json!({"source": {"path": path}, "breakpoints": [{"line": 7}]}));
    e.send("configurationDone", json!({}));
    assert_eq!(e.until(|m| m["event"] == "stopped")["body"]["reason"], "breakpoint");
    assert_eq!(e.evaluate("self.name", None), "Rex");
    assert_eq!(e.evaluate("\"{self.name}: {word}\" + \"!\".repeat(times)", None), "Rex: REX!!");
    assert_eq!(e.evaluate("myself", None), "error: unknown variable `myself`");
    e.send("continue", json!({"threadId": 1}));
    assert_eq!(e.until(|m| m["event"] == "exited")["body"]["exitCode"], 0);
    assert_eq!(e.printed, "REXREX\n");
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

/// `get` and `post` (PLAN Part 4 "The web"), in run and in build, against a small server
/// answering by path: every kind of answer, and every failure that needs no other machine.
#[test]
fn web() {
    use std::io::Read;
    use std::net::TcpListener;
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port().to_string();
    // A port where nothing answers: taken, then given back.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
    std::thread::spawn(move || {
        for link in server.incoming() {
            let mut link = link.unwrap();
            // One thread each: `/slow` must not hold the others.
            std::thread::spawn(move || {
            let mut got = vec![];
            let mut buffer = [0u8; 4096];
            // The request: its head, then as many bytes as its Content-Length says.
            let (head, body) = loop {
                let n = link.read(&mut buffer).unwrap();
                got.extend_from_slice(&buffer[..n]);
                let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let head = String::from_utf8_lossy(&got[..end]).into_owned();
                let length = head.lines().find_map(|l| l.strip_prefix("Content-Length: ")).map_or(0, |n| n.parse().unwrap());
                if got.len() >= end + 4 + length {
                    break (head, got[end + 4..].to_vec());
                }
                if n == 0 {
                    panic!("request cut");
                }
            };
            let mut words = head.split(' ');
            let method = words.next().unwrap().to_string();
            let path = words.next().unwrap().to_string();
            let header = |name: &str| head.lines().find_map(|l| l.split_once(": ").filter(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.to_string())).unwrap_or_default();
            let answer = |status: &str, headers: &str, body: &str| {
                format!("HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
            };
            let reply = match path.as_str() {
                "/hello" => answer("200 OK", "Content-Type: text/plain\r\n", "hello"),
                "/missing" => answer("404 Not Found", "", "not here"),
                "/pieces" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nin \r\n6\r\nthree \r\n6\r\npieces\r\n0\r\n\r\n".to_vec(),
                "/until-closed" => b"HTTP/1.1 200 OK\r\n\r\nuntil the end".to_vec(),
                "/echo" => {
                    let kind = head.lines().find_map(|l| l.strip_prefix("Content-Type: ")).unwrap_or("");
                    answer("200 OK", &format!("Content-Type: {kind}\r\n"), &String::from_utf8_lossy(&body))
                }
                "/moved" => answer("302 Found", "Location: /hello\r\n", ""),
                "/twice" => answer("200 OK", "Set-Cookie: a=1\r\nset-cookie: b=2\r\n", ""),
                "/loop" => answer("301 Moved Permanently", "Location: /loop\r\n", ""),
                "/asked" => answer("200 OK", "", &format!("{method}|{}|{}|{}", header("user-agent"), header("x-kind"), String::from_utf8_lossy(&body))),
                "/slow" => {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    answer("200 OK", "", "too late")
                }
                _ => b"nonsense\r\n\r\n".to_vec(),
            };
            let _ = link.write_all(&reply);
            });
        }
    });

    let source = "tests/web/client.kova";
    let expected = Expected {
        output: expectations(&std::fs::read_to_string(source).unwrap(), "expect")
            .iter()
            .map(|l| l.replace("PORT", &port).replace("CLOSED", &closed))
            .collect(),
        code: 0,
        stderr: vec![],
    };
    let mut failures = vec![];
    let run = kova(LANG).arg("run").arg(source).args([&port, &closed]).output().unwrap();
    check_output("web", "run", &run, &expected, &mut failures);
    let exe = std::env::temp_dir().join(format!("kova-web-{}{EXE}", std::process::id()));
    let out = kova(LANG).arg("build").arg(source).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let build = program(&exe, LANG).args([&port, &closed]).output().unwrap();
    let _ = std::fs::remove_file(&exe);
    check_output("web", "build", &build, &expected, &mut failures);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

/// `serve` (PLAN Part 4 "The web server"), in run and in build: tests/web/server.kova answers what
/// is asked here, then `/stop` ends it. With `KOVA_LEAKS=live`, its `exit` says how many Strings and
/// Lists are alive: the same after 1 round of questions and after 4, so no question leaves any.
#[test]
fn server() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    let source = "tests/web/server.kova";
    let ask = |port: u16, request: &[u8]| -> String {
        // The server may still be starting.
        for _ in 0..400 {
            if let Ok(mut link) = TcpStream::connect(("127.0.0.1", port)) {
                // A server stuck on another connection fails here instead of answering late.
                link.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
                link.write_all(request).unwrap();
                let mut got = vec![];
                let _ = link.read_to_end(&mut got);
                return String::from_utf8_lossy(&got).replace("\r\n", "\n");
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("no server on port {port}");
    };
    let get = |path: &str| format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").into_bytes();
    let head = |status: &str, kind: &str, length: usize| format!("HTTP/1.1 {status}\ncontent-type: {kind}\ncontent-length: {length}\nconnection: close\n\n");
    let plain = "text/plain; charset=utf-8";
    let round: Vec<(Vec<u8>, String)> = vec![
        (get("/"), head("200 OK", plain, 15) + "Hello from Kova"),
        (get("/book"), head("200 OK", "application/json", 28) + r#"{"title":"Dune","pages":412}"#),
        (get("/books"), head("200 OK", "application/json", 25) + r#"[{"title":"A","pages":1}]"#),
        (get("/search?author=Frank+Herbert%21"), head("200 OK", plain, 22) + "author: Frank Herbert!"),
        (get("/search"), head("400 Bad Request", plain, 15) + "400 Bad Request"),
        (get("/html"), head("200 OK", "text/html", 9) + "<p>hi</p>"),
        (b"POST /echo HTTP/1.1\r\nX-Kova: yes\r\nContent-Length: 8\r\n\r\nhi there".to_vec(), head("200 OK", plain, 17) + "POST hi there yes"),
        (get("/style.css"), head("200 OK", "text/css; charset=utf-8", 20) + "body { color: red }\n"),
        (get("/docs/dot.png"), head("200 OK", "image/png", 11) + &String::from_utf8_lossy(b"\x89PNG\n\x1a\n\x00\x01\xff")),
        (get("/docs/"), head("404 Not Found", plain, 13) + "404 Not Found"),
        (get("/../Cargo.toml"), head("404 Not Found", plain, 13) + "404 Not Found"),
        (get("/%2e%2e/Cargo.toml"), head("404 Not Found", plain, 13) + "404 Not Found"),
        // Refused before the program sees it.
        (b"NONSENSE\r\n\r\n".to_vec(), head("400 Bad Request", plain, 15) + "400 Bad Request"),
        (b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(), head("501 Not Implemented", plain, 19) + "501 Not Implemented"),
    ];
    let exe = std::env::temp_dir().join(format!("kova-server-{}{EXE}", std::process::id()));
    let out = kova(LANG).arg("build").arg(source).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let mut failures = vec![];
    for mode in ["run", "build"] {
        let mut alive = vec![];
        for rounds in [1, 4] {
            let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let mut c = if mode == "run" { kova(LANG) } else { program(&exe, LANG) };
            if mode == "run" {
                c.arg("run").arg(source);
            }
            let child = c.arg(port.to_string()).env("KOVA_LEAKS", "live").stderr(std::process::Stdio::piped()).spawn().unwrap();
            for _ in 0..rounds {
                for (question, want) in &round {
                    let got = ask(port, question);
                    if got != *want {
                        failures.push(format!("{mode}: {:?}\n  expected: {want:?}\n  got:      {got:?}", String::from_utf8_lossy(question)));
                    }
                }
            }
            if rounds == 1 {
                // A connection that says nothing (browsers open some in advance) and one that
                // stops halfway do not keep the others waiting.
                let silent = TcpStream::connect(("127.0.0.1", port)).unwrap();
                let mut half = TcpStream::connect(("127.0.0.1", port)).unwrap();
                half.write_all(b"GET / HTTP/1.1\r\nHo").unwrap();
                let got = ask(port, &get("/html"));
                if got != round[5].1 {
                    failures.push(format!("{mode}: behind a silent connection: {got:?}"));
                }
                half.write_all(b"st: x\r\n\r\n").unwrap();
                half.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
                let mut got = vec![];
                let _ = half.read_to_end(&mut got);
                if !String::from_utf8_lossy(&got).ends_with("Hello from Kova") {
                    failures.push(format!("{mode}: a question sent in two parts: {:?}", String::from_utf8_lossy(&got)));
                }
                drop(silent);
                // The port is taken: a second server stops at once.
                let mut second = if mode == "run" { kova(LANG) } else { program(&exe, LANG) };
                if mode == "run" {
                    second.arg("run").arg(source);
                }
                let second = second.arg(port.to_string()).output().unwrap();
                if second.status.code() != Some(101) || !text(&second.stderr).contains("cannot serve on this port") {
                    failures.push(format!("{mode}: a second server on the same port: {second:?}"));
                }
            }
            ask(port, &get("/stop"));
            let end = child.wait_with_output().unwrap();
            if end.status.code() != Some(0) {
                failures.push(format!("{mode}: ended with {:?}: {}", end.status, text(&end.stderr)));
            }
            // What it said: the address when it started, a line per question, then what is alive.
            let said = text(&end.stderr);
            let start = format!("\u{2713} Serving on port {port}\n  http://localhost:{port}/    this computer only\n  Stop with Ctrl+C\n");
            if !said.starts_with(&start) {
                failures.push(format!("{mode}: when it starts: {said:?}"));
            }
            let book = said.lines().find(|l| l.ends_with("  127.0.0.1  GET /book  200"));
            if book.is_none_or(|l| l.len() != "12:00:00  127.0.0.1  GET /book  200".len()) || !said.contains("  GET /../Cargo.toml  404\n") {
                failures.push(format!("{mode}: the questions written: {said:?}"));
            }
            alive.push(said.lines().last().unwrap_or("").to_string());
        }
        if alive[0] != alive[1] || !alive[0].starts_with("alive: ") {
            failures.push(format!("{mode}: what is alive grows with the questions: {alive:?}"));
        }
    }
    let _ = std::fs::remove_file(&exe);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

/// `serve(port) workers(4)` (PLAN Part 4 "Several questions at once"): built, a quick question is
/// answered while a slow one is still running; many at once get the right answers; what is alive
/// does not grow with them. `kova run` answers the same, one at a time.
#[test]
fn server_workers() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Instant;
    let ask = |port: u16, path: &str| -> String {
        for _ in 0..400 {
            if let Ok(mut link) = TcpStream::connect(("127.0.0.1", port)) {
                link.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
                link.write_all(format!("GET {path} HTTP/1.1\r\n\r\n").as_bytes()).unwrap();
                let mut got = vec![];
                let _ = link.read_to_end(&mut got);
                let got = String::from_utf8_lossy(&got).into_owned();
                return got.split("\r\n\r\n").nth(1).unwrap_or(&got).to_string();
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("no server on port {port}");
    };
    let mut failures = vec![];
    // The second one, built for Linux, makes its threads with the C library's `pthread_create`.
    for (source, modes) in [("tests/web/workers.kova", &["run", "build"][..]), ("tests/web/workers_web.kova", &["build"])] {
        let exe = std::env::temp_dir().join(format!("kova-workers-{}{EXE}", std::process::id()));
        let out = kova(LANG).arg("build").arg(source).arg("-o").arg(&exe).output().unwrap();
        assert!(out.status.success(), "{}", text(&out.stderr));
        for &mode in modes {
            let mut alive = vec![];
            for many in [8, 32] {
                let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
                let mut c = if mode == "run" { kova(LANG) } else { program(&exe, LANG) };
                if mode == "run" {
                    c.arg("run").arg(source);
                }
                let child = c.arg(port.to_string()).env("KOVA_LEAKS", "live").stderr(std::process::Stdio::piped()).spawn().unwrap();
                assert_eq!(ask(port, "/ada"), "hello 36");
                // The slow one first: built, the quick one is answered before it.
                let slow = std::thread::spawn(move || (ask(port, "/slow"), Instant::now()));
                std::thread::sleep(std::time::Duration::from_millis(300));
                let (quick, quick_at) = (ask(port, "/alan"), Instant::now());
                let (slow, slow_at) = slow.join().unwrap();
                if quick != "hello 41" || slow != "slow" {
                    failures.push(format!("{mode}: {quick:?} {slow:?}"));
                }
                if mode == "build" && quick_at > slow_at {
                    failures.push("build: the quick question waited for the slow one".into());
                }
                let askers: Vec<_> = (0..many)
                    .map(|k| std::thread::spawn(move || (0..10).map(|_| ask(port, if k % 2 == 0 { "/ada" } else { "/nobody" })).collect::<Vec<_>>()))
                    .collect();
                for (k, a) in askers.into_iter().enumerate() {
                    let want = if k % 2 == 0 { "hello 36" } else { "404 Not Found" };
                    if let Some(got) = a.join().unwrap().into_iter().find(|g| g != want) {
                        failures.push(format!("{mode}: {got:?} instead of {want:?}"));
                    }
                }
                ask(port, "/stop");
                let end = child.wait_with_output().unwrap();
                let said = text(&end.stderr);
                if end.status.code() != Some(0) {
                    failures.push(format!("{mode}: ended with {:?}: {said}", end.status));
                }
                let n: Option<i64> = said.lines().last().and_then(|l| l.strip_prefix("alive: ")?.parse().ok());
                alive.push(n);
            }
            // Each thread may have made its own Map's index meanwhile (4 threads: 3 more at most).
            match (alive[0], alive[1]) {
                (Some(a), Some(b)) if (b - a).abs() <= 3 => {}
                _ => failures.push(format!("{source} {mode}: what is alive grows with the questions: {alive:?}")),
            }
        }
        let _ = std::fs::remove_file(&exe);
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

/// `l.map workers(4) { … }` (PLAN Part 4 "Several things at once"), built: four elements that
/// each sleep a second take about one second, and come back in order.
#[test]
fn parallel_lists() {
    let exe = std::env::temp_dir().join(format!("kova-sleepy-{}{EXE}", std::process::id()));
    let out = kova(LANG).arg("build").arg("tests/parallel/sleepy.kova").arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let start = std::time::Instant::now();
    let out = program(&exe, LANG).output().unwrap();
    let took = start.elapsed();
    let _ = std::fs::remove_file(&exe);
    assert_eq!(text(&out.stdout), "[10, 20, 30, 40]\n", "{}", text(&out.stderr));
    assert!(took.as_secs_f64() < 3.0, "4 seconds of sleep on 4 threads took {took:?}");
}

/// `log(Log.Off)` writes nothing, `log(Log.Start)` only the addresses (PLAN Part 4 "The web
/// server, what it says"), in run and in build.
#[test]
fn server_log() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    let source = "tests/web/quiet.kova";
    let exe = std::env::temp_dir().join(format!("kova-quiet-{}{EXE}", std::process::id()));
    let out = kova(LANG).arg("build").arg(source).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let mut failures = vec![];
    for mode in ["run", "build"] {
        for log in ["off", "start"] {
            let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let mut c = if mode == "run" { kova(LANG) } else { program(&exe, LANG) };
            if mode == "run" {
                c.arg("run").arg(source);
            }
            let mut child = c.arg(port.to_string()).arg(log).stderr(std::process::Stdio::piped()).spawn().unwrap();
            let mut answered = false;
            for _ in 0..400 {
                if let Ok(mut link) = TcpStream::connect(("127.0.0.1", port)) {
                    link.write_all(b"GET /style.css HTTP/1.1\r\n\r\n").unwrap();
                    let _ = link.read_to_end(&mut vec![]);
                    answered = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if log == "start" {
                let _ = child.kill();
            }
            let end = child.wait_with_output().unwrap();
            let want = match log {
                "off" => String::new(),
                _ => format!("\u{2713} Serving tests/web/site/ on port {port}\n  http://localhost:{port}/    this computer only\n  Stop with Ctrl+C\n"),
            };
            if !answered || text(&end.stderr) != want {
                failures.push(format!("{mode}, {log}: {:?}", text(&end.stderr)));
            }
        }
    }
    let _ = std::fs::remove_file(&exe);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

/// `serve_network` answers on the computer's own network address, `serve` does not (PLAN Part 4
/// "The web server"), in run and in build. Skipped on a computer with no network address.
#[test]
fn server_network() {
    use std::io::{Read, Write};
    use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
    // Which address this computer would send from: connecting UDP sends nothing.
    let Some(ip) = UdpSocket::bind("0.0.0.0:0").and_then(|u| u.connect("192.0.2.1:80").and(u.local_addr())).ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback() && matches!(ip, IpAddr::V4(_))) else {
        eprintln!("server_network: no network address, skipped");
        return;
    };
    let source = "tests/web/network.kova";
    let ask = |ip: IpAddr, port: u16| -> Option<String> {
        let mut link = TcpStream::connect_timeout(&(ip, port).into(), std::time::Duration::from_secs(2)).ok()?;
        link.write_all(b"GET / HTTP/1.1\r\n\r\n").ok()?;
        let mut got = String::new();
        link.read_to_string(&mut got).ok()?;
        got.rsplit("\r\n").next().map(str::to_string)
    };
    let local: IpAddr = [127, 0, 0, 1].into();
    let exe = std::env::temp_dir().join(format!("kova-network-{}{EXE}", std::process::id()));
    let out = kova(LANG).arg("build").arg(source).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let mut failures = vec![];
    for mode in ["run", "build"] {
        for open in [true, false] {
            let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let mut c = if mode == "run" { kova(LANG) } else { program(&exe, LANG) };
            if mode == "run" {
                c.arg("run").arg(source);
            }
            c.arg(port.to_string());
            if !open {
                c.arg("here");
            }
            let mut child = c.spawn().unwrap();
            // Ready once it answers on this computer.
            let mut first = None;
            for _ in 0..400 {
                first = ask(local, port);
                if first.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            let want = if open { "everyone" } else { "here" };
            if first.as_deref() != Some(want) {
                failures.push(format!("{mode}, open {open}: on 127.0.0.1: {first:?}"));
            }
            let far = ask(ip, port);
            if far.as_deref() != if open { Some(want) } else { None } {
                failures.push(format!("{mode}, open {open}: on {ip}: {far:?}"));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let _ = std::fs::remove_file(&exe);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

/// HTTPS against real sites (PLAN Part 4 "The web, step 3"), in run and in build: only where the
/// network is promised (`KOVA_NETWORK`, set by the GitHub workflow).
#[test]
fn https() {
    if std::env::var_os("KOVA_NETWORK").is_none() {
        return;
    }
    let source = "tests/web/https.kova";
    let expected = Expected { output: expectations(&std::fs::read_to_string(source).unwrap(), "expect"), code: 0, stderr: vec![] };
    let exe = std::env::temp_dir().join(format!("kova-https-{}{EXE}", std::process::id()));
    let out = kova(LANG).arg("build").arg(source).arg("-o").arg(&exe).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    // badssl.com sometimes refuses or drops the runners' connections: 3 tries each.
    let mut failures = vec![];
    for mode in ["run", "build"] {
        let mut tries = vec![];
        for _ in 0..3 {
            let out = if mode == "run" { kova(LANG).arg("run").arg(source).output() } else { program(&exe, LANG).output() };
            tries.clear();
            check_output("https", mode, &out.unwrap(), &expected, &mut tries);
            if tries.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        failures.extend(tries);
    }
    let _ = std::fs::remove_file(&exe);
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

/// `system.run` (PLAN Part 4 "The library after Python", step 1): `run.kova` runs the built
/// `child.kova`, in run and in build: words to quote (Windows' command line), what the child
/// writes on stderr, its exit code, and a program that is nowhere.
#[test]
fn run_programs() {
    let id = std::process::id();
    let child = std::env::temp_dir().join(format!("kova-child-{id}{EXE}"));
    let parent = std::env::temp_dir().join(format!("kova-parent-{id}{EXE}"));
    for (source, exe) in [("tests/system/child.kova", &child), ("tests/system/run.kova", &parent)] {
        let out = kova(LANG).arg("build").arg(source).arg("-o").arg(exe).output().unwrap();
        assert!(out.status.success(), "{}", text(&out.stderr));
    }
    let run = kova(LANG).arg("run").arg("tests/system/run.kova").arg(&child).output().unwrap();
    let built = program(&parent, LANG).arg(&child).output().unwrap();
    let _ = std::fs::remove_file(&child);
    let _ = std::fs::remove_file(&parent);
    let expected = "[]\né ok\nerrors: false, code 0\n[\"a b\", \"\", \"say \"hi\"\", \"back\\\", \"tab\there\"]\né ok\nerrors: false, code 0\n\
        [\"fail\"]\né ok\nerrors: true, code 101\ntrue\n[\"three\"]\né ok\nerrors: false, code 3\nNotFound(\"kova-no-such-program\")\n";
    assert_eq!(text(&run.stdout), expected, "run: {}", text(&run.stderr));
    assert_eq!(text(&built.stdout), expected, "build: {}", text(&built.stderr));
}
