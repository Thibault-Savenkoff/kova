// KIR : le programme réduit à des instructions très simples, communes aux deux backends.
//
// Chaque fonction a des cases numérotées (« locaux », toutes des entiers 64 bits ;
// un Bool vaut 0 ou 1) et des blocs. Un bloc = une suite d'instructions + une fin
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
use crate::types::{Infos, Type};

pub type Local = usize;
pub type NumBloc = usize;

pub struct Programme {
    pub fonctions: Vec<Fonction>,
    pub principale: usize,
}

pub struct Fonction {
    pub nom: String,
    /// Les paramètres sont les locaux `0..nb_params`.
    pub nb_params: usize,
    pub nb_locaux: usize,
    pub renvoie: bool,
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
    Const(Local, i64),
    Copie(Local, Local),
    /// `dst := a op b` ; `+ - * / %` vérifient dépassement et division par zéro.
    Binaire(Local, Op, Local, Local, Lieu),
    /// `dst := -a`, vérifié (`-(-9223372036854775808)` déborde).
    Neg(Local, Local, Lieu),
    /// `dst := !a` (Bool).
    Non(Local, Local),
    Appel(Option<Local>, usize, Vec<Local>),
    /// `print` (`ligne` = true) ou `write`.
    Afficher { valeur: Local, booleen: bool, ligne: bool },
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
    Retour(Option<Local>),
    /// Jamais atteint (le typage garantit qu'il y a un `return` avant).
    Inatteignable,
}

/// Les erreurs d'exécution. Le runtime C (`runtime/runtime.c`) a les mêmes textes.
#[derive(Clone, Copy)]
pub enum Panne {
    Depassement = 0,
    DivisionParZero = 1,
}

impl Panne {
    pub fn message(self) -> &'static str {
        match self {
            Panne::Depassement => "dépassement d'entier",
            Panne::DivisionParZero => "division par zéro",
        }
    }
}

/// Message complet d'une erreur d'exécution, identique en `run` et en `build`.
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
                nb_locaux: 0,
                blocs: vec![],
                courant: 0,
                boucles: vec![],
            };
            for p in &fonc.params {
                let l = t.nouveau_local();
                t.noms[0].insert(p.nom.texte.clone(), l);
            }
            t.courant = t.nouveau_bloc();
            t.bloc(&fonc.corps);
            let fin = if fonc.retour.is_some() { Fin::Inatteignable } else { Fin::Retour(None) };
            t.terminer(fin);
            Fonction {
                nom: fonc.nom.texte.clone(),
                nb_params: fonc.params.len(),
                nb_locaux: t.nb_locaux,
                renvoie: fonc.retour.is_some(),
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
    /// Nom → local, une table par bloc ouvert.
    noms: Vec<HashMap<String, Local>>,
    nb_locaux: usize,
    /// Blocs en construction ; `None` = fin pas encore posée.
    blocs: Vec<(Vec<Inst>, Option<Fin>)>,
    courant: NumBloc,
    /// Pour chaque boucle ouverte : (cible de `continue`, cible de `break`).
    boucles: Vec<(NumBloc, NumBloc)>,
}

