// Entry point: reads the command line and runs the right action.
// See docs/PLAN.md, Part 1 (commands) and 2.11 (header).

mod ast;
mod codegen;
mod diag;
mod interp;
mod kir;
mod lexer;
mod lsp;
mod parser;
mod types;

use std::process::exit;

const USAGE: &str = "\
usage :
  kova <fichier.kova>                  lance selon le header du fichier (run par défaut)
  kova run <fichier.kova>              lance tout de suite
  kova build <fichier.kova> [-o sortie] [--boost]
                                       crée un exécutable autonome
  kova test <fichier.kova>             lance les blocs `test`
  kova lsp                             serveur de langage (pour l'éditeur)
  kova --version";

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
                eprintln!("erreur: kova lsp : {e}");
                exit(1);
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
        Err(e) => {
            eprintln!("erreur: impossible de lire `{file}` : {e}");
            exit(1);
        }
    };

    let (tree, info, diags) = analyze(&source);
    let Some(info) = info else {
        for d in &diags {
            eprintln!("{}", diag::render(d, file, &source));
        }
        exit(1);
    };

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
            eprintln!("erreur: ce programme est réservé à `kova {mode}` (header ligne {line})");
            exit(1);
        }
    }

    let contracts = !matches!(command, Command::Build { boost: true, .. });
    let program = kir::translate(&tree, &info, &source, contracts, matches!(command, Command::Test));

    match command {
        Command::Run => exit(interp::run(&program, file)),
        Command::Build { output, .. } => {
            let output = output.unwrap_or_else(|| file.trim_end_matches(".kova").to_string());
            if output == file {
                eprintln!("erreur: l'exécutable écraserait `{file}` ; choisis un autre nom avec `-o`");
                exit(1);
            }
            if let Err(e) = codegen::build(&program, file, &output) {
                eprintln!("erreur: {e}");
                exit(1);
            }
        }
        Command::Test => exit(interp::test(&program, file)),
    }
}

/// Lexer + parser + type checker. Reads in-memory text: `kova lsp` calls it on
/// text that isn't saved yet. `Some(info)` only if there is no error at all.
fn analyze(source: &str) -> (ast::File, Option<types::Info>, Vec<diag::Diag>) {
    let (tokens, mut diags) = lexer::lex(source);
    let (file, nb_exprs, d) = parser::parse(tokens, source);
    diags.extend(d);
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
    eprintln!("{USAGE}");
    exit(2);
}
