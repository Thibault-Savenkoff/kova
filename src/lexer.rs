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
    Flottant(f64),
    Caractere(char),
    /// `"..."` ; le contenu se relit avec `contenu_texte`.
    Texte,
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
            // Après un `.`, pas de virgule : `t.0.1` = deux numéros de champ, pas `0.1`.
            let apres_point = matches!(jetons.last(), Some((Jeton { genre: Genre::Sym("."), .. }, _)));
            let n = longueur_nombre(reste, !apres_point);
            i += n;
            match lire_nombre(&reste[..n]) {
                Ok(g) => g,
                Err(msg) => {
                    diags.push(Diag::new(Span::new(debut, i), msg));
                    Genre::Entier(0)
                }
            }
        } else if c == '\'' {
            let (n, resultat) = lire_caractere(reste);
            i += n;
            match resultat {
                Ok(c) => Genre::Caractere(c),
                Err(msg) => {
                    diags.push(Diag::new(Span::new(debut, i), msg));
                    Genre::Caractere(' ')
                }
            }
        } else if c == '"' {
            let (n, erreurs) = lire_texte(reste);
            for (k, msg) in erreurs {
                diags.push(Diag::new(Span::new(debut + k, debut + k + 1), msg));
            }
            i += n;
            Genre::Texte
        } else if let Some(s) = SYMBOLES.iter().find(|s| reste.starts_with(**s)) {
            i += s.len();
            Genre::Sym(s)
        } else {
            i += c.len_utf8();
            let msg = format!("caractère inattendu `{c}`");
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

/// Longueur du nombre au début de `s` : `42`, `0xFF`, `3.14`, `1e-9`, `2.5E+3`.
/// Avec `virgule` = false, on s'arrête avant un `.` (numéro de champ de tuple).
fn longueur_nombre(s: &str, virgule: bool) -> usize {
    let o = s.as_bytes();
    let chiffres = |mut i: usize| {
        while i < o.len() && (o[i].is_ascii_alphanumeric() || o[i] == b'_') && !matches!(o[i], b'e' | b'E') {
            i += 1;
        }
        i
    };
    if s.starts_with("0x") || s.starts_with("0b") {
        return s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(s.len());
    }
    let mut i = chiffres(0);
    // `1..10` reste un intervalle : il faut un chiffre juste après le `.`.
    if virgule && i + 1 < o.len() && o[i] == b'.' && o[i + 1].is_ascii_digit() {
        i = chiffres(i + 1);
    }
    if i < o.len() && matches!(o[i], b'e' | b'E') {
        let j = if matches!(o.get(i + 1), Some(b'+' | b'-')) { i + 2 } else { i + 1 };
        if o.get(j).is_some_and(u8::is_ascii_digit) {
            i = chiffres(j);
        }
    }
    // Lettres collées (`12abc`) : on les prend, `lire_nombre` signalera l'erreur.
    i + s[i..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(s.len() - i)
}

fn lire_nombre(texte: &str) -> Result<Genre, String> {
    let decimal = !texte.starts_with("0x") && !texte.starts_with("0b");
    if decimal && texte.contains(['.', 'e', 'E']) {
        let propre = texte.replace('_', "");
        return match propre.parse::<f64>() {
            Ok(v) if v.is_infinite() => Err(format!("nombre trop grand `{texte}`")),
            Ok(v) => Ok(Genre::Flottant(v)),
            Err(_) => Err(format!("nombre mal écrit `{texte}`")),
        };
    }
    lire_entier(texte).map(Genre::Entier)
}

/// `'a'`, `'\n'`, `'\u{1F600}'`. Renvoie la longueur lue et le caractère.
fn lire_caractere(s: &str) -> (usize, Result<char, String>) {
    let fin_ligne = s.find('\n').unwrap_or(s.len());
    let mut it = s[1..fin_ligne].char_indices();
    let resultat = match it.next() {
        None | Some((_, '\'')) => Err("caractère vide : écris par exemple `'a'`".to_string()),
        Some((_, '\\')) => echappement(&mut it),
        Some((_, c)) => Ok(c),
    };
    match it.next() {
        Some((k, '\'')) => (k + 2, resultat),
        _ => {
            let n = s[1..fin_ligne].find('\'').map_or(fin_ligne, |k| k + 2);
            (n, resultat.and(Err("un caractère s'écrit entre `'` : un seul caractère, comme `'a'`".to_string())))
        }
    }
}

/// `"..."` au début de `s` : sa longueur, et les erreurs (position dans `s`, message).
fn lire_texte(s: &str) -> (usize, Vec<(usize, String)>) {
    let mut erreurs = vec![];
    let mut it = s.char_indices().skip(1).peekable();
    while let Some((k, c)) = it.next() {
        match c {
            '"' => return (k + 1, erreurs),
            '\n' => break,
            '\\' => {
                let mut reste = s[k + 1..].char_indices();
                if let Err(msg) = echappement(&mut reste) {
                    erreurs.push((k, msg));
                }
                // Avance d'autant de caractères que l'échappement en a lu.
                let lu = reste.next().map_or(s.len() - k - 1, |(j, _)| j);
                while it.peek().is_some_and(|&(j, _)| j <= k + lu) {
                    it.next();
                }
            }
            // Une seule erreur par texte : `{x}` en donnerait sinon deux.
            '{' | '}' if !erreurs.iter().any(|(_, m)| m.starts_with("l'interpolation")) => {
                erreurs.push((k, "l'interpolation `{…}` arrive au jalon J3 (écris `\\{` pour une accolade)".into()))
            }
            _ => {}
        }
    }
    let n = s.find('\n').unwrap_or(s.len());
    erreurs.push((0, "texte jamais fermé : il manque le `\"` de fin".into()));
    (n, erreurs)
}

/// Le contenu d'un jeton `Texte` (`"a\nb"` → `a`, retour à la ligne, `b`), déjà vérifié par le lexer.
pub fn contenu_texte(jeton: &str) -> String {
    let interieur = jeton.strip_prefix('"').unwrap_or(jeton);
    let interieur = interieur.strip_suffix('"').unwrap_or(interieur);
    let mut texte = String::new();
    let mut it = interieur.char_indices();
    while let Some((_, c)) = it.next() {
        texte.push(if c == '\\' { echappement(&mut it).unwrap_or('?') } else { c });
    }
    texte
}

/// Lit ce qui suit un `\` : `n t r 0 \\ " ' { }` ou `u{1F600}`.
pub fn echappement(it: &mut std::str::CharIndices) -> Result<char, String> {
    Ok(match it.next().map(|(_, c)| c) {
        Some('n') => '\n',
        Some('t') => '\t',
        Some('r') => '\r',
        Some('0') => '\0',
        Some(c @ ('\\' | '"' | '\'' | '{' | '}')) => c,
        Some('u') => {
            let mut hex = String::new();
            if it.next().map(|(_, c)| c) != Some('{') {
                return Err("écris `\\u{1F600}` (code en hexadécimal entre accolades)".into());
            }
            for (_, c) in it.by_ref() {
                if c == '}' {
                    break;
                }
                hex.push(c);
            }
            let code = u32::from_str_radix(&hex, 16).map_err(|_| format!("code `{hex}` mal écrit (hexadécimal attendu)"))?;
            char::from_u32(code).ok_or_else(|| format!("`{hex}` n'est pas un caractère Unicode valide"))?
        }
        Some(c) => return Err(format!("échappement inconnu `\\{c}`")),
        None => return Err("échappement incomplet".into()),
    })
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
