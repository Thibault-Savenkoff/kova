// Lexer : découpe le texte en jetons (mots, nombres, symboles).
//
//   "let x: Int := 5"  →  [Cle(let)] [Ident] [Sym(:)] [Type] [Sym(:=)] [Entier(5)]
//
// Il ajoute aussi les fins de ligne (`Ligne`) qui terminent les instructions,
// en suivant les règles de docs/PLAN.md 2.1 (« Fins de ligne »).

use crate::diag::{Diag, Span};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Genre {
    /// Nom en minuscule : variable, fonction (`total`, `carré`).
    Ident,
    /// Nom en majuscule : type (`Int`, `Chien`).
    Type,
    Entier(u64),
    /// Mot-clé (`fn`, `let`, …).
    Cle(&'static str),
    /// Symbole (`:=`, `+`, `{`, …).
    Sym(&'static str),
    /// Fin d'instruction.
    Ligne,
    /// Fin du fichier.
    Fin,
}

#[derive(Clone, Copy, Debug)]
pub struct Jeton {
    pub genre: Genre,
    pub span: Span,
}

const MOTS_CLES: &[&str] = &[
    "fn", "let", "var", "move", "return", "if", "else", "while", "for", "in", "from", "to", "match",
    "struct", "enum", "interface", "import", "public", "private", "require", "ensure", "check",
    "result", "self", "true", "false", "none", "ok", "err", "and", "or", "copy", "any", "break",
    "continue", "header", "test",
];

/// Du plus long au plus court, pour que `..=` gagne sur `..` puis `.`.
const SYMBOLES: &[&str] = &[
    "..=", ":=", "+=", "-=", "*=", "/=", "->", "=>", "!=", "<=", ">=", "&&", "||", "..", "+", "-", "*",
    "/", "%", "=", "<", ">", "!", "(", ")", "[", "]", "{", "}", ",", ":", ".", "?",
];

/// Une ligne qui finit par un de ces jetons continue sur la suivante.
fn continue_apres(g: Genre) -> bool {
    matches!(g, Genre::Sym(s) if !matches!(s, ")" | "]" | "}" | "?" | "!"))
}

/// Une ligne qui commence par un de ces jetons continue la précédente.
fn continue_avant(g: Genre) -> bool {
    matches!(g, Genre::Sym("." | "&&" | "||" | ")" | "]" | "}"))
}

pub fn lexer(source: &str) -> (Vec<Jeton>, Vec<Diag>) {
    let (bruts, diags) = jetons_bruts(source);
    (ajouter_lignes(bruts), diags)
}

/// Les jetons sans les `Ligne`, chacun avec « y a-t-il un retour à la ligne juste avant ? ».
fn jetons_bruts(source: &str) -> (Vec<(Jeton, bool)>, Vec<Diag>) {
    let mut jetons = vec![];
    let mut diags = vec![];
    let mut nouvelle_ligne = false;
    let mut i = 0;

    while let Some(c) = source[i..].chars().next() {
        let debut = i;
        let reste = &source[i..];

        if c == '\n' {
            nouvelle_ligne = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if reste.starts_with("//") {
            i += reste.find('\n').unwrap_or(reste.len());
            continue;
        }
        if reste.starts_with("/*") {
            match fin_commentaire(reste) {
                Some(n) => {
                    nouvelle_ligne |= reste[..n].contains('\n');
                    i += n;
                }
                None => {
                    diags.push(Diag::new(Span::new(i, i + 2), "commentaire `/*` jamais fermé"));
                    i = source.len();
                }
            }
            continue;
        }

        let genre = if c.is_alphabetic() || c == '_' {
            let n = reste.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(reste.len());
            let mot = &reste[..n];
            i += n;
            if let Some(cle) = MOTS_CLES.iter().find(|&&k| k == mot) {
                Genre::Cle(cle)
            } else if c.is_uppercase() {
                Genre::Type
            } else {
                Genre::Ident
            }
        } else if c.is_ascii_digit() {
            let n = reste.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(reste.len());
            i += n;
            match lire_entier(&reste[..n]) {
                Ok(v) => Genre::Entier(v),
                Err(msg) => {
                    diags.push(Diag::new(Span::new(debut, i), msg));
                    Genre::Entier(0)
                }
            }
        } else if let Some(s) = SYMBOLES.iter().find(|s| reste.starts_with(**s)) {
            i += s.len();
            Genre::Sym(s)
        } else {
            i += c.len_utf8();
            let msg = if c == '"' || c == '\'' {
                "les textes et les caractères arrivent au jalon J3".to_string()
            } else {
                format!("caractère inattendu `{c}`")
            };
            diags.push(Diag::new(Span::new(debut, i), msg));
            continue;
        };

        jetons.push((Jeton { genre, span: Span::new(debut, i) }, nouvelle_ligne));
        nouvelle_ligne = false;
    }
    jetons.push((Jeton { genre: Genre::Fin, span: Span::new(source.len(), source.len()) }, true));
    (jetons, diags)
}

/// Longueur d'un commentaire `/* ... */` (imbricable) au début de `s`.
fn fin_commentaire(s: &str) -> Option<usize> {
    let mut profondeur = 0;
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with("/*") {
            profondeur += 1;
            i += 2;
        } else if s[i..].starts_with("*/") {
            profondeur -= 1;
            i += 2;
            if profondeur == 0 {
                return Some(i);
            }
        } else {
            i += s[i..].chars().next().unwrap().len_utf8();
        }
    }
    None
}

/// `42`, `1_000_000`, `0xFF`, `0b1010`.
fn lire_entier(texte: &str) -> Result<u64, String> {
    let (chiffres, base) = if let Some(h) = texte.strip_prefix("0x") {
        (h, 16)
    } else if let Some(b) = texte.strip_prefix("0b") {
        (b, 2)
    } else {
        (texte, 10)
    };
    let chiffres = chiffres.replace('_', "");
    if chiffres.is_empty() || !chiffres.chars().all(|c| c.is_digit(base)) {
        return Err(format!("nombre mal écrit `{texte}`"));
    }
    u64::from_str_radix(&chiffres, base).map_err(|_| format!("nombre trop grand `{texte}`"))
}

/// Ajoute les `Ligne` qui terminent les instructions (PLAN 2.1, règles 1 à 3).
fn ajouter_lignes(bruts: Vec<(Jeton, bool)>) -> Vec<Jeton> {
    let mut sortie: Vec<Jeton> = vec![];
    // Parenthèses ouvertes : dans `(` et `[`, les retours à la ligne ne comptent pas.
    let mut ouverts: Vec<&str> = vec![];

    for (j, nouvelle_ligne) in bruts {
        let dans_accolades = matches!(ouverts.last(), None | Some(&"{"));
        if let Some(prec) = sortie.last()
            && nouvelle_ligne
            && dans_accolades
            && prec.genre != Genre::Ligne
            && !continue_apres(prec.genre)
            && !continue_avant(j.genre)
        {
            sortie.push(Jeton { genre: Genre::Ligne, span: Span::new(prec.span.fin, prec.span.fin) });
        }
        match j.genre {
            Genre::Sym(s @ ("(" | "[" | "{")) => ouverts.push(s),
            Genre::Sym(")" | "]" | "}") => {
                ouverts.pop();
            }
            _ => {}
        }
        sortie.push(j);
    }
    sortie
}
