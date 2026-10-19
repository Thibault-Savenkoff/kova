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

#[derive(Clone, PartialEq, Debug)]
pub enum Type {
    Int,
    Float,
    Bool,
    Char,
    /// `(Int, Float)` : au moins 2 éléments.
    Tuple(Vec<Type>),
    /// Ce que renvoie une fonction sans `->`.
    Rien,
    /// Type inconnu à cause d'une erreur déjà signalée : on ne la répète pas.
    Erreur,
}

impl Type {
    /// Le type comme dans le code : `Int`, `(Int, Float)`.
    pub fn texte(&self) -> String {
        match self {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::Bool => "Bool".into(),
            Type::Char => "Char".into(),
            Type::Tuple(ts) => format!("({})", ts.iter().map(Type::texte).collect::<Vec<_>>().join(", ")),
            Type::Rien => "rien".into(),
            Type::Erreur => "?".into(),
        }
    }
}

/// Les fonctions prédéfinies : `print(x)` passe à la ligne, `write(x)` non.
pub const PREDEFINIES: &[&str] = &["print", "write"];

/// Les méthodes prédéfinies (J2 : les conversions) : (type du receveur, nom, résultat).
pub const METHODES: &[(Type, &str, Type)] = &[
    (Type::Int, "to_float", Type::Float),
    (Type::Float, "to_int", Type::Int),
    (Type::Char, "to_int", Type::Int),
];

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
            v.signaler_inconnus(t);
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
            if params.contains_key(&p.nom.texte) {
                v.erreur(p.nom.span, format!("le paramètre `{}` existe déjà", p.nom.texte));
            }
            params.insert(p.nom.texte.clone(), Variable { ty: type_de(&p.ty), sorte: Sorte::Param, span: p.nom.span });
        }
        v.portees = vec![params];
        v.bloc(&fonc.corps, vec![]);
        if v.retour != Type::Rien && !termine(&fonc.corps) {
            v.diags.push(
                Diag::new(fonc.corps.fin, format!("`{}` peut arriver ici sans `return`", fonc.nom.texte))
                    .aide(format!("la fonction doit renvoyer un `{}` dans tous les cas", v.retour.texte())),
            );
        }
    }

    (Infos { types: v.types }, v.diags)
}

