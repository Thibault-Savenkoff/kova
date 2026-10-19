// Vérification : noms connus, types corrects, règles de `let`/`var`.
//
// Kova ne devine (presque) rien : il vérifie que ce qui est écrit est cohérent.
//
//   let x: Int := true    →  erreur: types différents : `Int` attendu, `Bool` donné
//   a := 2  (a en let)    →  erreur: `a` est déclarée avec let
//
// Le résultat : le type de chaque expression (indexé par `Expr::id`).

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diag, Span, ligne_colonne};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Type {
    Int,
    Bool,
    /// Ce que renvoie une fonction sans `->`.
    Rien,
    /// Type inconnu à cause d'une erreur déjà signalée : on ne la répète pas.
    Erreur,
}

impl Type {
    fn nom(self) -> &'static str {
        match self {
            Type::Int => "Int",
            Type::Bool => "Bool",
            Type::Rien => "rien",
            Type::Erreur => "?",
        }
    }
}

/// Les fonctions prédéfinies : `print(x)` passe à la ligne, `write(x)` non.
pub const PREDEFINIES: &[&str] = &["print", "write"];

pub struct Infos {
    /// Type de chaque expression, par `Expr::id`.
    pub types: Vec<Type>,
}

#[derive(Clone, Copy, PartialEq)]
enum Sorte {
    Let,
    Var,
    Param,
    Compteur,
}

struct Variable {
    ty: Type,
    sorte: Sorte,
    span: Span,
}

struct Signature {
    params: Vec<Type>,
    retour: Type,
}

struct Verif<'a> {
    src: &'a str,
    fonctions: HashMap<&'a str, Signature>,
    /// Une table par bloc ouvert, de l'extérieur vers l'intérieur.
    portees: Vec<HashMap<String, Variable>>,
    retour: Type,
    boucles: usize,
    types: Vec<Type>,
    diags: Vec<Diag>,
}

pub fn verifier(f: &Fichier, nb_exprs: usize, src: &str) -> (Infos, Vec<Diag>) {
    let mut v = Verif {
        src,
        fonctions: HashMap::new(),
        portees: vec![],
        retour: Type::Rien,
        boucles: 0,
        types: vec![Type::Erreur; nb_exprs],
        diags: vec![],
    };

    // 1. Les signatures d'abord : on peut appeler une fonction écrite plus bas.
    for fonc in &f.fonctions {
        for t in fonc.params.iter().map(|p| &p.ty).chain(&fonc.retour) {
            if type_de(t) == Type::Erreur {
                v.erreur(t.span, format!("type inconnu `{}`", t.texte));
            }
        }
        let params = fonc.params.iter().map(|p| type_de(&p.ty)).collect();
        let retour = fonc.retour.as_ref().map_or(Type::Rien, type_de);
        let nom = fonc.nom.texte.as_str();
        if PREDEFINIES.contains(&nom) {
            v.erreur(fonc.nom.span, format!("`{nom}` est une fonction prédéfinie"));
        } else if v.fonctions.contains_key(nom) {
            v.erreur(fonc.nom.span, format!("la fonction `{nom}` existe déjà"));
        } else {
            v.fonctions.insert(nom, Signature { params, retour });
        }
    }

    // 2. `fn main()` : obligatoire, sans paramètre ni résultat.
    match f.fonctions.iter().find(|f| f.nom.texte == "main") {
        None => v.erreur(Span::new(0, 0), "il manque la fonction `main`"),
        Some(m) if !m.params.is_empty() || m.retour.is_some() => {
            v.diags.push(Diag::new(m.nom.span, "`main` ne prend pas de paramètre et ne renvoie rien"));
        }
        _ => {}
    }

    // 3. Les corps.
    for fonc in &f.fonctions {
        v.retour = fonc.retour.as_ref().map_or(Type::Rien, type_de);
        let mut params = HashMap::new();
        for p in &fonc.params {
            let ty = type_de(&p.ty);
            if params.contains_key(&p.nom.texte) {
                v.erreur(p.nom.span, format!("le paramètre `{}` existe déjà", p.nom.texte));
            }
            params.insert(p.nom.texte.clone(), Variable { ty, sorte: Sorte::Param, span: p.nom.span });
        }
        v.portees = vec![params];
        v.bloc(&fonc.corps, vec![]);
        if v.retour != Type::Rien && !termine(&fonc.corps) {
            v.diags.push(
                Diag::new(fonc.corps.fin, format!("`{}` peut arriver ici sans `return`", fonc.nom.texte))
                    .aide(format!("la fonction doit renvoyer un `{}` dans tous les cas", v.retour.nom())),
            );
        }
    }

    (Infos { types: v.types }, v.diags)
}

