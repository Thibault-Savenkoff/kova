// Point d'entrée : lit la ligne de commande et lance la bonne action.
// Voir docs/PLAN.md, Partie 1 (commandes) et 2.11 (header).

use std::process::exit;

const USAGE: &str = "\
usage :
  kova <fichier.kova>                  lance selon le header du fichier (run par défaut)
  kova run <fichier.kova>              lance tout de suite
  kova build <fichier.kova> [-o sortie] [--boost]
                                       crée un exécutable autonome
  kova test <fichier.kova>             lance les blocs `test`
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

    let _ = (source, commande);
    eprintln!("erreur: pas encore implémenté (jalon J1)");
    exit(1);
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
