// Entry point: reads the command line and runs the right action.
// See docs/PLAN.md, Part 1 (commands), 2.11 (header) and 2.13 (translations).

mod ast;
mod codegen;
mod diag;
mod interp;
mod kir;
mod lexer;
mod locale;
mod lsp;
mod parser;
mod types;

use std::process::exit;

use locale::t;

enum Command {
    Run,
    Build { output: Option<String>, boost: bool },
    Test,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let (command, file) = match args.as_slice() {
        ["--version" | "-V"] => {
            println!("kova {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        ["lsp"] => {
            if let Err(e) = lsp::serve() {
                fail(t!("lsp_failed", error = e));
            }
            return;
        }
        ["run", f] => (Some(Command::Run), *f),
        ["test", f] => (Some(Command::Test), *f),
        ["build", f, options @ ..] => match build_options(options) {
            Some((output, boost)) => (Some(Command::Build { output, boost }), *f),
            None => usage_error(),
        },
        // Without a command: the file's `header` decides (run by default).
        [f] if f.ends_with(".kova") => (None, *f),
        _ => usage_error(),
    };

    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => fail(t!("cannot_read", file = file, error = e)),
    };

    let (tree, info, diags) = analyze(&source);
    let Some(info) = info else {
        for d in &diags {
            eprintln!("{}", diag::render(d, file, &source));
        }
        exit(1);
    };

    // A translation file: `kova run` checks it, it is not a program.
    if !tree.langs.is_empty() {
        match command {
            None | Some(Command::Run) => exit(check_translations(file, &source)),
            Some(_) => fail(t!("lang_file_not_program", file = file)),
        }
    }

    // The header: `mode` picks the default command, `lock := true` refuses the other one.
    let mode = types::setting(&tree, &source, "mode").map_or("run", |(v, _)| v);
    let command = command.unwrap_or(if mode == "build" {
        Command::Build { output: None, boost: false }
    } else {
        Command::Run
    });
    if let Some(("true", line)) = types::setting(&tree, &source, "lock") {
        let other = matches!((&command, mode), (Command::Run, "build") | (Command::Build { .. }, "run"));
        if other {
            fail(t!("reserved_for", mode = mode, line = line));
        }
    }

    let contracts = !matches!(command, Command::Build { boost: true, .. });
    let program = kir::translate(&tree, &info, &source, contracts, matches!(command, Command::Test));

    match command {
        Command::Run => exit(interp::run(&program, file)),
        Command::Build { output, .. } => {
            let output = output.unwrap_or_else(|| file.trim_end_matches(".kova").to_string());
            if output == file {
                fail(t!("output_overwrites", file = file));
            }
            let lang = types::setting(&tree, &source, "lang").and_then(|(code, _)| locale::recognize(code));
            if let Err(e) = codegen::build(&program, file, &output, lang) {
                fail(e);
            }
        }
        Command::Test => exit(interp::test(&program, file)),
    }
}

/// Lexer + parser + type checker. Reads in-memory text: `kova lsp` calls it on
/// text that isn't saved yet. `Some(info)` only if there is no error at all.
/// Also picks the language of the messages: the `header`'s, otherwise the system's.
fn analyze(source: &str) -> (ast::File, Option<types::Info>, Vec<diag::Diag>) {
    let read = || {
        let (tokens, mut diags) = lexer::lex(source);
        let (file, nb_exprs, d) = parser::parse(tokens, source);
        diags.extend(d);
        (file, nb_exprs, diags)
    };
    let (mut file, mut nb_exprs, mut diags) = read();
    let lang = file.header.iter().flat_map(|h| &h.settings).find(|(k, _)| k.text == "lang").map(|(_, v)| v.text.as_str());
    if locale::choose(lang) {
        // The messages were written in the other language: read again.
        (file, nb_exprs, diags) = read();
    }
    // Syntax errors: stop here, otherwise the type checker would invent more of them
    // (a variable whose `let` is unreadable would be "unknown" everywhere).
    if diags.is_empty() {
        let (info, d) = types::check(&file, nb_exprs, source);
        if d.is_empty() {
            return (file, Some(info), d);
        }
        diags = d;
    }
    diags.sort_by_key(|d| d.span.start);
    (file, None, diags)
}

/// Reads `-o output` and `--boost`, in any order.
fn build_options(options: &[&str]) -> Option<(Option<String>, bool)> {
    let (mut output, mut boost) = (None, false);
    let mut i = 0;
    while i < options.len() {
        match options[i] {
            "--boost" => boost = true,
            "-o" => {
                output = Some(options.get(i + 1)?.to_string());
                i += 1;
            }
            _ => return None,
        }
        i += 1;
    }
    Some((output, boost))
}

fn usage_error() -> ! {
    eprintln!("{}", t!("usage"));
    exit(2);
}

/// `error: ...` and code 1.
fn fail(message: String) -> ! {
    eprintln!("{}{message}", t!("error_prefix"));
    exit(1);
}

/// `kova run fr.kova`: compares the keys of a translation file with those of the other
/// translation files of its folder. Code 1 if a key is missing or its `{holes}` differ.
fn check_translations(file: &str, source: &str) -> i32 {
    let mine = locale::read(source);
    println!("{}", t!("lang_check_count", file = file, count = mine.len()));
    let path = std::path::Path::new(file);
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let mut others: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "kova") && p.file_name() != path.file_name())
        .filter_map(|p| {
            let src = std::fs::read_to_string(&p).ok()?;
            let langs = parser::parse(lexer::lex(&src).0, &src).0.langs;
            (!langs.is_empty()).then(|| (p.file_name().unwrap().to_string_lossy().into_owned(), locale::read(&src)))
        })
        .collect();
    others.sort_by(|a, b| a.0.cmp(&b.0));
    if others.is_empty() {
        println!("{}", t!("lang_check_alone"));
        return 0;
    }
    let mut code = 0;
    for (other, theirs) in &others {
        let find = |key: &str| mine.iter().find(|(k, ..)| k == key);
        let missing: Vec<String> = theirs.iter().filter(|(k, ..)| find(k).is_none()).map(|(k, ..)| k.clone()).collect();
        let holes: Vec<String> = theirs
            .iter()
            .filter(|(k, s, _)| find(k).is_some_and(|(_, m, _)| locale::holes(m) != locale::holes(s)))
            .map(|(k, ..)| k.clone())
            .collect();
        if !missing.is_empty() {
            println!("{}", t!("lang_check_missing", other = other, keys = missing.join(", ")));
            code = 1;
        }
        if !holes.is_empty() {
            println!("{}", t!("lang_check_holes", other = other, keys = holes.join(", ")));
            code = 1;
        }
    }
    if code == 0 {
        let names: Vec<&str> = others.iter().map(|(n, _)| n.as_str()).collect();
        println!("{}", t!("lang_check_ok", others = names.join(", ")));
    }
    code
}
