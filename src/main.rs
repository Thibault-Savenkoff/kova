// Entry point: reads the command line and runs the right action.
// See docs/PLAN.md, Part 1 (commands), 2.11 (header) and 2.13 (translations).

extern crate alloc;

mod ast;
mod bigint;
mod codegen;
mod crypto;
mod csv;
mod debug;
mod diag;
mod format;
mod http;
mod icon;
mod json;
#[cfg(target_os = "macos")]
mod network_framework;
#[cfg(target_os = "linux")]
mod openssl;
#[cfg(windows)]
mod schannel;
mod interp;
mod kir;
mod lexer;
mod link;
mod locale;
mod lsp;
mod ownership;
mod packages;
mod parser;
mod regex;
mod shared;
mod types;
mod ui;
mod web;

use std::process::exit;

use locale::t;

enum Command {
    Run,
    Build { output: Option<String>, boost: bool, target: Option<codegen::Target> },
    /// `kova test file.kova "name"`: only that test.
    Test { only: Option<String> },
}

fn main() {
    let started = std::time::Instant::now();
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
            Some((output, boost, target)) => (Some(Command::Build { output, boost, target }), *f),
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

    // Only here may packages be downloaded (never by the editor).
    packages::DOWNLOAD.store(true, std::sync::atomic::Ordering::Relaxed);
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
        Command::Build { output: None, boost: false, target: None }
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
    // The `header`'s language, written into an executable.
    let lang = || types::setting(&tree, &source, "lang").and_then(|(code, _)| locale::recognize(code));
    let contracts = !matches!(command, Command::Build { boost: true, .. });
    let mut program = kir::translate(&tree, &info, &sources.text, contracts, matches!(command, Command::Test { .. }), matches!(command, Command::Build { .. }), false);

    program.icon = icon(file, &tree).and_then(Result::ok);
    match command {
        Command::Run => {
            let skip = if args[0] == "run" { 2 } else { 1 };
            let given: Vec<String> = args[skip..].iter().map(|a| a.to_string()).collect();
            if let Some(code) = discreet(&tree).then(|| native_run(&program, &files, lang(), &given, types::program_name(&tree))).flatten() {
                exit(code)
            }
            // `extern` (PLAN Part 4 "`extern`, step 1"): C is called by native code only; so
            // are the system's windows (`import ui`).
            if !tree.externs.is_empty() || tree.ui_from.is_some() {
                match native_run(&program, &files, lang(), &given, types::program_name(&tree)) {
                    Some(code) => exit(code),
                    None => fail(t!("extern_no_native", file = file)),
                }
            }
            // Each Kova call is a call of the interpreter: a stack as deep as a built program's
            // (a 100,000-deep recursion) needs more than the 8 MB the system gives.
            let code = std::thread::scope(|s| {
                let run = std::thread::Builder::new().stack_size(1 << 30).spawn_scoped(s, || interp::run(&program, &files, &given));
                run.expect("a thread for the interpreter").join().unwrap_or(101)
            });
            exit(code)
        }
        Command::Build { output, target, .. } => {
            let suffix = target.or(codegen::Target::host()).map_or(std::env::consts::EXE_SUFFIX, |t| t.suffix());
            // `header { name := "Counter" }` names the executable, next to the file.
            let named = types::program_name(&tree).map(|n| std::path::Path::new(&file).with_file_name(format!("{n}{suffix}")).to_string_lossy().into_owned());
            let output = output.or(named).unwrap_or_else(|| format!("{}{suffix}", file.trim_end_matches(".kova")));
            if output == file {
                fail(t!("output_overwrites", file = file));
            }
            if let Err(e) = codegen::build(&program, &files, &output, lang(), target) {
                fail(e);
            }
            // One line on stderr, never mixed with what the program prints: `✓ hello.exe
            // (Windows, 22 KB) in 12 ms` (user, 2026-10-23).
            let system = target.or(codegen::Target::host()).map_or(format!("{} {}", std::env::consts::OS, std::env::consts::ARCH), |t| t.title().to_string());
            let size = std::fs::metadata(&output).map_or(0, |m| m.len()).div_ceil(1024);
            let ms = started.elapsed().as_millis();
            let time = if ms < 1000 { format!("{ms} ms") } else { format!("{}{}{} s", ms / 1000, t!("decimal_mark"), ms % 1000 / 100) };
            eprintln!("{}", t!("built", file = output, system = system, size = size, time = time));
        }
        Command::Test { only } => exit(interp::test(&program, &files, only.as_deref())),
    }
}

