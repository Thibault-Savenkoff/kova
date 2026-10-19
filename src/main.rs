// Point d'entrée : lit la ligne de commande et lance la bonne action.
// Voir docs/PLAN.md, Partie 1 (commandes) et 2.11 (header).

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

enum Commande {
    Run,
    Build { sortie: Option<String>, boost: bool },
    Test,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let (commande, fichier) = match args.as_slice() {
        ["--version" | "-V"] => {
            println!("kova {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        ["lsp"] => {
            if let Err(e) = lsp::lancer() {
                eprintln!("erreur: kova lsp : {e}");
                exit(1);
            }
            return;
        }
        ["run", f] => (Some(Commande::Run), *f),
        ["test", f] => (Some(Commande::Test), *f),
        ["build", f, options @ ..] => match options_build(options) {
            Some((sortie, boost)) => (Some(Commande::Build { sortie, boost }), *f),
            None => erreur_usage(),
        },
        // Sans commande : le `header` du fichier choisit (run par défaut).
        [f] if f.ends_with(".kova") => (None, *f),
        _ => erreur_usage(),
    };

    let source = match std::fs::read_to_string(fichier) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erreur: impossible de lire `{fichier}` : {e}");
            exit(1);
        }
    };

    let (arbre, infos, diags) = analyser(&source);
    let Some(infos) = infos else {
        for d in &diags {
            eprintln!("{}", diag::afficher(d, fichier, &source));
        }
        exit(1);
    };

    // Le header : `mode` choisit la commande par défaut, `lock := true` refuse l'autre.
    let mode = types::reglage(&arbre, &source, "mode").map_or("run", |(v, _)| v);
    let commande = commande.unwrap_or(if mode == "build" {
        Commande::Build { sortie: None, boost: false }
    } else {
        Commande::Run
    });
    if let Some(("true", ligne)) = types::reglage(&arbre, &source, "lock") {
        let autre = matches!((&commande, mode), (Commande::Run, "build") | (Commande::Build { .. }, "run"));
        if autre {
            eprintln!("erreur: ce programme est réservé à `kova {mode}` (header ligne {ligne})");
            exit(1);
        }
    }

    let contrats = !matches!(commande, Commande::Build { boost: true, .. });
    let programme = kir::traduire(&arbre, &infos, &source, contrats, matches!(commande, Commande::Test));

    match commande {
        Commande::Run => exit(interp::executer(&programme, fichier)),
        Commande::Build { sortie, .. } => {
            let sortie = sortie.unwrap_or_else(|| fichier.trim_end_matches(".kova").to_string());
            if sortie == fichier {
                eprintln!("erreur: l'exécutable écraserait `{fichier}` ; choisis un autre nom avec `-o`");
                exit(1);
            }
            if let Err(e) = codegen::construire(&programme, fichier, &sortie) {
                eprintln!("erreur: {e}");
                exit(1);
            }
        }
        Commande::Test => exit(interp::tester(&programme, fichier)),
    }
}

/// Lexer + parser + typage. Lit du texte en mémoire : `kova lsp` l'appelle sur le
/// texte pas encore sauvegardé. `Some(infos)` seulement s'il n'y a aucune erreur.
fn analyser(source: &str) -> (ast::Fichier, Option<types::Infos>, Vec<diag::Diag>) {
    let (jetons, mut diags) = lexer::lexer(source);
    let (fichier, nb_exprs, d) = parser::parser(jetons, source);
    diags.extend(d);
    // Erreurs de syntaxe : on s'arrête là, sinon le typage en inventerait d'autres
    // (une variable dont le `let` est illisible serait « inconnue » partout).
    if diags.is_empty() {
        let (infos, d) = types::verifier(&fichier, nb_exprs, source);
        if d.is_empty() {
            return (fichier, Some(infos), d);
        }
        diags = d;
    }
    diags.sort_by_key(|d| d.span.debut);
    (fichier, None, diags)
}

/// Lit `-o sortie` et `--boost`, dans n'importe quel ordre.
fn options_build(options: &[&str]) -> Option<(Option<String>, bool)> {
    let (mut sortie, mut boost) = (None, false);
    let mut i = 0;
    while i < options.len() {
        match options[i] {
            "--boost" => boost = true,
            "-o" => {
                sortie = Some(options.get(i + 1)?.to_string());
                i += 1;
            }
            _ => return None,
        }
        i += 1;
    }
    Some((sortie, boost))
}

fn erreur_usage() -> ! {
    eprintln!("{USAGE}");
    exit(2);
}
