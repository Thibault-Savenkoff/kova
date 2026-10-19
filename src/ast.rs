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
    pub retour: Option<TypeAst>,
    pub corps: Bloc,
}

/// Un type tel qu'écrit : `Int`, `(Int, Float)`, `((Int, Int), Bool)`.
pub enum TypeAst {
    Nom(Nom),
    Tuple(Vec<TypeAst>),
}

impl TypeAst {
    /// Le type comme dans le code : `(Int, Float)`.
    pub fn texte(&self) -> String {
        match self {
            TypeAst::Nom(n) => n.texte.clone(),
            TypeAst::Tuple(ts) => format!("({})", ts.iter().map(TypeAst::texte).collect::<Vec<_>>().join(", ")),
        }
    }
}

/// Ce qui reçoit une valeur dans un `let` : `x`, `(a, b)`, `(a, _)`.
pub enum Motif {
    Nom(Nom),
    Tuple(Vec<Motif>, Span),
    /// `_` : la valeur est ignorée.
    Ignorer(Span),
}

/// Ce qui reçoit une valeur dans `:=` : `x`, `t.0`, `(a, b)`.
pub enum Cible {
    /// Une variable, suivie de numéros de champ : `t.1.0` = `nom: t, champs: [1, 0]`.
    Lieu { nom: Nom, champs: Vec<usize>, span: Span },
    Tuple(Vec<Cible>, Span),
}

impl Cible {
    pub fn span(&self) -> Span {
        match self {
            Cible::Lieu { span, .. } | Cible::Tuple(_, span) => *span,
        }
    }
}

/// Un nom et sa position (variable, fonction ou type).
#[derive(Clone)]
pub struct Nom {
    pub texte: String,
    pub span: Span,
}

pub struct Param {
    pub nom: Nom,
    pub ty: TypeAst,
}

pub struct Bloc {
    /// Position de la `{` ouvrante.
    pub debut: Span,
    pub instrs: Vec<Instr>,
    /// Position de la `}` fermante (pour « il manque un return »).
    pub fin: Span,
}

pub enum Instr {
    /// `let x: Int := 5`, `var (a, b): (Int, Int) := f()` (`var` si `modifiable`).
    Declarer { modifiable: bool, motif: Motif, ty: Option<TypeAst>, valeur: Expr },
    /// `x := 5`, `(a, b) := (b, a)`, ou `x += 5` (`op` = `Some(Plus)`).
    Affecter { cible: Cible, op: Option<OpBin>, valeur: Expr },
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
    Flottant(f64),
    Caractere(char),
    Bool(bool),
    Var(String),
    Appel { nom: Nom, args: Vec<Expr> },
    /// `x.to_float()` : méthode prédéfinie (J2 : seulement les conversions).
    Methode { recu: Box<Expr>, nom: Nom, args: Vec<Expr> },
    Unaire(OpUn, Box<Expr>),
    Binaire(OpBin, Box<Expr>, Box<Expr>),
    /// `(1, 2.5)`
    Tuple(Vec<Expr>),
    /// `t.0`
    Champ(Box<Expr>, usize),
    /// `if c { 1 } else { 2 }` qui donne une valeur : la dernière ligne de la branche choisie.
    Si { cond: Box<Expr>, alors: Bloc, sinon: Bloc },
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
