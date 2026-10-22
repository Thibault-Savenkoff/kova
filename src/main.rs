// Entry point: reads the command line and runs the right action.
// See docs/PLAN.md, Part 1 (commands), 2.11 (header) and 2.13 (translations).

mod ast;
mod codegen;
mod debug;
mod diag;
mod format;
mod interp;
mod kir;
mod lexer;
mod locale;
mod lsp;
mod ownership;
mod parser;
mod types;

use std::process::exit;

use locale::t;

enum Command {
    Run,
    Build { output: Option<String>, boost: bool },
    /// `kova test file.kova "name"`: only that test.
    Test { only: Option<String> },
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let (command, file) = match args.as_slice() {
        ["--version" | "-V"] => {
            println!("kova {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        ["fmt", files @ ..] if !files.is_empty() => exit(fmt(files)),
        // The debugger, for the editor (DAP on stdin/stdout): the editor says which program.
        ["debug"] => exit(debug::serve()),
        ["lsp"] => {
            if let Err(e) = lsp::serve() {
                fail(t!("lsp_failed", error = e));
            }
            return;
        }
        // The words after the file go to the program (`args`).
        ["run", f, ..] => (Some(Command::Run), *f),
        ["test", f] => (Some(Command::Test { only: None }), *f),
        ["test", f, name] => (Some(Command::Test { only: Some(name.to_string()) }), *f),
        ["build", f, options @ ..] => match build_options(options) {
            Some((output, boost)) => (Some(Command::Build { output, boost }), *f),
            None => usage_error(),
        },
        // Without a command: the file's `header` decides (run by default).
        [f, ..] if f.ends_with(".kova") => (None, *f),
        _ => usage_error(),
    };

    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => fail(t!("cannot_read", file = file, error = e)),
    };

    let (tree, info, diags, sources) = analyze(file, &source);
    let (Some(info), true) = (info, diags.is_empty()) else {
        for d in &diags {
            eprintln!("{}", diag::render(d, &sources));
        }
        exit(1);
    };
    let files: Vec<String> = sources.files.iter().map(|(path, _)| path.clone()).collect();

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

    if !matches!(command, Command::Test { .. }) && !tree.functions.iter().any(|f| f.name.text == "main") {
        eprintln!("{}", diag::render(&diag::Diag::new(diag::Span::new(0, 0), t!("main_missing")), &sources));
        exit(1);
    }
    let contracts = !matches!(command, Command::Build { boost: true, .. });
    let program = kir::translate(&tree, &info, &sources.text, contracts, matches!(command, Command::Test { .. }), matches!(command, Command::Build { .. }), false);

    match command {
        Command::Run => {
            let skip = if args[0] == "run" { 2 } else { 1 };
            let given: Vec<String> = args[skip..].iter().map(|a| a.to_string()).collect();
            exit(interp::run(&program, &files, &given))
        }
        Command::Build { output, .. } => {
            let output = output.unwrap_or_else(|| file.trim_end_matches(".kova").to_string());
            if output == file {
                fail(t!("output_overwrites", file = file));
            }
            let lang = types::setting(&tree, &source, "lang").and_then(|(code, _)| locale::recognize(code));
            if let Err(e) = codegen::build(&program, &files, &output, lang) {
                fail(e);
            }
        }
        Command::Test { only } => exit(interp::test(&program, &files, only.as_deref())),
    }
}

/// `kova fmt a.kova b.kova`: lays each file out in place (`format.rs`). Code 1 if one doesn't read.
fn fmt(files: &[&str]) -> i32 {
    let mut code = 0;
    for file in files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => fail(t!("cannot_read", file = file, error = e)),
        };
        match format::format(&source) {
            Some(tidy) if tidy != source => {
                if let Err(e) = std::fs::write(file, tidy) {
                    fail(t!("cannot_write", file = file, error = e));
                }
            }
            Some(_) => {}
            None => {
                eprintln!("{}", t!("fmt_unreadable", file = file));
                code = 1;
            }
        }
    }
    code
}

