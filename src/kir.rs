// KIR : le programme réduit à des instructions très simples, communes aux deux backends.
//
// Chaque fonction a des cases numérotées (« locaux ») de 64 bits : entier ou flottant
// (un Bool vaut 0 ou 1). Une valeur Kova occupe une ou plusieurs cases (`Valeur`) :
// un `Int` en prend une, un tuple `(Int, Float)` deux, un résultat vide aucune.
// Une fonction a aussi des blocs. Un bloc = une suite d'instructions + une fin
// (saut, branchement ou retour). Il n'y a plus ni `for`, ni `from`, ni `&&` :
//
//   from i := 1 to 3 { print(i) }
//
//   b0: %0 := 1          %1 := 3          saut b1
//   b1: %2 := %0 <= %1                    si %2 alors b2 sinon b4
//   b2: afficher %0                       saut b3
//   b3: %3 := %0 = %1                     si %3 alors b4 sinon b5
//   b5: %0 := %0 + 1                      saut b1
//   b4: ...
//
// Les erreurs d'exécution (dépassement, division par zéro) sont vérifiées dans
// `Binaire` et `Neg` : chaque backend les traduit à l'identique.

use std::collections::HashMap;

use crate::ast::{self, Bloc, Expr, ExprGenre, Instr, OpBin, OpUn};
use crate::diag::{Span, ligne_colonne};
use crate::types::{Infos, Type, type_de};

pub type Local = usize;
pub type NumBloc = usize;
/// Les cases qui contiennent une valeur Kova.
pub type Valeur = Vec<Local>;

/// Le type machine d'une case.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scalaire {
    /// Int, Bool, Char.
    Entier,
    Flottant,
}

/// Les cases nécessaires pour une valeur de ce type, dans l'ordre.
pub fn scalaires(t: &Type) -> Vec<Scalaire> {
    match t {
        Type::Int | Type::Bool | Type::Char => vec![Scalaire::Entier],
        Type::Float => vec![Scalaire::Flottant],
        Type::Rien => vec![],
        Type::Erreur => unreachable!("type erroné après le typage"),
    }
}

pub struct Programme {
    pub fonctions: Vec<Fonction>,
    pub principale: usize,
}

pub struct Fonction {
    pub nom: String,
    /// Les paramètres sont les locaux `0..nb_params`.
    pub nb_params: usize,
    /// Le type de chaque local.
    pub locaux: Vec<Scalaire>,
    pub retours: Vec<Scalaire>,
    pub blocs: Vec<BlocK>,
}

pub struct BlocK {
    pub instrs: Vec<Inst>,
    pub fin: Fin,
}

/// Position dans le fichier source, pour les messages d'erreur à l'exécution.
#[derive(Clone, Copy)]
pub struct Lieu {
    pub ligne: usize,
    pub colonne: usize,
}

pub enum Inst {
    /// Une constante ; pour une case flottante, `v` contient les bits du Float.
    Const(Local, i64),
    Copie(Local, Local),
    /// `dst := a op b` ; `+ - * / %` vérifient dépassement et division par zéro.
    Binaire(Local, Op, Local, Local, Lieu),
    /// `dst := -a`, vérifié (`-(-9223372036854775808)` déborde).
    Neg(Local, Local, Lieu),
    /// `dst := !a` (Bool).
    Non(Local, Local),
    /// `dst := a op b` sur des Float ; `/` vérifie la division par zéro.
    /// Les comparaisons donnent un Bool (case entière).
    Flottant(Local, Op, Local, Local, Lieu),
    NegFlottant(Local, Local),
    Convertir(Local, Conversion, Local, Lieu),
    /// `résultats := f(args)`.
    Appel(Valeur, usize, Valeur),
    /// `print` (`ligne` = true) ou `write`.
    Afficher { valeur: Local, genre: Affichage, ligne: bool },
}

/// Comment afficher une case. Même ordre que dans `runtime/runtime.c`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Affichage {
    Int = 0,
    Bool = 1,
    Float = 2,
    Char = 3,
}

