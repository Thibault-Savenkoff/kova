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
//
// Les contrats deviennent des tests ordinaires suivis d'un `Echec` (message en morceaux) :
//   - `require` : au début de la fonction. Elle reçoit 2 paramètres cachés, la ligne et la
//     colonne de l'appel, pour que le message dise qui l'a mal appelée ;
//   - `ensure` : avant chaque `return`, avec `result` = la valeur renvoyée ;
//   - `check` : là où il est écrit.

use std::collections::HashMap;

use crate::ast::{self, Bloc, Cible, Contrat, Expr, ExprGenre, GenreContrat, Instr, Motif, OpBin, OpUn};
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

/// Où se trouve `t.i` dans les cases d'un tuple de type `t` : (début, longueur).
fn tranche(t: &Type, i: usize) -> (usize, usize) {
    let Type::Tuple(ts) = t else { unreachable!("tuple vérifié par le typage") };
    let debut = ts[..i].iter().map(|t| scalaires(t).len()).sum();
    (debut, scalaires(&ts[i]).len())
}

fn element(t: &Type, i: usize) -> Type {
    let Type::Tuple(ts) = t else { unreachable!("tuple vérifié par le typage") };
    ts[i].clone()
}

/// Les cases nécessaires pour une valeur de ce type, dans l'ordre.
pub fn scalaires(t: &Type) -> Vec<Scalaire> {
    match t {
        Type::Int | Type::Bool | Type::Char => vec![Scalaire::Entier],
        Type::Float => vec![Scalaire::Flottant],
        Type::Tuple(ts) => ts.iter().flat_map(scalaires).collect(),
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
    /// Contrat non respecté : écrit le message sur la sortie d'erreur et arrête (code 101).
    Echec(Vec<Morceau>),
}

/// Un morceau du message d'un contrat non respecté.
pub enum Morceau {
    Texte(String),
    Valeur(Local, Affichage),
    /// Le nom du fichier lancé (inconnu à la traduction : `kova lsp` traduit du texte).
    Fichier,
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

/// `contrats` = false pour `kova build --boost` : les `require`, `ensure` et `check` disparaissent.
pub fn traduire(f: &ast::Fichier, infos: &Infos, src: &str, contrats: bool) -> Programme {
    let numeros: HashMap<&str, usize> =
        f.fonctions.iter().enumerate().map(|(i, f)| (f.nom.texte.as_str(), i)).collect();
    // Les fonctions qui reçoivent le lieu de l'appel (`main` est appelée par le runtime).
    let sites: Vec<bool> =
        f.fonctions.iter().map(|g| contrats && !g.contrats.is_empty() && g.nom.texte != "main").collect();
    let fonctions = f
        .fonctions
        .iter()
        .enumerate()
        .map(|(i, fonc)| {
            let retour = fonc.retour.as_ref().map_or(Type::Rien, type_de);
            let gardes = |g: GenreContrat| fonc.contrats.iter().filter(move |c| contrats && c.genre == g);
            let mut t = Traducteur {
                src,
                infos,
                numeros: &numeros,
                sites: &sites,
                contrats,
                ensures: gardes(GenreContrat::Ensure).collect(),
                retour: retour.clone(),
                resultat: None,
                site: None,
                noms: vec![HashMap::new()],
                locaux: vec![],
                blocs: vec![],
                courant: 0,
                boucles: vec![],
            };
            for p in &fonc.params {
                let ty = type_de(&p.ty);
                let v = t.nouvelle_valeur(&ty);
                t.noms[0].insert(p.nom.texte.clone(), (v, ty));
            }
            t.site = sites[i].then(|| (t.entier(), t.entier()));
            let nb_params = t.locaux.len();
            t.courant = t.nouveau_bloc();
            for c in gardes(GenreContrat::Require) {
                t.verifier(c, t.site);
            }
            t.bloc(&fonc.corps);
            if retour == Type::Rien {
                t.sortir(vec![]);
            } else {
                t.terminer(Fin::Inatteignable);
            }
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
    /// Par numéro de fonction : reçoit-elle le lieu de l'appel ?
    sites: &'a [bool],
    /// Garder les contrats (faux avec `--boost`).
    contrats: bool,
    ensures: Vec<&'a Contrat>,
    retour: Type,
    /// La valeur de `result` pendant la vérification d'un `ensure`.
    resultat: Option<(Valeur, Type)>,
    /// Les 2 paramètres cachés : ligne et colonne de l'appel (voir `sites`).
    site: Option<(Local, Local)>,
    /// Nom → ses cases et son type, une table par bloc ouvert.
    noms: Vec<HashMap<String, (Valeur, Type)>>,
    locaux: Vec<Scalaire>,
    /// Blocs en construction ; `None` = fin pas encore posée.
    blocs: Vec<(Vec<Inst>, Option<Fin>)>,
    courant: NumBloc,
    /// Pour chaque boucle ouverte : (cible de `continue`, cible de `break`).
    boucles: Vec<(NumBloc, NumBloc)>,
}

impl<'a> Traducteur<'a> {
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
    fn variable(&self, nom: &str) -> (Valeur, Type) {
        self.noms.iter().rev().find_map(|t| t.get(nom)).expect("variable vérifiée par le typage").clone()
    }
    fn valeur_de(&self, nom: &str) -> Valeur {
        self.variable(nom).0
    }

    /// Donne leurs cases aux noms d'un motif : `(a, b)` sur 2 cases → `a` la 1re, `b` la 2e.
    fn lier(&mut self, m: &Motif, cases: &[Local], ty: &Type) {
        match m {
            Motif::Nom(n) => {
                self.noms.last_mut().unwrap().insert(n.texte.clone(), (cases.to_vec(), ty.clone()));
            }
            Motif::Ignorer(_) => {}
            Motif::Tuple(ms, _) => {
                for (i, m) in ms.iter().enumerate() {
                    let (debut, n) = tranche(ty, i);
                    self.lier(m, &cases[debut..debut + n], &element(ty, i));
                }
            }
        }
    }

    /// Les cases désignées par une cible de `:=`.
    fn cases_cible(&self, c: &Cible) -> Valeur {
        match c {
            Cible::Tuple(cs, _) => cs.iter().flat_map(|c| self.cases_cible(c)).collect(),
            Cible::Lieu { nom, champs, .. } => {
                let (mut cases, mut ty) = self.variable(&nom.texte);
                for &i in champs {
                    let (debut, n) = tranche(&ty, i);
                    cases = cases[debut..debut + n].to_vec();
                    ty = element(&ty, i);
                }
                cases
            }
        }
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
            Instr::Declarer { motif, valeur, .. } => {
                let v = self.expr(valeur);
                let ty = self.infos.types[valeur.id].clone();
                let cases = self.nouvelle_valeur(&ty);
                self.copier(&cases, &v);
                self.lier(motif, &cases, &ty);
            }
            Instr::Affecter { cible, op, valeur } => {
                let cases = self.cases_cible(cible);
                let v = self.expr(valeur);
                match op {
                    // `(a, b) := (b, a)` : on lit tout avant d'écrire, sinon `a` serait écrasé trop tôt.
                    None if matches!(cible, Cible::Tuple(..)) => {
                        let lu = self.nouvelle_valeur(&self.infos.types[valeur.id].clone());
                        self.copier(&lu, &v);
                        self.copier(&cases, &lu);
                    }
                    None => self.copier(&cases, &v),
                    Some(op) => {
                        let lieu = self.lieu(cible.span());
                        let r = self.operation(op_kir(*op), cases[0], v[0], self.est_float(valeur), lieu);
                        self.emettre(Inst::Copie(cases[0], r));
                    }
                }
            }
            Instr::Retour { valeur, .. } => {
                let v = valeur.as_ref().map_or(vec![], |e| self.expr(e));
                self.sortir(v);
            }
            Instr::Verifier(c) => {
                if self.contrats {
                    self.verifier(c, None);
                }
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
                self.noms.push(HashMap::from([(compteur.texte.clone(), (vec![i], Type::Int))]));
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

    /// `return v` : vérifie les `ensure` avec `result` = v, puis sort.
    fn sortir(&mut self, v: Valeur) {
        for c in self.ensures.clone() {
            self.resultat = Some((v.clone(), self.retour.clone()));
            self.verifier(c, self.site);
        }
        self.resultat = None;
        self.terminer(Fin::Retour(v));
    }

    /// Teste la condition d'un contrat ; si elle est fausse : `Echec` avec un message comme
    ///
    ///   erreur: on ne peut pas diviser par zéro
    ///     --> diviser.kova:2:13
    ///     require b != 0
    ///     valeurs : b = 0
    ///     appelé depuis diviser.kova:8:11
    fn verifier(&mut self, c: &Contrat, site: Option<(Local, Local)>) {
        let ok = self.expr1(&c.cond);
        let (echec, suite) = (self.nouveau_bloc(), self.nouveau_bloc());
        self.aller(Fin::Si(ok, suite, echec), echec);

        let l = self.lieu(c.cond.span);
        let cond: Vec<&str> = self.src[c.cond.span.debut..c.cond.span.fin].split_whitespace().collect();
        let mut m = vec![
            Morceau::Texte(format!("erreur: {}\n  --> ", c.message.as_deref().unwrap_or("contrat non respecté"))),
            Morceau::Fichier,
            Morceau::Texte(format!(":{}:{}\n  {} {}\n", l.ligne, l.colonne, c.genre.mot(), cond.join(" "))),
        ];
        let mut noms = vec![];
        noms_dans(&c.cond, &mut noms);
        let valeurs: Vec<(String, (Valeur, Type))> = noms
            .into_iter()
            .filter_map(|n| {
                let v = if n == "result" { self.resultat.clone() } else { self.noms.iter().rev().find_map(|t| t.get(&n)).cloned() };
                Some((n, v?))
            })
            .collect();
        for (k, (nom, (v, ty))) in valeurs.iter().enumerate() {
            m.push(Morceau::Texte(format!("{}{nom} = ", if k == 0 { "  valeurs : " } else { ", " })));
            morceaux_valeur(v, ty, &mut m);
        }
        if !valeurs.is_empty() {
            m.push(Morceau::Texte("\n".into()));
        }
        if let Some((ligne, colonne)) = site {
            m.extend([
                Morceau::Texte("  appelé depuis ".into()),
                Morceau::Fichier,
                Morceau::Texte(":".into()),
                Morceau::Valeur(ligne, Affichage::Int),
                Morceau::Texte(":".into()),
                Morceau::Valeur(colonne, Affichage::Int),
                Morceau::Texte("\n".into()),
            ]);
        }
        self.emettre(Inst::Echec(m));
        self.aller(Fin::Inatteignable, suite);
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
            ExprGenre::Resultat => self.resultat.clone().expect("`result` vérifié par le typage").0,
            ExprGenre::Tuple(elems) => elems.iter().flat_map(|x| self.expr(x)).collect(),
            ExprGenre::Champ(x, i) => {
                let v = self.expr(x);
                let (debut, n) = tranche(&self.infos.types[x.id], *i);
                v[debut..debut + n].to_vec()
            }
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
                if self.sites[num] {
                    let l = self.lieu(nom.span);
                    let (ligne, colonne) = (self.constante(l.ligne as i64), self.constante(l.colonne as i64));
                    vals.extend([ligne, colonne]);
                }
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

/// Les variables d'une condition, dans l'ordre, sans doublon (`result` compris).
fn noms_dans(e: &Expr, noms: &mut Vec<String>) {
    let mut ajouter = |n: &str| {
        if !noms.iter().any(|m| m == n) {
            noms.push(n.to_string());
        }
    };
    match &e.genre {
        ExprGenre::Var(n) => ajouter(n),
        ExprGenre::Resultat => ajouter("result"),
        ExprGenre::Appel { args, .. } | ExprGenre::Tuple(args) => args.iter().for_each(|a| noms_dans(a, noms)),
        ExprGenre::Methode { recu, args, .. } => {
            noms_dans(recu, noms);
            args.iter().for_each(|a| noms_dans(a, noms));
        }
        ExprGenre::Unaire(_, x) | ExprGenre::Champ(x, _) => noms_dans(x, noms),
        ExprGenre::Si { cond, .. } => noms_dans(cond, noms),
        ExprGenre::Binaire(_, a, b) => {
            noms_dans(a, noms);
            noms_dans(b, noms);
        }
        ExprGenre::Entier(_) | ExprGenre::Flottant(_) | ExprGenre::Caractere(_) | ExprGenre::Bool(_) => {}
    }
}

/// Une valeur comme on l'écrirait en Kova : `3`, `2.5`, `'a'`, `(1, true)`.
fn morceaux_valeur(v: &[Local], ty: &Type, m: &mut Vec<Morceau>) {
    match ty {
        Type::Tuple(ts) => {
            m.push(Morceau::Texte("(".into()));
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    m.push(Morceau::Texte(", ".into()));
                }
                let (debut, n) = tranche(ty, i);
                morceaux_valeur(&v[debut..debut + n], t, m);
            }
            m.push(Morceau::Texte(")".into()));
        }
        Type::Char => {
            m.push(Morceau::Texte("'".into()));
            m.push(Morceau::Valeur(v[0], Affichage::Char));
            m.push(Morceau::Texte("'".into()));
        }
        _ => m.push(Morceau::Valeur(v[0], Affichage::de(ty))),
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
