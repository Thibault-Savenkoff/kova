// Messages d'erreur : une position dans le texte + un message.
// Le compilateur renvoie une liste de `Diag` au lieu de s'arrêter :
// `kova run` les affiche, `kova lsp` les enverra à l'éditeur.

/// Un morceau du texte source, en octets : `source[debut..fin]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub debut: usize,
    pub fin: usize,
}

impl Span {
    pub fn new(debut: usize, fin: usize) -> Span {
        Span { debut, fin }
    }
    /// Du début de `self` à la fin de `autre`.
    pub fn jusqua(self, autre: Span) -> Span {
        Span::new(self.debut, autre.fin)
    }
}

#[derive(Debug)]
pub struct Diag {
    pub message: String,
    pub span: Span,
    pub aide: Option<String>,
}

impl Diag {
    pub fn new(span: Span, message: impl Into<String>) -> Diag {
        Diag { message: message.into(), span, aide: None }
    }
    pub fn aide(mut self, aide: impl Into<String>) -> Diag {
        self.aide = Some(aide.into());
        self
    }
}

/// Ligne et colonne (à partir de 1, colonne en caractères) d'une position en octets.
pub fn ligne_colonne(source: &str, pos: usize) -> (usize, usize) {
    let avant = &source[..pos.min(source.len())];
    let ligne = avant.matches('\n').count() + 1;
    let debut_ligne = avant.rfind('\n').map_or(0, |i| i + 1);
    (ligne, avant[debut_ligne..].chars().count() + 1)
}

/// Affiche une erreur à la façon de rustc :
///
/// ```text
/// erreur: `a` est déclarée avec let
///   --> main.kova:3:5
///    |
///  3 |     a := 2
///    |     ^
///    = aide : ...
/// ```
pub fn afficher(d: &Diag, fichier: &str, source: &str) -> String {
    let (ligne, col) = ligne_colonne(source, d.span.debut);
    let texte_ligne = source.lines().nth(ligne - 1).unwrap_or("");
    // Soulignement : la longueur du span, coupée en fin de ligne, au moins 1.
    let reste = texte_ligne.chars().count().saturating_sub(col - 1);
    let largeur = source
        .get(d.span.debut..d.span.fin)
        .map_or(1, |s| s.chars().count())
        .clamp(1, reste.max(1));
    let marge = " ".repeat(ligne.to_string().len());

    let mut s = format!("erreur: {}\n{marge}--> {fichier}:{ligne}:{col}\n", d.message);
    s += &format!("{marge} |\n{ligne} | {texte_ligne}\n");
    s += &format!("{marge} | {}{}\n", " ".repeat(col - 1), "^".repeat(largeur));
    if let Some(aide) = &d.aide {
        s += &format!("{marge} = aide : {aide}\n");
    }
    s
}