/// Lexer + parser + type checker. Reads in-memory text for the file `path`: `kova lsp` calls
/// it on text that isn't saved yet (the files it imports are read from the disk).
/// `Some(info)` once the types are checked, even with errors: the editor still shows them.
/// Also picks the language of the messages: the `header`'s, otherwise the system's.
fn analyze(path: &str, source: &str) -> (ast::File, Option<types::Info>, Vec<diag::Diag>, diag::Sources) {
    let read = || load(path, source);
    let (mut file, mut nb_exprs, mut diags, mut sources) = read();
    let lang = file.header.iter().flat_map(|h| &h.settings).find(|(k, _)| k.text == "lang").map(|(_, v)| v.text.as_str());
    if locale::choose(lang) {
        // The messages were written in the other language: read again.
        (file, nb_exprs, diags, sources) = read();
    }
    let source = sources.text.as_str();
    // Syntax errors: stop here, otherwise the type checker would invent more of them
    // (a variable whose `let` is unreadable would be "unknown" everywhere).
    if diags.is_empty() {
        let (mut info, d) = types::check(&file, nb_exprs, source);
        // Ownership only on a well-typed program: it relies on the types.
        diags = if d.is_empty() { ownership::check(&file, &mut info, source) } else { d };
        diags.sort_by_key(|d| d.span.start);
        return (file, Some(info), diags, sources);
    }
    diags.sort_by_key(|d| d.span.start);
    (file, None, diags, sources)
}

/// Reads the file `path` (its text: `source`) and the files it imports, one after the other in
/// one text (`diag::Sources`), into a single tree: the imported files' names come qualified
/// (`geometry.Point`). Their `main`, `test`s and `header` are not used (PLAN 2.9).
fn load(path: &str, source: &str) -> (ast::File, usize, Vec<diag::Diag>, diag::Sources) {
    let mut sources = diag::Sources::new();
    sources.add(path, source);
    let (tokens, mut diags) = lexer::lex(source);
    let (mut root, mut ids, d) = parser::parse(tokens, source);
    diags.extend(d);
    let dir = std::path::Path::new(path).parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut loaded: Vec<ast::File> = vec![];
    let mut done = std::collections::HashSet::new();
    // Depth first: `chain` = the files being read, to refuse a cycle.
    fn visit(
        imports: &[ast::Import],
        origin: Option<diag::Span>,
        chain: &mut Vec<String>,
        done: &mut std::collections::HashSet<String>,
        dir: &std::path::Path,
        sources: &mut diag::Sources,
        ids: &mut usize,
        diags: &mut Vec<diag::Diag>,
        loaded: &mut Vec<ast::File>,
    ) {
        let mut aliases: std::collections::HashMap<&str, diag::Span> = std::collections::HashMap::new();
        for imp in imports {
            let m = &imp.module.text;
            if let Some(first) = aliases.insert(&imp.alias.text, imp.alias.span) {
                let (line, _) = diag::line_col(&sources.text, first.start);
                diags.push(diag::Diag::new(imp.alias.span, t!("import_twice", name = imp.alias.text, line = line)));
                continue;
            }
            if chain.contains(m) {
                let cycle = [&chain[chain.iter().position(|c| c == m).unwrap()..], std::slice::from_ref(m)].concat();
                diags.push(diag::Diag::new(imp.module.span, t!("import_cycle", cycle = cycle.join(" → "))));
                continue;
            }
            if !done.insert(m.clone()) {
                continue;
            }
            let file = dir.join(format!("{m}.kova"));
            let shown = file.to_string_lossy().into_owned();
            let Ok(text) = std::fs::read_to_string(&file) else {
                diags.push(diag::Diag::new(imp.module.span, t!("import_not_found", file = shown)));
                continue;
            };
            let base = sources.add(&shown, &text);
            let origin = origin.unwrap_or(imp.module.span);
            *sources.origins.last_mut().unwrap() = Some(origin);
            let (mut tokens, d) = lexer::lex(&text);
            let shift = |s: diag::Span| diag::Span::new(s.start + base, s.end + base);
            tokens.iter_mut().for_each(|t| t.span = shift(t.span));
            diags.extend(d.into_iter().map(|d| diag::Diag { span: shift(d.span), ..d }));
            let (tree, next, d) = parser::parse_in(tokens, &sources.text, *ids, Some(m));
            *ids = next;
            diags.extend(d);
            for l in &tree.langs {
                diags.push(diag::Diag::new(l.name.span, t!("import_lang_later")));
            }
            chain.push(m.clone());
            visit(&tree.imports, Some(origin), chain, done, dir, sources, ids, diags, loaded);
            chain.pop();
            loaded.push(tree);
        }
    }
    let mut chain = vec![stem];
    let imports = std::mem::take(&mut root.imports);
    visit(&imports, None, &mut chain, &mut done, &dir, &mut sources, &mut ids, &mut diags, &mut loaded);
    root.imports = imports;
    for m in loaded {
        root.functions.extend(m.functions);
        root.structs.extend(m.structs);
        root.interfaces.extend(m.interfaces);
        root.private.extend(m.private);
    }
    (root, ids, diags, sources)
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