/// A `mode build` function without `else` (PLAN 2.14): the program has parts that want native code.
fn discreet(tree: &ast::File) -> bool {
    let methods = tree.structs.iter().flat_map(|s| &s.methods);
    (tree.functions.iter().chain(methods)).any(|f| matches!(&f.only, Some(ast::OnlyIn { build: true, otherwise: ast::Otherwise::Discreet, .. })))
}

/// `kova run` of a program with a discreet `mode build` function: the whole program, translated
/// for `kova run` (its `mode = run` branches, its `else`s), runs as native code, which prints the
/// same as the interpreter (PLAN Part 4 "Run or build, discreet functions"). `None` when it
/// cannot be built or started here: the interpreter runs it.
fn native_run(program: &kir::Program, files: &[String], lang: Option<usize>, given: &[String], named: Option<String>) -> Option<i32> {
    let target = codegen::Target::native()?;
    // `kova-run-123/counter`: the program keeps its `header` name or its file's name, which the Dock, the taskbar and
    // the process list show.
    let temp = std::env::temp_dir();
    let dir = temp.join(format!("kova-run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let name = match named {
        Some(n) => n,
        None => std::path::Path::new(files.first()?).file_stem()?.to_string_lossy().into_owned(),
    };
    let exe = dir.join(format!("{name}{}", target.suffix()));
    codegen::build(program, files, exe.to_str()?, lang, Some(target)).ok()?;
    let mut command = std::process::Command::new(&exe);
    command.args(given);
    // Linux, macOS: `kova` becomes the program (same process: stopping one stops the other). Its
    // file stays until the next such run removes it (a running program keeps its file).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let old = |name: &str| name.starts_with("kova-run-") && !dir.ends_with(name);
        for e in std::fs::read_dir(&temp).into_iter().flatten().flatten() {
            let a_while = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|d| d.as_secs() > 10));
            if old(&e.file_name().to_string_lossy()) && a_while {
                // Folders since 2026-10-29, single files before.
                let _ = std::fs::remove_dir_all(e.path()).or_else(|_| std::fs::remove_file(e.path()));
            }
        }
        let _ = command.exec(); // returns only if it could not start
        let _ = std::fs::remove_dir_all(&dir);
        None
    }
    // Windows: the program is in a job that ends with `kova`, so stopping `kova` stops it too.
    #[cfg(windows)]
    {
        let child = command.spawn();
        let status = child.ok().map(|mut c| {
            windows_job::tie(&c);
            c.wait()
        });
        let _ = std::fs::remove_dir_all(&dir);
        status?.ok()?.code()
    }
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, size: u32) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    }

    /// JOBOBJECT_EXTENDED_LIMIT_INFORMATION.
    #[repr(C)]
    #[derive(Default)]
    struct Limits {
        process_time: i64,
        job_time: i64,
        flags: u32,
        smallest_set: usize,
        largest_set: usize,
        processes: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }

    /// The job's handle is never closed: Windows closes it when `kova` ends, which ends the child.
    pub fn tie(child: &std::process::Child) {
        const EXTENDED_LIMITS: i32 = 9;
        const KILL_ON_JOB_CLOSE: u32 = 0x2000;
        let limits = Limits { flags: KILL_ON_JOB_CLOSE, ..Default::default() };
        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if !job.is_null() {
                SetInformationJobObject(job, EXTENDED_LIMITS, (&raw const limits).cast(), size_of::<Limits>() as u32);
                AssignProcessToJobObject(job, child.as_raw_handle());
            }
        }
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
/// The folder `import shapes.circle` starts from: the file's own, or the nearest folder above it
/// where every file it imports exists (`shapes/circle.kova` opened alone, importing `shapes.point`).
pub fn program_folder(path: &str, imports: &[ast::Import]) -> std::path::PathBuf {
    let own = std::path::Path::new(path).parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let files: Vec<String> = (imports.iter().filter(|i| i.package.is_none()).map(|i| &i.module.text))
        .filter(|m| !types::MODULES.contains(&m.as_str()))
        .map(|m| format!("{}.kova", m.replace('.', std::path::MAIN_SEPARATOR_STR)))
        .collect();
    let finds = |d: &std::path::Path| files.iter().all(|f| d.join(f).is_file());
    if files.is_empty() || finds(&own) {
        return own;
    }
    let here = if own.as_os_str().is_empty() { std::path::Path::new(".") } else { &own };
    let above = here.canonicalize().ok().and_then(|o| o.ancestors().skip(1).find(|d| finds(d)).map(|d| d.to_path_buf()));
    above.unwrap_or(own)
}