/// Le type écrit ; `Erreur` s'il contient un nom inconnu.
pub fn type_de(t: &TypeAst) -> Type {
    match t {
        TypeAst::Nom(n) => match n.texte.as_str() {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "Bool" => Type::Bool,
            "Char" => Type::Char,
            _ => Type::Erreur,
        },
        TypeAst::Tuple(ts) => {
            let ts: Vec<Type> = ts.iter().map(type_de).collect();
            if ts.contains(&Type::Erreur) { Type::Erreur } else { Type::Tuple(ts) }
        }
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

    fn signaler_inconnus(&mut self, t: &TypeAst) {
        match t {
            TypeAst::Nom(n) if type_de(t) == Type::Erreur => self.erreur(n.span, format!("type inconnu `{}`", n.texte)),
            TypeAst::Nom(_) => {}
            TypeAst::Tuple(ts) => ts.iter().for_each(|t| self.signaler_inconnus(t)),
        }
    }

    fn type_ecrit(&mut self, t: &TypeAst) -> Type {
        self.signaler_inconnus(t);
        type_de(t)
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

    /// Déclare les noms d'un motif : `(a, b)` reçoit un `(Int, Float)` → `a: Int`, `b: Float`.
    fn lier(&mut self, m: &Motif, ty: &Type, sorte: Sorte) {
        match (m, ty) {
            (Motif::Nom(n), _) => self.declarer(n, ty.clone(), sorte),
            (Motif::Ignorer(_), _) => {}
            (Motif::Tuple(ms, _), Type::Tuple(ts)) if ms.len() == ts.len() => {
                for (m, t) in ms.iter().zip(ts) {
                    self.lier(m, t, sorte);
                }
            }
            (Motif::Tuple(ms, span), _) => {
                if *ty != Type::Erreur {
                    let msg = format!("ce motif a {} éléments, mais la valeur est un `{}`", ms.len(), ty.texte());
                    self.erreur(*span, msg);
                }
                for m in ms {
                    self.lier(m, &Type::Erreur, sorte);
                }
            }
        }
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
    fn attendre(&mut self, e: &Expr, attendu: &Type) {
        let t = self.expr(e);
        if t != *attendu && t != Type::Erreur && *attendu != Type::Erreur {
            let msg = format!("types différents : `{}` attendu, `{}` donné", attendu.texte(), t.texte());
            let mut d = Diag::new(e.span, msg);
            if let (Type::Float, ExprGenre::Entier(v)) = (attendu, &e.genre) {
                d = d.aide(format!("pour un `Float`, écris `{v}.0`"));
            } else if matches!((attendu, &t), (Type::Int, Type::Float) | (Type::Float, Type::Int)) {
                d = d.aide("Kova ne convertit jamais tout seul : utilise `.to_float()` ou `.to_int()`");
            }
            self.diags.push(d);
        }
    }

    /// Vérifie que `e` a un des types `permis` ; renvoie son type.
    fn parmi(&mut self, e: &Expr, permis: &[Type], quoi: &str) -> Type {
        let t = self.expr(e);
        if t == Type::Erreur || permis.contains(&t) {
            return t;
        }
        let noms: Vec<String> = permis.iter().map(|t| format!("`{}`", t.texte())).collect();
        self.erreur(e.span, format!("{quoi} s'applique à {}, pas à `{}`", noms.join(" ou "), t.texte()));
        Type::Erreur
    }

    /// Le type d'une cible de `:=`, en vérifiant qu'elle peut changer.
    /// `vues` : les lieux déjà rencontrés dans la même cible (`(a, a) := …` est refusé).
    fn cible(&mut self, c: &Cible, vues: &mut Vec<(String, Vec<usize>)>) -> Type {
        match c {
            Cible::Tuple(cs, _) => {
                let ts: Vec<Type> = cs.iter().map(|c| self.cible(c, vues)).collect();
                if ts.contains(&Type::Erreur) { Type::Erreur } else { Type::Tuple(ts) }
            }
            Cible::Lieu { nom, champs, span } => {
                let n = &nom.texte;
                // `t` et `t.0` dans la même cible : la même case recevrait deux valeurs.
                let chevauche = vues.iter().any(|(m, ch)| m == n && (ch.starts_with(champs) || champs.starts_with(ch)));
                if chevauche {
                    self.erreur(*span, format!("`{}` reçoit deux valeurs dans la même affectation", &self.src[span.debut..span.fin]));
                }
                vues.push((n.clone(), champs.clone()));
                let Some(var) = self.chercher(n) else {
                    self.erreur(nom.span, format!("variable inconnue `{n}`"));
                    return Type::Erreur;
                };
                let (mut ty, sorte) = (var.ty.clone(), var.sorte);
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
                for &i in champs {
                    ty = self.champ(ty, i, *span);
                }
                ty
            }
        }
    }

    /// Le type de `t.i` quand `t` est de type `ty`.
    fn champ(&mut self, ty: Type, i: usize, span: Span) -> Type {
        match ty {
            Type::Tuple(ts) if i < ts.len() => ts[i].clone(),
            Type::Tuple(ts) => {
                let msg = format!("ce tuple a {} éléments : de `.0` à `.{}`", ts.len(), ts.len() - 1);
                self.erreur(span, msg);
                Type::Erreur
            }
            Type::Erreur => Type::Erreur,
            autre => {
                self.erreur(span, format!("`.{i}` s'applique à un tuple, pas à `{}`", autre.texte()));
                Type::Erreur
            }
        }
    }

    fn instr(&mut self, i: &Instr) {
        match i {
            Instr::Declarer { modifiable, motif, ty, valeur } => {
                let ty = match ty {
                    Some(t) => self.type_ecrit(t),
                    None => {
                        let (span, nom) = match motif {
                            Motif::Nom(n) => (n.span, n.texte.clone()),
                            Motif::Tuple(_, s) | Motif::Ignorer(s) => (*s, self.src[s.debut..s.fin].to_string()),
                        };
                        self.diags.push(
                            Diag::new(span, format!("il faut écrire le type de `{nom}`"))
                                .aide(format!("par exemple `{nom}: Int`")),
                        );
                        Type::Erreur
                    }
                };
                self.attendre(valeur, &ty);
                self.lier(motif, &ty, if *modifiable { Sorte::Var } else { Sorte::Let });
            }
            Instr::Affecter { cible, op, valeur } => {
                let ty = self.cible(cible, &mut vec![]);
                if op.is_some() && !matches!(ty, Type::Int | Type::Float | Type::Erreur) {
                    let texte = &self.src[cible.span().debut..cible.span().fin];
                    self.erreur(cible.span(), format!("`{texte}` n'est ni un `Int` ni un `Float`"));
                }
                self.attendre(valeur, &ty);
            }
            Instr::Retour { valeur, span } => match (valeur, self.retour.clone()) {
                (None, Type::Rien | Type::Erreur) => {}
                (None, t) => self.erreur(*span, format!("cette fonction doit renvoyer un `{}`", t.texte())),
                (Some(e), Type::Rien) => {
                    self.expr(e);
                    self.diags.push(
                        Diag::new(e.span, "cette fonction ne renvoie rien")
                            .aide("ajoute `-> Type` après les paramètres pour renvoyer une valeur"),
                    );
                }
                (Some(e), t) => self.attendre(e, &t),
            },
            Instr::Sortir(span) | Instr::Continuer(span) => {
                if self.boucles == 0 {
                    let mot = if matches!(i, Instr::Sortir(_)) { "break" } else { "continue" };
                    self.erreur(*span, format!("`{mot}` en dehors d'une boucle"));
                }
            }
            Instr::Si { cond, alors, sinon } => {
                self.attendre(cond, &Type::Bool);
                self.bloc(alors, vec![]);
                if let Some(s) = sinon {
                    self.bloc(s, vec![]);
                }
            }
            Instr::TantQue { cond, corps } => {
                self.attendre(cond, &Type::Bool);
                self.boucles += 1;
                self.bloc(corps, vec![]);
                self.boucles -= 1;
            }
            Instr::Boucle { compteur, debut, fin, corps, .. } => {
                self.attendre(debut, &Type::Int);
                self.attendre(fin, &Type::Int);
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
        self.types[e.id] = t.clone();
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
            ExprGenre::Flottant(_) => Type::Float,
            ExprGenre::Caractere(_) => Type::Char,
            ExprGenre::Bool(_) => Type::Bool,
            ExprGenre::Var(nom) => match self.chercher(nom) {
                Some(v) => v.ty.clone(),
                None => {
                    self.erreur(e.span, format!("variable inconnue `{nom}`"));
                    Type::Erreur
                }
            },
            ExprGenre::Tuple(elems) => {
                let mut ts = vec![];
                for x in elems {
                    let t = self.expr(x);
                    if t == Type::Rien {
                        self.erreur(x.span, "cette expression ne donne pas de valeur à mettre dans un tuple");
                    }
                    ts.push(t);
                }
                if ts.iter().any(|t| matches!(t, Type::Erreur | Type::Rien)) { Type::Erreur } else { Type::Tuple(ts) }
            }
            ExprGenre::Champ(x, i) => {
                let t = self.expr(x);
                self.champ(t, *i, e.span)
            }
            ExprGenre::Appel { nom, args } => self.appel(nom, args),
            ExprGenre::Methode { recu, nom, args } => {
                let t = self.expr(recu);
                for a in args {
                    self.expr(a);
                }
                if t == Type::Erreur {
                    return Type::Erreur;
                }
                let Some((_, _, resultat)) = METHODES.iter().find(|(r, n, _)| *r == t && *n == nom.texte) else {
                    let connues: Vec<String> =
                        METHODES.iter().filter(|(r, ..)| *r == t).map(|(_, n, _)| format!("`{n}`")).collect();
                    let mut d = Diag::new(nom.span, format!("`{}` n'a pas de méthode `{}`", t.texte(), nom.texte));
                    if !connues.is_empty() {
                        d = d.aide(format!("méthodes de `{}` : {}", t.texte(), connues.join(", ")));
                    }
                    self.diags.push(d);
                    return Type::Erreur;
                };
                if !args.is_empty() {
                    self.erreur(nom.span, format!("`{}` ne prend pas d'argument", nom.texte));
                }
                resultat.clone()
            }
            ExprGenre::Unaire(OpUn::Moins, x) => {
                // `-9223372036854775808` : le seul cas où le nombre seul serait trop grand.
                if let ExprGenre::Entier(v) = x.genre
                    && v == 1 << 63
                {
                    self.types[x.id] = Type::Int;
                    return Type::Int;
                }
                self.parmi(x, &[Type::Int, Type::Float], "`-`")
            }
            ExprGenre::Unaire(OpUn::Non, x) => {
                self.attendre(x, &Type::Bool);
                Type::Bool
            }
            ExprGenre::Si { cond, alors, sinon } => self.si_valeur(cond, alors, sinon),
            ExprGenre::Binaire(op, a, b) => {
                use OpBin::*;
                match op {
                    Ou | Et => {
                        self.attendre(a, &Type::Bool);
                        self.attendre(b, &Type::Bool);
                        Type::Bool
                    }
                    Egal | Different => {
                        let ta = self.expr(a);
                        match ta {
                            Type::Rien => self.erreur(a.span, "cette expression ne renvoie rien"),
                            Type::Tuple(_) => self.diags.push(
                                Diag::new(e.span, format!("`{}` ne compare pas encore des tuples", op.texte()))
                                    .aide("compare les éléments un par un : `t.0 = u.0 && t.1 = u.1`"),
                            ),
                            _ => {}
                        }
                        self.attendre(b, &ta);
                        Type::Bool
                    }
                    Inf | InfEgal | Sup | SupEgal => {
                        let t = self.parmi(a, &[Type::Int, Type::Float, Type::Char], &format!("`{}`", op.texte()));
                        self.attendre(b, &t);
                        Type::Bool
                    }
                    Intervalle | IntervalleInclus => {
                        self.expr(a);
                        self.expr(b);
                        self.erreur(e.span, format!("`{}` ne s'utilise que dans `for`", op.texte()));
                        Type::Erreur
                    }
                    Reste => {
                        self.attendre(a, &Type::Int);
                        self.attendre(b, &Type::Int);
                        Type::Int
                    }
                    Plus | Moins | Fois | Divise => {
                        let t = self.parmi(a, &[Type::Int, Type::Float], &format!("`{}`", op.texte()));
                        self.attendre(b, &t);
                        t
                    }
                }
            }
        }
    }

    fn si_valeur(&mut self, cond: &Expr, alors: &Bloc, sinon: &Bloc) -> Type {
        self.attendre(cond, &Type::Bool);
        let (a, b) = (self.bloc_valeur(alors), self.bloc_valeur(sinon));
        if a != b && a != Type::Erreur && b != Type::Erreur {
            let msg = format!("les deux branches doivent donner le même type : `{}` et `{}`", a.texte(), b.texte());
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
                match self.expr(a) {
                    Type::Rien => {
                        self.erreur(a.span, format!("`{}` : cette expression ne donne pas de valeur à afficher", nom.texte))
                    }
                    Type::Tuple(_) => self.diags.push(
                        Diag::new(a.span, format!("`{}` n'affiche pas encore les tuples", nom.texte))
                            .aide("affiche les éléments un par un : `print(t.0)`"),
                    ),
                    _ => {}
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
        let (params, retour) = (sig.params.clone(), sig.retour.clone());
        if params.len() != args.len() {
            let msg = format!("`{}` prend {} argument(s), {} donné(s)", nom.texte, params.len(), args.len());
            self.erreur(nom.span, msg);
        }
        for (i, a) in args.iter().enumerate() {
            match params.get(i) {
                Some(t) => self.attendre(a, t),
                None => {
                    self.expr(a);
                }
            }
        }
        retour
    }
}