pub fn type_de(t: &Nom) -> Type {
    match t.texte.as_str() {
        "Int" => Type::Int,
        "Bool" => Type::Bool,
        _ => Type::Erreur,
    }
}

/// Le bloc se termine-t-il toujours par un `return` ?
fn termine(b: &Bloc) -> bool {
    b.instrs.iter().any(|i| match i {
        Instr::Retour { .. } => true,
        Instr::Si { alors, sinon: Some(sinon), .. } => termine(alors) && termine(sinon),
        _ => false,
    })
}

impl Verif<'_> {
    fn erreur(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    fn type_ecrit(&mut self, t: &Nom) -> Type {
        let ty = type_de(t);
        if ty == Type::Erreur {
            self.erreur(t.span, format!("type inconnu `{}`", t.texte));
        }
        ty
    }

    fn chercher(&self, nom: &str) -> Option<&Variable> {
        self.portees.iter().rev().find_map(|p| p.get(nom))
    }

    /// Déclare une variable. Refusé si le nom est encore visible (même bloc ou bloc englobant).
    fn declarer(&mut self, nom: &Nom, ty: Type, sorte: Sorte) {
        if let Some(ancienne) = self.chercher(&nom.texte) {
            let (ligne, _) = ligne_colonne(self.src, ancienne.span.debut);
            self.diags.push(
                Diag::new(nom.span, format!("`{}` existe déjà (ligne {ligne})", nom.texte))
                    .aide("choisis un autre nom : un nom encore visible ne peut pas être redéclaré"),
            );
            return;
        }
        let v = Variable { ty, sorte, span: nom.span };
        self.portees.last_mut().unwrap().insert(nom.texte.clone(), v);
    }

    /// Vérifie un bloc ; `avant` = variables à déclarer au début (le compteur d'une boucle).
    fn bloc(&mut self, b: &Bloc, avant: Vec<(&Nom, Type, Sorte)>) {
        self.portees.push(HashMap::new());
        for (nom, ty, sorte) in avant {
            self.declarer(nom, ty, sorte);
        }
        for i in &b.instrs {
            self.instr(i);
        }
        self.portees.pop();
    }

    /// Vérifie que `e` est du type `attendu`.
    fn attendre(&mut self, e: &Expr, attendu: Type) {
        let t = self.expr(e);
        if t != attendu && t != Type::Erreur && attendu != Type::Erreur {
            let msg = format!("types différents : `{}` attendu, `{}` donné", attendu.nom(), t.nom());
            self.erreur(e.span, msg);
        }
    }

    fn instr(&mut self, i: &Instr) {
        match i {
            Instr::Declarer { modifiable, nom, ty, valeur } => {
                let ty = match ty {
                    Some(t) => self.type_ecrit(t),
                    None => {
                        self.diags.push(
                            Diag::new(nom.span, format!("il faut écrire le type de `{}`", nom.texte))
                                .aide(format!("par exemple `{}: Int`", nom.texte)),
                        );
                        Type::Erreur
                    }
                };
                self.attendre(valeur, ty);
                self.declarer(nom, ty, if *modifiable { Sorte::Var } else { Sorte::Let });
            }
            Instr::Affecter { nom, op, valeur } => {
                let Some(var) = self.chercher(&nom.texte) else {
                    self.erreur(nom.span, format!("variable inconnue `{}`", nom.texte));
                    self.expr(valeur);
                    return;
                };
                let (ty, sorte) = (var.ty, var.sorte);
                let n = &nom.texte;
                match sorte {
                    Sorte::Var => {}
                    Sorte::Let => self.diags.push(
                        Diag::new(nom.span, format!("`{n}` est déclarée avec let"))
                            .aide(format!("déclare-la avec `var {n}` si elle doit changer")),
                    ),
                    Sorte::Param => self.diags.push(
                        Diag::new(nom.span, format!("`{n}` est un paramètre : il ne peut pas changer"))
                            .aide(format!("copie-le d'abord : `var {n}2: ... := {n}`")),
                    ),
                    Sorte::Compteur => {
                        self.erreur(nom.span, format!("`{n}` est un compteur de boucle : il ne peut pas changer"))
                    }
                }
                if op.is_some() && ty != Type::Int && ty != Type::Erreur {
                    self.erreur(nom.span, format!("`{n}` n'est pas un `Int`"));
                }
                self.attendre(valeur, ty);
            }
            Instr::Retour { valeur, span } => match (valeur, self.retour) {
                (None, Type::Rien | Type::Erreur) => {}
                (None, t) => self.erreur(*span, format!("cette fonction doit renvoyer un `{}`", t.nom())),
                (Some(e), Type::Rien) => {
                    self.expr(e);
                    self.diags.push(
                        Diag::new(e.span, "cette fonction ne renvoie rien")
                            .aide("ajoute `-> Type` après les paramètres pour renvoyer une valeur"),
                    );
                }
                (Some(e), t) => self.attendre(e, t),
            },
            Instr::Sortir(span) | Instr::Continuer(span) => {
                if self.boucles == 0 {
                    let mot = if matches!(i, Instr::Sortir(_)) { "break" } else { "continue" };
                    self.erreur(*span, format!("`{mot}` en dehors d'une boucle"));
                }
            }
            Instr::Si { cond, alors, sinon } => {
                self.attendre(cond, Type::Bool);
                self.bloc(alors, vec![]);
                if let Some(s) = sinon {
                    self.bloc(s, vec![]);
                }
            }
            Instr::TantQue { cond, corps } => {
                self.attendre(cond, Type::Bool);
                self.boucles += 1;
                self.bloc(corps, vec![]);
                self.boucles -= 1;
            }
            Instr::Boucle { compteur, debut, fin, corps, .. } => {
                self.attendre(debut, Type::Int);
                self.attendre(fin, Type::Int);
                self.boucles += 1;
                self.bloc(corps, vec![(compteur, Type::Int, Sorte::Compteur)]);
                self.boucles -= 1;
            }
            Instr::Expr(e) => {
                self.expr(e);
                if !matches!(e.genre, ExprGenre::Appel { .. }) {
                    let mut d = Diag::new(e.span, "cette expression ne fait rien");
                    if let ExprGenre::Binaire(OpBin::Egal, ..) = e.genre {
                        d = d.aide("`=` compare ; pour donner une valeur, écris `:=`");
                    }
                    self.diags.push(d);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> Type {
        let t = self.expr_genre(e);
        self.types[e.id] = t;
        t
    }

    fn expr_genre(&mut self, e: &Expr) -> Type {
        match &e.genre {
            ExprGenre::Entier(v) => {
                if *v > i64::MAX as u64 {
                    self.erreur(e.span, "nombre trop grand pour un `Int` (maximum 9223372036854775807)");
                }
                Type::Int
            }
            ExprGenre::Bool(_) => Type::Bool,
            ExprGenre::Var(nom) => match self.chercher(nom) {
                Some(v) => v.ty,
                None => {
                    self.erreur(e.span, format!("variable inconnue `{nom}`"));
                    Type::Erreur
                }
            },
            ExprGenre::Appel { nom, args } => self.appel(nom, args),
            ExprGenre::Unaire(OpUn::Moins, x) => {
                // `-9223372036854775808` : le seul cas où le nombre seul serait trop grand.
                if let ExprGenre::Entier(v) = x.genre
                    && v == 1 << 63
                {
                    self.types[x.id] = Type::Int;
                    return Type::Int;
                }
                self.attendre(x, Type::Int);
                Type::Int
            }
            ExprGenre::Unaire(OpUn::Non, x) => {
                self.attendre(x, Type::Bool);
                Type::Bool
            }
            ExprGenre::Si { cond, alors, sinon } => self.si_valeur(cond, alors, sinon),
            ExprGenre::Binaire(op, a, b) => {
                use OpBin::*;
                match op {
                    Ou | Et => {
                        self.attendre(a, Type::Bool);
                        self.attendre(b, Type::Bool);
                        Type::Bool
                    }
                    Egal | Different => {
                        let ta = self.expr(a);
                        if ta == Type::Rien {
                            self.erreur(a.span, "cette expression ne renvoie rien");
                        }
                        self.attendre(b, ta);
                        Type::Bool
                    }
                    Inf | InfEgal | Sup | SupEgal => {
                        self.attendre(a, Type::Int);
                        self.attendre(b, Type::Int);
                        Type::Bool
                    }
                    Intervalle | IntervalleInclus => {
                        self.expr(a);
                        self.expr(b);
                        self.erreur(e.span, format!("`{}` ne s'utilise que dans `for`", op.texte()));
                        Type::Erreur
                    }
                    Plus | Moins | Fois | Divise | Reste => {
                        self.attendre(a, Type::Int);
                        self.attendre(b, Type::Int);
                        Type::Int
                    }
                }
            }
        }
    }

    fn si_valeur(&mut self, cond: &Expr, alors: &Bloc, sinon: &Bloc) -> Type {
        self.attendre(cond, Type::Bool);
        let (a, b) = (self.bloc_valeur(alors), self.bloc_valeur(sinon));
        if a != b && a != Type::Erreur && b != Type::Erreur {
            let msg = format!("les deux branches doivent donner le même type : `{}` et `{}`", a.nom(), b.nom());
            self.erreur(sinon.debut, msg);
            return Type::Erreur;
        }
        a
    }

    /// Le type d'un bloc qui donne une valeur : celui de sa dernière ligne.
    fn bloc_valeur(&mut self, b: &Bloc) -> Type {
        self.portees.push(HashMap::new());
        let (dernier, avant) = match b.instrs.split_last() {
            Some((d, a)) => (Some(d), a),
            None => (None, &[][..]),
        };
        for i in avant {
            self.instr(i);
        }
        let ty = match dernier {
            Some(Instr::Expr(e)) => self.expr(e),
            Some(Instr::Si { cond, alors, sinon: Some(sinon) }) => self.si_valeur(cond, alors, sinon),
            autre => {
                if let Some(i) = autre {
                    self.instr(i);
                }
                self.diags.push(
                    Diag::new(b.fin, "cette branche doit finir par une valeur")
                        .aide("la dernière ligne d'une branche est la valeur du `if`"),
                );
                Type::Erreur
            }
        };
        self.portees.pop();
        ty
    }

    fn appel(&mut self, nom: &Nom, args: &[Expr]) -> Type {
        if PREDEFINIES.contains(&nom.texte.as_str()) {
            if args.len() != 1 {
                self.erreur(nom.span, format!("`{}` prend exactement 1 argument", nom.texte));
            }
            for a in args {
                let t = self.expr(a);
                if !matches!(t, Type::Int | Type::Bool | Type::Erreur) {
                    self.erreur(a.span, format!("`{}` n'affiche que des `Int` et des `Bool`", nom.texte));
                }
            }
            return Type::Rien;
        }
        let Some(sig) = self.fonctions.get(nom.texte.as_str()) else {
            self.erreur(nom.span, format!("fonction inconnue `{}`", nom.texte));
            args.iter().for_each(|a| {
                self.expr(a);
            });
            return Type::Erreur;
        };
        let (params, retour) = (sig.params.clone(), sig.retour);
        if params.len() != args.len() {
            let msg = format!("`{}` prend {} argument(s), {} donné(s)", nom.texte, params.len(), args.len());
            self.erreur(nom.span, msg);
        }
        for (i, a) in args.iter().enumerate() {
            match params.get(i) {
                Some(&t) => self.attendre(a, t),
                None => {
                    self.expr(a);
                }
            }
        }
        retour
    }
}
