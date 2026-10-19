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
        ["run", f] => (Commande::Run, *f),
        ["test", f] => (Commande::Test, *f),
        ["build", f, options @ ..] => match options_build(options) {
            Some((sortie, boost)) => (Commande::Build { sortie, boost }, *f),
            None => erreur_usage(),
        },
        // ponytail: le choix run/build via `header` arrive en J2 ; d'ici là, run.
        [f] if f.ends_with(".kova") => (Commande::Run, *f),
        _ => erreur_usage(),
    };

    let source = match std::fs::read_to_string(fichier) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("erreur: impossible de lire `{fichier}` : {e}");
            exit(1);
        }
    };

    let contrats = !matches!(commande, Commande::Build { boost: true, .. });
    let programme = match compiler(&source, contrats) {
        Ok(p) => p,
        Err(diags) => {
            for d in &diags {
                eprintln!("{}", diag::afficher(d, fichier, &source));
            }
            exit(1);
        }
    };

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
        Commande::Test => {
            eprintln!("erreur: `kova test` arrive au jalon J2");
            exit(1);
        }
    }
}

/// Texte source → KIR, ou la liste de toutes les erreurs.
/// `contrats` = false : `require`, `ensure` et `check` sont retirés (`--boost`).
fn compiler(source: &str, contrats: bool) -> Result<kir::Programme, Vec<diag::Diag>> {
    match analyser(source) {
        (fichier, Some(infos), _) => Ok(kir::traduire(&fichier, &infos, source, contrats)),
        (_, None, diags) => Err(diags),
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
