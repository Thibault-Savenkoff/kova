// AST : l'arbre du programme, tel qu'écrit. Chaque nœud garde sa position (`Span`).
//
//   total := total + i   →   Affecter { nom: "total", valeur: Binaire(Plus, Var(total), Var(i)) }

use crate::diag::Span;

pub struct Fichier {
    pub fonctions: Vec<Fonction>,
}

pub struct Fonction {
    pub nom: Nom,
    pub params: Vec<Param>,
    pub retour: Option<Nom>,
    pub corps: Bloc,
}

/// Un nom et sa position (variable, fonction ou type).
#[derive(Clone)]
pub struct Nom {
    pub texte: String,
    pub span: Span,
}

pub struct Param {
    pub nom: Nom,
    pub ty: Nom,
}

pub struct Bloc {
    /// Position de la `{` ouvrante.
    pub debut: Span,
    pub instrs: Vec<Instr>,
    /// Position de la `}` fermante (pour « il manque un return »).
    pub fin: Span,
}

pub enum Instr {
    /// `let x: Int := 5` (`var` si `modifiable`).
    Declarer { modifiable: bool, nom: Nom, ty: Option<Nom>, valeur: Expr },
    /// `x := 5`, ou `x += 5` (`op` = `Some(Plus)`).
    Affecter { nom: Nom, op: Option<OpBin>, valeur: Expr },
    Retour { valeur: Option<Expr>, span: Span },
    Sortir(Span),
    Continuer(Span),
    Si { cond: Expr, alors: Bloc, sinon: Option<Bloc> },
    TantQue { cond: Expr, corps: Bloc },
    /// `for i in a..b` (`inclus` = false) et `for i in a..=b` / `from i := a to b` (`inclus` = true).
    Boucle { compteur: Nom, debut: Expr, fin: Expr, inclus: bool, corps: Bloc },
    Expr(Expr),
}

pub struct Expr {
    /// Numéro unique : sert d'index dans la table des types (`types::Infos`).
    pub id: usize,
    pub span: Span,
    pub genre: ExprGenre,
}

pub enum ExprGenre {
    Entier(u64),
    Bool(bool),
    Var(String),
    Appel { nom: Nom, args: Vec<Expr> },
    Unaire(OpUn, Box<Expr>),
    Binaire(OpBin, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OpUn {
    Moins,
    Non,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OpBin {
    Ou,
    Et,
    Egal,
    Different,
    Inf,
    InfEgal,
    Sup,
    SupEgal,
    Intervalle,
    IntervalleInclus,
    Plus,
    Moins,
    Fois,
    Divise,
    Reste,
}

impl OpBin {
    pub fn texte(self) -> &'static str {
        use OpBin::*;
        match self {
            Ou => "||",
            Et => "&&",
            Egal => "=",
            Different => "!=",
            Inf => "<",
            InfEgal => "<=",
            Sup => ">",
            SupEgal => ">=",
            Intervalle => "..",
            IntervalleInclus => "..=",
            Plus => "+",
            Moins => "-",
            Fois => "*",
            Divise => "/",
            Reste => "%",
        }
    }
}