impl Affichage {
    pub fn de(t: &Type) -> Affichage {
        match t {
            Type::Bool => Affichage::Bool,
            Type::Float => Affichage::Float,
            Type::Char => Affichage::Char,
            _ => Affichage::Int,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Conversion {
    /// `n.to_float()`
    EntierVersFlottant,
    /// `x.to_int()` : coupe les décimales ; erreur si NaN ou trop grand.
    FlottantVersEntier,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Op {
    Plus,
    Moins,
    Fois,
    Divise,
    Reste,
    Egal,
    Different,
    Inf,
    InfEgal,
    Sup,
    SupEgal,
}

pub enum Fin {
    Saut(NumBloc),
    Si(Local, NumBloc, NumBloc),
    Retour(Valeur),
    /// Jamais atteint (le typage garantit qu'il y a un `return` avant).
    Inatteignable,
}

/// Les erreurs d'exécution. Le runtime C (`runtime/runtime.c`) a les mêmes textes.
#[derive(Clone, Copy)]
pub enum Panne {
    Depassement = 0,
    DivisionParZero = 1,
    ConversionImpossible = 2,
}

impl Panne {
    pub fn message(self) -> &'static str {
        match self {
            Panne::Depassement => "dépassement d'entier",
            Panne::DivisionParZero => "division par zéro",
            Panne::ConversionImpossible => "conversion impossible : ce Float ne tient pas dans un Int",
        }
    }
}

/// Message complet d'une erreur d'exécution, identique en `run` et en `build`.
/// Un Float tel que l'affiche Kova : `3.0`, `0.30000000000000004`, `1e16`, `NaN`, `-inf`.
///
/// Les chiffres les plus courts qui redonnent exactement le nombre (à égalité, le chiffre
/// pair, comme Python). Écriture scientifique sous 0.0001 et à partir de 10^16.
/// Le runtime C (`ecrire_float`) fait exactement le même calcul, étape par étape.
pub fn texte_float(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    let a = x.abs();
    let (ch, exp) = plus_courts(a);
    let ch: String = ch.iter().map(|d| (b'0' + d) as char).collect();
    let n = ch.len() as i32;
    let corps = if a == 0.0 || (1e-4..1e16).contains(&a) {
        if exp >= 0 {
            let entier: String = (0..=exp).map(|i| ch.as_bytes().get(i as usize).map_or('0', |&c| c as char)).collect();
            let decimales = if n > exp + 1 { &ch[exp as usize + 1..] } else { "0" };
            format!("{entier}.{decimales}")
        } else {
            format!("0.{}{ch}", "0".repeat((-exp - 1) as usize))
        }
    } else if n > 1 {
        format!("{}.{}e{exp}", &ch[..1], &ch[1..])
    } else {
        format!("{ch}e{exp}")
    };
    format!("{}{corps}", if x.is_sign_negative() { "-" } else { "" })
}

/// Les chiffres les plus courts de `a` (≥ 0) qui redonnent `a`, et l'exposant du premier.
fn plus_courts(a: f64) -> (Vec<u8>, i32) {
    if a == 0.0 {
        return (vec![0], 0);
    }
    // Les chiffres exacts : un Float en a au plus ~770, `{:.800e}` les donne tous.
    let s = format!("{a:.800e}");
    let (m, e) = s.split_once('e').unwrap();
    let mut exact: Vec<u8> = m.bytes().filter(|&c| c != b'.').map(|c| c - b'0').collect();
    while exact.len() > 1 && exact.last() == Some(&0) {
        exact.pop();
    }
    let e: i32 = e.parse().unwrap();
    let relire = |ch: &[u8], e: i32| {
        let ch: String = ch.iter().map(|d| (b'0' + d) as char).collect();
        format!("{}.{}e{e}", &ch[..1], &ch[1..]).parse::<f64>().unwrap() == a
    };
    for p in 1..=17 {
        if exact.len() <= p {
            return (exact, e);
        }
        // Les deux nombres de p chiffres qui entourent `a` : `bas` (coupé) et `haut`.
        let bas = exact[..p].to_vec();
        let reste = &exact[p..];
        let haut = plus_un(&bas, e);
        let monter = reste[0] > 5 || (reste[0] == 5 && (reste[1..].iter().any(|&d| d != 0) || bas[p - 1] % 2 == 1));
        let (proche, loin) = if monter { (haut, (bas, e)) } else { ((bas, e), haut) };
        for (mut ch, e) in [proche, loin] {
            if relire(&ch, e) {
                while ch.len() > 1 && ch.last() == Some(&0) {
                    ch.pop();
                }
                return (ch, e);
            }
        }
    }
    unreachable!("17 chiffres suffisent toujours")
}

/// `chiffres + 1` sur le dernier chiffre, avec retenue (`999` → `100` et exposant + 1).
fn plus_un(chiffres: &[u8], e: i32) -> (Vec<u8>, i32) {
    let mut ch = chiffres.to_vec();
    for d in ch.iter_mut().rev() {
        if *d < 9 {
            *d += 1;
            return (ch, e);
        }
        *d = 0;
    }
    ch.insert(0, 1);
    ch.pop();
    (ch, e + 1)
}

pub fn texte_panne(p: Panne, fichier: &str, l: Lieu) -> String {
    format!("erreur: {}\n  --> {fichier}:{}:{}\n", p.message(), l.ligne, l.colonne)
}

// ---- Traduction AST → KIR ----

pub fn traduire(f: &ast::Fichier, infos: &Infos, src: &str) -> Programme {
    let numeros: HashMap<&str, usize> =
        f.fonctions.iter().enumerate().map(|(i, f)| (f.nom.texte.as_str(), i)).collect();
    let fonctions = f
        .fonctions
        .iter()
        .map(|fonc| {
            let mut t = Traducteur {
                src,
                infos,
                numeros: &numeros,
                noms: vec![HashMap::new()],
                locaux: vec![],
                blocs: vec![],
                courant: 0,
                boucles: vec![],
            };
            for p in &fonc.params {
                let v = t.nouvelle_valeur(&type_de(&p.ty));
                t.noms[0].insert(p.nom.texte.clone(), v);
            }
            let nb_params = t.locaux.len();
            t.courant = t.nouveau_bloc();
            t.bloc(&fonc.corps);
            let retour = fonc.retour.as_ref().map_or(Type::Rien, type_de);
            let fin = if retour == Type::Rien { Fin::Retour(vec![]) } else { Fin::Inatteignable };
            t.terminer(fin);
            Fonction {
                nom: fonc.nom.texte.clone(),
                nb_params,
                locaux: t.locaux,
                retours: scalaires(&retour),
                blocs: t.blocs.into_iter().map(|(instrs, fin)| BlocK { instrs, fin: fin.unwrap_or(Fin::Inatteignable) }).collect(),
            }
        })
        .collect();
    Programme { fonctions, principale: numeros["main"] }
}

struct Traducteur<'a> {
    src: &'a str,
    infos: &'a Infos,
    numeros: &'a HashMap<&'a str, usize>,
    /// Nom → ses cases, une table par bloc ouvert.
    noms: Vec<HashMap<String, Valeur>>,
    locaux: Vec<Scalaire>,
    /// Blocs en construction ; `None` = fin pas encore posée.
    blocs: Vec<(Vec<Inst>, Option<Fin>)>,
    courant: NumBloc,
    /// Pour chaque boucle ouverte : (cible de `continue`, cible de `break`).
    boucles: Vec<(NumBloc, NumBloc)>,
}

impl Traducteur<'_> {
    fn nouveau_local(&mut self, s: Scalaire) -> Local {
        self.locaux.push(s);
        self.locaux.len() - 1
    }
    fn entier(&mut self) -> Local {
        self.nouveau_local(Scalaire::Entier)
    }
    /// Des cases neuves pour une valeur de type `t`.
    fn nouvelle_valeur(&mut self, t: &Type) -> Valeur {
        scalaires(t).into_iter().map(|s| self.nouveau_local(s)).collect()
    }
    fn nouveau_bloc(&mut self) -> NumBloc {
        self.blocs.push((vec![], None));
        self.blocs.len() - 1
    }
    fn emettre(&mut self, i: Inst) {
        self.blocs[self.courant].0.push(i);
    }
    /// Copie case par case.
    fn copier(&mut self, dst: &[Local], src: &[Local]) {
        for (&d, &s) in dst.iter().zip(src) {
            self.emettre(Inst::Copie(d, s));
        }
    }
    /// Pose la fin du bloc courant, puis continue dans un bloc neuf
    /// (le code après un `return` ou un `break` y atterrit, jamais exécuté).
    fn terminer(&mut self, fin: Fin) {
        self.blocs[self.courant].1 = Some(fin);
        self.courant = self.nouveau_bloc();
    }
    /// Pose la fin du bloc courant et continue dans `suite`.
    fn aller(&mut self, fin: Fin, suite: NumBloc) {
        self.blocs[self.courant].1 = Some(fin);
        self.courant = suite;
    }
    fn lieu(&self, s: Span) -> Lieu {
        let (ligne, colonne) = ligne_colonne(self.src, s.debut);
        Lieu { ligne, colonne }
    }
    fn valeur_de(&self, nom: &str) -> Valeur {
        self.noms.iter().rev().find_map(|t| t.get(nom)).expect("variable vérifiée par le typage").clone()
    }
    fn constante(&mut self, v: i64) -> Local {
        let l = self.entier();
        self.emettre(Inst::Const(l, v));
        l
    }
    fn constante_float(&mut self, v: f64) -> Local {
        let l = self.nouveau_local(Scalaire::Flottant);
        self.emettre(Inst::Const(l, v.to_bits() as i64));
        l
    }
    fn est_float(&self, e: &Expr) -> bool {
        self.infos.types[e.id] == Type::Float
    }
    /// `dst := a op b`, en entier ou en flottant selon le type de `a`.
    fn operation(&mut self, op: Op, a: Local, b: Local, flottant: bool, lieu: Lieu) -> Local {
        let resultat_float = flottant && matches!(op, Op::Plus | Op::Moins | Op::Fois | Op::Divise);
        let dst = self.nouveau_local(if resultat_float { Scalaire::Flottant } else { Scalaire::Entier });
        self.emettre(if flottant { Inst::Flottant(dst, op, a, b, lieu) } else { Inst::Binaire(dst, op, a, b, lieu) });
        dst
    }

    fn bloc(&mut self, b: &Bloc) {
        self.noms.push(HashMap::new());
        for i in &b.instrs {
            self.instr(i);
        }
        self.noms.pop();
    }

    fn instr(&mut self, i: &Instr) {
        match i {
            Instr::Declarer { nom, valeur, .. } => {
                let v = self.expr(valeur);
                let cases = self.nouvelle_valeur(&self.infos.types[valeur.id].clone());
                self.copier(&cases, &v);
                self.noms.last_mut().unwrap().insert(nom.texte.clone(), cases);
            }
            Instr::Affecter { nom, op, valeur } => {
                let cible = self.valeur_de(&nom.texte);
                let v = self.expr(valeur);
                match op {
                    None => self.copier(&cible, &v),
                    Some(op) => {
                        let lieu = self.lieu(nom.span);
                        let r = self.operation(op_kir(*op), cible[0], v[0], self.est_float(valeur), lieu);
                        self.emettre(Inst::Copie(cible[0], r));
                    }
                }
            }
            Instr::Retour { valeur, .. } => {
                let v = valeur.as_ref().map_or(vec![], |e| self.expr(e));
                self.terminer(Fin::Retour(v));
            }
            Instr::Sortir(_) => {
                let (_, sortie) = *self.boucles.last().unwrap();
                self.terminer(Fin::Saut(sortie));
            }
            Instr::Continuer(_) => {
                let (suite, _) = *self.boucles.last().unwrap();
                self.terminer(Fin::Saut(suite));
            }
            Instr::Si { cond, alors, sinon } => {
                let c = self.expr1(cond);
                let (b_alors, b_sinon, b_apres) = (self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc());
                self.aller(Fin::Si(c, b_alors, b_sinon), b_alors);
                self.bloc(alors);
                self.aller(Fin::Saut(b_apres), b_sinon);
                if let Some(s) = sinon {
                    self.bloc(s);
                }
                self.aller(Fin::Saut(b_apres), b_apres);
            }
            Instr::TantQue { cond, corps } => {
                let (entete, b_corps, sortie) = (self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc());
                self.aller(Fin::Saut(entete), entete);
                let c = self.expr1(cond);
                self.aller(Fin::Si(c, b_corps, sortie), b_corps);
                self.boucles.push((entete, sortie));
                self.bloc(corps);
                self.boucles.pop();
                self.aller(Fin::Saut(entete), sortie);
            }
            Instr::Boucle { compteur, debut, fin, inclus, corps } => {
                // La borne de fin est calculée une seule fois, avant la boucle.
                let d = self.expr1(debut);
                let i = self.entier();
                self.emettre(Inst::Copie(i, d));
                let f = self.expr1(fin);
                let borne = self.entier();
                self.emettre(Inst::Copie(borne, f));

                let (entete, b_corps, suite, increment, sortie) =
                    (self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc());
                let lieu = self.lieu(compteur.span);
                self.aller(Fin::Saut(entete), entete);
                let c = self.entier();
                let test = if *inclus { Op::InfEgal } else { Op::Inf };
                self.emettre(Inst::Binaire(c, test, i, borne, lieu));
                self.aller(Fin::Si(c, b_corps, sortie), b_corps);

                self.boucles.push((suite, sortie));
                self.noms.push(HashMap::from([(compteur.texte.clone(), vec![i])]));
                self.bloc(corps);
                self.noms.pop();
                self.boucles.pop();
                self.aller(Fin::Saut(suite), suite);

                // `continue` arrive ici. Pour `..=`, on sort AVANT d'ajouter 1 si on est
                // sur la borne : `from i := 1 to 9223372036854775807` ne déborde pas.
                if *inclus {
                    let dernier = self.entier();
                    self.emettre(Inst::Binaire(dernier, Op::Egal, i, borne, lieu));
                    self.aller(Fin::Si(dernier, sortie, increment), increment);
                } else {
                    self.aller(Fin::Saut(increment), increment);
                }
                let un = self.constante(1);
                self.emettre(Inst::Binaire(i, Op::Plus, i, un, lieu));
                self.aller(Fin::Saut(entete), sortie);
            }
            Instr::Expr(e) => {
                self.expr(e);
            }
        }
    }

    fn si_valeur(&mut self, cond: &Expr, alors: &Bloc, sinon: &Bloc, ty: &Type) -> Valeur {
        let dst = self.nouvelle_valeur(ty);
        let c = self.expr1(cond);
        let (b_alors, b_sinon, b_apres) = (self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc());
        self.aller(Fin::Si(c, b_alors, b_sinon), b_alors);
        let v = self.bloc_valeur(alors, ty);
        self.copier(&dst, &v);
        self.aller(Fin::Saut(b_apres), b_sinon);
        let v = self.bloc_valeur(sinon, ty);
        self.copier(&dst, &v);
        self.aller(Fin::Saut(b_apres), b_apres);
        dst
    }

    /// La valeur d'un bloc : celle de sa dernière ligne (vérifiée par le typage).
    fn bloc_valeur(&mut self, b: &Bloc, ty: &Type) -> Valeur {
        self.noms.push(HashMap::new());
        let (dernier, avant) = b.instrs.split_last().expect("bloc vérifié par le typage");
        for i in avant {
            self.instr(i);
        }
        let v = match dernier {
            Instr::Expr(e) => self.expr(e),
            Instr::Si { cond, alors, sinon: Some(sinon) } => self.si_valeur(cond, alors, sinon, ty),
            _ => unreachable!("bloc vérifié par le typage"),
        };
        self.noms.pop();
        v
    }

    /// Une expression dont la valeur tient dans une seule case (Int, Bool…).
    fn expr1(&mut self, e: &Expr) -> Local {
        self.expr(e)[0]
    }

    fn expr(&mut self, e: &Expr) -> Valeur {
        match &e.genre {
            ExprGenre::Entier(v) => vec![self.constante(*v as i64)],
            ExprGenre::Flottant(v) => vec![self.constante_float(*v)],
            ExprGenre::Caractere(c) => vec![self.constante(*c as i64)],
            ExprGenre::Bool(b) => vec![self.constante(*b as i64)],
            ExprGenre::Var(nom) => self.valeur_de(nom),
            ExprGenre::Appel { nom, args } => {
                let mut vals = vec![];
                for a in args {
                    vals.extend(self.expr(a));
                }
                if let n @ ("print" | "write") = nom.texte.as_str() {
                    let genre = Affichage::de(&self.infos.types[args[0].id]);
                    self.emettre(Inst::Afficher { valeur: vals[0], genre, ligne: n == "print" });
                    return vec![];
                }
                let num = self.numeros[nom.texte.as_str()];
                let resultat = self.nouvelle_valeur(&self.infos.types[e.id].clone());
                self.emettre(Inst::Appel(resultat.clone(), num, vals));
                resultat
            }
            ExprGenre::Unaire(OpUn::Moins, x) => {
                // Un nombre écrit avec `-` devant est une constante (gère `-9223372036854775808`).
                match x.genre {
                    ExprGenre::Entier(v) => return vec![self.constante((v as i64).wrapping_neg())],
                    ExprGenre::Flottant(v) => return vec![self.constante_float(-v)],
                    _ => {}
                }
                let a = self.expr1(x);
                if self.est_float(x) {
                    let dst = self.nouveau_local(Scalaire::Flottant);
                    self.emettre(Inst::NegFlottant(dst, a));
                    return vec![dst];
                }
                let dst = self.entier();
                let lieu = self.lieu(e.span);
                self.emettre(Inst::Neg(dst, a, lieu));
                vec![dst]
            }
            ExprGenre::Unaire(OpUn::Non, x) => {
                let a = self.expr1(x);
                let dst = self.entier();
                self.emettre(Inst::Non(dst, a));
                vec![dst]
            }
            ExprGenre::Si { cond, alors, sinon } => {
                let ty = self.infos.types[e.id].clone();
                self.si_valeur(cond, alors, sinon, &ty)
            }
            ExprGenre::Binaire(op @ (OpBin::Et | OpBin::Ou), a, b) => {
                // Court-circuit : `b` n'est calculé que si `a` ne suffit pas.
                let dst = self.entier();
                let va = self.expr1(a);
                self.emettre(Inst::Copie(dst, va));
                let (calcul_b, apres) = (self.nouveau_bloc(), self.nouveau_bloc());
                let fin = if *op == OpBin::Et { Fin::Si(va, calcul_b, apres) } else { Fin::Si(va, apres, calcul_b) };
                self.aller(fin, calcul_b);
                let vb = self.expr1(b);
                self.emettre(Inst::Copie(dst, vb));
                self.aller(Fin::Saut(apres), apres);
                vec![dst]
            }
            ExprGenre::Binaire(op, a, b) => {
                let (va, vb) = (self.expr1(a), self.expr1(b));
                // L'erreur pointe sur l'opérateur : le premier caractère non blanc après `a`.
                let apres_a = &self.src[a.span.fin..];
                let pos = a.span.fin + apres_a.len() - apres_a.trim_start().len();
                let lieu = self.lieu(Span::new(pos, pos));
                vec![self.operation(op_kir(*op), va, vb, self.est_float(a), lieu)]
            }
            ExprGenre::Methode { recu, nom, .. } => {
                let v = self.expr1(recu);
                let conversion = match (&self.infos.types[recu.id], nom.texte.as_str()) {
                    (Type::Int, "to_float") => Conversion::EntierVersFlottant,
                    (Type::Float, "to_int") => Conversion::FlottantVersEntier,
                    (Type::Char, "to_int") => return vec![v], // un Char est déjà son code
                    autre => unreachable!("méthode vérifiée par le typage : {autre:?}"),
                };
                let dst = self.nouvelle_valeur(&self.infos.types[e.id].clone())[0];
                let lieu = self.lieu(nom.span);
                self.emettre(Inst::Convertir(dst, conversion, v, lieu));
                vec![dst]
            }
        }
    }
}

fn op_kir(op: OpBin) -> Op {
    match op {
        OpBin::Plus => Op::Plus,
        OpBin::Moins => Op::Moins,
        OpBin::Fois => Op::Fois,
        OpBin::Divise => Op::Divise,
        OpBin::Reste => Op::Reste,
        OpBin::Egal => Op::Egal,
        OpBin::Different => Op::Different,
        OpBin::Inf => Op::Inf,
        OpBin::InfEgal => Op::InfEgal,
        OpBin::Sup => Op::Sup,
        OpBin::SupEgal => Op::SupEgal,
        OpBin::Et | OpBin::Ou | OpBin::Intervalle | OpBin::IntervalleInclus => {
            unreachable!("traité ailleurs")
        }
    }
}