impl Traducteur<'_> {
    fn nouveau_local(&mut self) -> Local {
        self.nb_locaux += 1;
        self.nb_locaux - 1
    }
    fn nouveau_bloc(&mut self) -> NumBloc {
        self.blocs.push((vec![], None));
        self.blocs.len() - 1
    }
    fn emettre(&mut self, i: Inst) {
        self.blocs[self.courant].0.push(i);
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
    fn local_de(&self, nom: &str) -> Local {
        *self.noms.iter().rev().find_map(|t| t.get(nom)).expect("variable vérifiée par le typage")
    }
    fn constante(&mut self, v: i64) -> Local {
        let l = self.nouveau_local();
        self.emettre(Inst::Const(l, v));
        l
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
                let l = self.nouveau_local();
                self.emettre(Inst::Copie(l, v));
                self.noms.last_mut().unwrap().insert(nom.texte.clone(), l);
            }
            Instr::Affecter { nom, op, valeur } => {
                let cible = self.local_de(&nom.texte);
                let v = self.expr(valeur);
                match op {
                    None => self.emettre(Inst::Copie(cible, v)),
                    Some(op) => {
                        let lieu = self.lieu(nom.span);
                        self.emettre(Inst::Binaire(cible, op_kir(*op), cible, v, lieu));
                    }
                }
            }
            Instr::Retour { valeur, .. } => {
                let v = valeur.as_ref().map(|e| self.expr(e));
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
                let c = self.expr(cond);
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
                let c = self.expr(cond);
                self.aller(Fin::Si(c, b_corps, sortie), b_corps);
                self.boucles.push((entete, sortie));
                self.bloc(corps);
                self.boucles.pop();
                self.aller(Fin::Saut(entete), sortie);
            }
            Instr::Boucle { compteur, debut, fin, inclus, corps } => {
                // La borne de fin est calculée une seule fois, avant la boucle.
                let d = self.expr(debut);
                let i = self.nouveau_local();
                self.emettre(Inst::Copie(i, d));
                let f = self.expr(fin);
                let borne = self.nouveau_local();
                self.emettre(Inst::Copie(borne, f));

                let (entete, b_corps, suite, increment, sortie) =
                    (self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc(), self.nouveau_bloc());
                let lieu = self.lieu(compteur.span);
                self.aller(Fin::Saut(entete), entete);
                let c = self.nouveau_local();
                let test = if *inclus { Op::InfEgal } else { Op::Inf };
                self.emettre(Inst::Binaire(c, test, i, borne, lieu));
                self.aller(Fin::Si(c, b_corps, sortie), b_corps);

                self.boucles.push((suite, sortie));
                self.noms.push(HashMap::from([(compteur.texte.clone(), i)]));
                self.bloc(corps);
                self.noms.pop();
                self.boucles.pop();
                self.aller(Fin::Saut(suite), suite);

                // `continue` arrive ici. Pour `..=`, on sort AVANT d'ajouter 1 si on est
                // sur la borne : `from i := 1 to 9223372036854775807` ne déborde pas.
                if *inclus {
                    let dernier = self.nouveau_local();
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

    fn expr(&mut self, e: &Expr) -> Local {
        match &e.genre {
            ExprGenre::Entier(v) => self.constante(*v as i64),
            ExprGenre::Bool(b) => self.constante(*b as i64),
            ExprGenre::Var(nom) => self.local_de(nom),
            ExprGenre::Appel { nom, args } => {
                let vals: Vec<Local> = args.iter().map(|a| self.expr(a)).collect();
                if let n @ ("print" | "write") = nom.texte.as_str() {
                    let booleen = self.infos.types[args[0].id] == Type::Bool;
                    self.emettre(Inst::Afficher { valeur: vals[0], booleen, ligne: n == "print" });
                    return self.constante(0);
                }
                let num = self.numeros[nom.texte.as_str()];
                let dst = self.nouveau_local();
                let resultat = (self.infos.types[e.id] != Type::Rien).then_some(dst);
                self.emettre(Inst::Appel(resultat, num, vals));
                dst
            }
            ExprGenre::Unaire(OpUn::Moins, x) => {
                // Un nombre écrit avec `-` devant est une constante (gère `-9223372036854775808`).
                if let ExprGenre::Entier(v) = x.genre {
                    return self.constante((v as i64).wrapping_neg());
                }
                let a = self.expr(x);
                let dst = self.nouveau_local();
                let lieu = self.lieu(e.span);
                self.emettre(Inst::Neg(dst, a, lieu));
                dst
            }
            ExprGenre::Unaire(OpUn::Non, x) => {
                let a = self.expr(x);
                let dst = self.nouveau_local();
                self.emettre(Inst::Non(dst, a));
                dst
            }
            ExprGenre::Binaire(op @ (OpBin::Et | OpBin::Ou), a, b) => {
                // Court-circuit : `b` n'est calculé que si `a` ne suffit pas.
                let dst = self.nouveau_local();
                let va = self.expr(a);
                self.emettre(Inst::Copie(dst, va));
                let (calcul_b, apres) = (self.nouveau_bloc(), self.nouveau_bloc());
                let fin = if *op == OpBin::Et { Fin::Si(va, calcul_b, apres) } else { Fin::Si(va, apres, calcul_b) };
                self.aller(fin, calcul_b);
                let vb = self.expr(b);
                self.emettre(Inst::Copie(dst, vb));
                self.aller(Fin::Saut(apres), apres);
                dst
            }
            ExprGenre::Binaire(op, a, b) => {
                let (va, vb) = (self.expr(a), self.expr(b));
                let dst = self.nouveau_local();
                // L'erreur pointe sur l'opérateur : le premier caractère non blanc après `a`.
                let apres_a = &self.src[a.span.fin..];
                let pos = a.span.fin + apres_a.len() - apres_a.trim_start().len();
                let lieu = self.lieu(Span::new(pos, pos));
                self.emettre(Inst::Binaire(dst, op_kir(*op), va, vb, lieu));
                dst
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