/// `header { icon := "counter.svg" }`: the image, from the program's folder (src/icon.rs), or
/// the error to show on the setting.
fn icon(path: &str, tree: &ast::File) -> Option<Result<icon::Icon, diag::Diag>> {
    let (written, span) = types::program_icon(tree)?;
    // Not a known format: the type checker says what to write.
    if !types::ICON_FORMATS.iter().any(|f| written.to_ascii_lowercase().ends_with(&format!(".{f}"))) {
        return None;
    }
    let read = std::fs::read(std::path::Path::new(path).with_file_name(&written)).map_err(|e| t!("icon_unreadable", file = written, error = e));
    Some(read.and_then(|bytes| icon::read(&written, bytes)).map_err(|msg| diag::Diag::new(span, msg)))
}

fn load(path: &str, source: &str) -> (ast::File, usize, Vec<diag::Diag>, diag::Sources) {
    let mut sources = diag::Sources::new();
    sources.add(path, source);
    let (tokens, mut diags) = lexer::lex(source);
    let (mut root, mut ids, d) = parser::parse(tokens, source);
    diags.extend(d);
    if let Some(Err(d)) = icon(path, &root) {
        diags.push(d);
    }
    let dir = program_folder(path, &root.imports);
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
        // Where `kova.lock` is: the root program's folder.
        root_dir: &std::path::Path,
        sources: &mut diag::Sources,
        ids: &mut usize,
        diags: &mut Vec<diag::Diag>,
        loaded: &mut Vec<ast::File>,
        // A package's name → (address, version, its folder).
        packs: &mut std::collections::HashMap<String, (String, String, std::path::PathBuf)>,
    ) {
        let mut aliases: std::collections::HashMap<&str, diag::Span> = std::collections::HashMap::new();
        for imp in imports {
            let m = &imp.module.text;
            if let Some(first) = aliases.insert(&imp.alias.text, imp.alias.span) {
                let (line, _) = diag::line_col(&sources.text, first.start);
                diags.push(diag::Diag::new(imp.alias.span, t!("import_twice", name = imp.alias.text, line = line)));
                continue;
            }
            // `import web`: Kova's own module, even from a file named `web.kova`.
            if types::MODULES.contains(&m.as_str()) {
                continue;
            }
            if let Some((address, version)) = &imp.package {
                match packs.get(m) {
                    Some((a, v, _)) if (a, v) == (&address.text, &version.text) => continue,
                    Some((a, v, _)) => {
                        let msg = t!("package_two_versions", name = m, first = format!("{a} {v}"), second = format!("{} {}", address.text, version.text));
                        diags.push(diag::Diag::new(address.span, msg));
                        continue;
                    }
                    None if done.contains(m) => {
                        diags.push(diag::Diag::new(address.span, t!("package_name_taken", name = m)));
                        continue;
                    }
                    None => {}
                }
                let download = packages::DOWNLOAD.load(std::sync::atomic::Ordering::Relaxed);
                match packages::locate(&address.text, &version.text, root_dir, download) {
                    Ok(d) => {
                        packs.insert(m.clone(), (address.text.clone(), version.text.clone(), d));
                    }
                    Err(msg) => {
                        diags.push(diag::Diag::new(address.span, msg));
                        continue;
                    }
                }
            } else if packs.contains_key(m) {
                diags.push(diag::Diag::new(imp.module.span, t!("package_name_taken", name = m)));
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
            // Written with the separator the importing file's path uses (`/`, or Windows' `\`),
            // so that run and build show the same path everywhere.
            // A package's files are in its folder: `kova_json` = `package.kova`, `kova_json.util` =
            // `util.kova` there.
            let (first, rest) = m.split_once('.').unwrap_or((m, ""));
            let pack = packs.get(first).map(|(.., d)| d.clone());
            let here = pack.as_deref().unwrap_or(dir);
            let dir_text = here.to_string_lossy();
            let sep = dir_text.chars().rev().find(|c| matches!(c, '/' | '\\')).unwrap_or('/');
            // `shapes.circle` = `shapes/circle.kova`, from the program's folder.
            let path = match &pack {
                Some(_) if rest.is_empty() => packages::ENTRY.trim_end_matches(".kova").to_string(),
                Some(_) => rest.replace('.', &sep.to_string()),
                None => m.replace('.', &sep.to_string()),
            };
            let shown = if dir_text.is_empty() { format!("{path}.kova") } else { format!("{dir_text}{sep}{path}.kova") };
            let file = std::path::PathBuf::from(&shown);
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
            let package = pack.is_some().then_some(first);
            let (tree, next, d) = parser::parse_in(tokens, &sources.text, *ids, Some(m), package);
            *ids = next;
            diags.extend(d);
            for l in &tree.langs {
                diags.push(diag::Diag::new(l.name.span, t!("import_lang_later")));
            }
            chain.push(m.clone());
            visit(&tree.imports, Some(origin), chain, done, dir, root_dir, sources, ids, diags, loaded, packs);
            chain.pop();
            loaded.push(tree);
        }
    }
    let mut chain = vec![stem];
    let imports = std::mem::take(&mut root.imports);
    let root_dir = std::path::Path::new(path).parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let mut packs = std::collections::HashMap::new();
    visit(&imports, None, &mut chain, &mut done, &dir, &root_dir, &mut sources, &mut ids, &mut diags, &mut loaded, &mut packs);
    root.imports = imports;
    // With the file each one is read from, as messages name it: a package's under its address.
    root.modules = (done.into_iter())
        .map(|m| {
            let (first, rest) = m.split_once('.').unwrap_or((&m, ""));
            let file = match packs.get(first) {
                Some((address, ..)) if rest.is_empty() => format!("{address}/{}", packages::ENTRY),
                Some((address, ..)) => format!("{address}/{}.kova", rest.replace('.', "/")),
                None => format!("{}.kova", m.replace('.', "/")),
            };
            (m, file)
        })
        .collect();
    // `import ui`: its module, made for this program (src/ui.rs).
    let files: Vec<&ast::File> = std::iter::once(&root).chain(&loaded).collect();
    // After a syntax error, nothing is checked further: the module would only add errors.
    match ui::module(&files).filter(|_| diags.is_empty()) {
        Some(Ok(text)) => {
            let base = sources.add("ui", &text);
            root.ui_from = Some(base);
            let (mut tokens, d) = lexer::lex(&text);
            tokens.iter_mut().for_each(|t| t.span = diag::Span::new(t.span.start + base, t.span.end + base));
            diags.extend(d);
            let (tree, next, d) = parser::parse_in(tokens, &sources.text, ids, Some("ui"), None);
            ids = next;
            diags.extend(d);
            loaded.push(tree);
        }
        Some(Err(d)) => diags.extend(d),
        None => {}
    }
    for m in loaded {
        root.functions.extend(m.functions);
        root.structs.extend(m.structs);
        root.interfaces.extend(m.interfaces);
        root.private.extend(m.private);
        root.externs.extend(m.externs);
        root.ui_runs.extend(m.ui_runs);
    }
    (root, ids, diags, sources)
}

/// Reads `-o output`, `--boost` and `--target system`, in any order.
fn build_options(options: &[&str]) -> Option<(Option<String>, bool, Option<codegen::Target>)> {
    let (mut output, mut boost, mut target) = (None, false, None);
    let mut i = 0;
    while i < options.len() {
        match options[i] {
            "--boost" => boost = true,
            "-o" => {
                output = Some(options.get(i + 1)?.to_string());
                i += 1;
            }
            "--target" => {
                let name = options.get(i + 1)?;
                let names: Vec<&str> = codegen::Target::NAMES.iter().map(|(n, _)| *n).collect();
                target = Some(codegen::Target::named(name).unwrap_or_else(|| fail(t!("unknown_target", name = name, names = names.join(", ")))));
                i += 1;
            }
            _ => return None,
        }
        i += 1;
    }
    Some((output, boost, target))
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
