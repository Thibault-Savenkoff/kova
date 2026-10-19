// Parser : transforme la liste de jetons en arbre (AST).
//
// Il ne s'arrête pas à la première erreur : il la note, saute jusqu'à la fin de
// l'instruction, puis continue. On voit donc toutes les erreurs d'un coup.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{Genre, Jeton};

/// `Err(())` = une erreur a déjà été ajoutée à `diags` ; l'appelant se resynchronise.
type R<T> = Result<T, ()>;

/// Les opérateurs binaires, de la priorité la plus faible à la plus forte (PLAN 2.4).
/// `false` = non chaînable (`a < b < c` est refusé).
const NIVEAUX: &[(&[(&str, OpBin)], bool)] = &[
    (&[("||", OpBin::Ou)], true),
    (&[("&&", OpBin::Et)], true),
    (&[("=", OpBin::Egal), ("!=", OpBin::Different)], false),
    (&[("<", OpBin::Inf), ("<=", OpBin::InfEgal), (">", OpBin::Sup), (">=", OpBin::SupEgal)], false),
    (&[("..", OpBin::Intervalle), ("..=", OpBin::IntervalleInclus)], false),
    (&[("+", OpBin::Plus), ("-", OpBin::Moins)], true),
    (&[("*", OpBin::Fois), ("/", OpBin::Divise), ("%", OpBin::Reste)], true),
];

const AFFECTATIONS: &[(&str, Option<OpBin>)] = &[
    (":=", None),
    ("+=", Some(OpBin::Plus)),
    ("-=", Some(OpBin::Moins)),
    ("*=", Some(OpBin::Fois)),
    ("/=", Some(OpBin::Divise)),
];

/// Renvoie l'arbre, le nombre d'expressions (pour la table des types) et les erreurs.
pub fn parser(jetons: Vec<Jeton>, source: &str) -> (Fichier, usize, Vec<Diag>) {
    let mut p = Parser { j: jetons, pos: 0, src: source, diags: vec![], ids: 0 };
    let fichier = p.fichier();
    (fichier, p.ids, p.diags)
}

struct Parser<'a> {
    j: Vec<Jeton>,
    pos: usize,
    src: &'a str,
    diags: Vec<Diag>,
    ids: usize,
}

impl Parser<'_> {
    // ---- Outils ----

    fn jeton(&self) -> Jeton {
        self.j[self.pos]
    }
    fn genre(&self) -> Genre {
        self.jeton().genre
    }
    fn avancer(&mut self) -> Jeton {
        let j = self.jeton();
        if j.genre != Genre::Fin {
            self.pos += 1;
        }
        j
    }
    fn est_sym(&self, s: &str) -> bool {
        matches!(self.genre(), Genre::Sym(x) if x == s)
    }
    fn est_cle(&self, k: &str) -> bool {
        matches!(self.genre(), Genre::Cle(x) if x == k)
    }
    fn sauter_lignes(&mut self) {
        while self.genre() == Genre::Ligne {
            self.pos += 1;
        }
    }
    fn texte(&self, span: Span) -> String {
        self.src[span.debut..span.fin].to_string()
    }
    /// Comment nommer le jeton courant dans un message.
    fn decrire(&self) -> String {
        match self.genre() {
            Genre::Ligne => "une fin de ligne".into(),
            Genre::Fin => "la fin du fichier".into(),
            _ => format!("`{}`", self.texte(self.jeton().span)),
        }
    }
    fn erreur<T>(&mut self, span: Span, msg: impl Into<String>) -> R<T> {
        self.diags.push(Diag::new(span, msg));
        Err(())
    }
    fn attendre(&mut self, s: &str) -> R<Span> {
        if self.est_sym(s) {
            return Ok(self.avancer().span);
        }
        let msg = format!("`{s}` attendu, trouvé {}", self.decrire());
        self.erreur(self.jeton().span, msg)
    }
    fn expr(&mut self, span: Span, genre: ExprGenre) -> Expr {
        self.ids += 1;
        Expr { id: self.ids - 1, span, genre }
    }

    /// Un nom de variable ou de fonction : minuscule obligatoire.
    fn nom_valeur(&mut self, quoi: &str) -> R<Nom> {
        let j = self.jeton();
        match j.genre {
            Genre::Ident => {
                self.avancer();
                Ok(Nom { texte: self.texte(j.span), span: j.span })
            }
            Genre::Type => {
                let t = self.texte(j.span);
                let mut c = t.chars();
                let minuscule: String = c.next().unwrap().to_lowercase().chain(c).collect();
                self.diags.push(
                    Diag::new(j.span, format!("un nom de {quoi} commence par une minuscule"))
                        .aide(format!("écris `{minuscule}` (la majuscule est réservée aux types)")),
                );
                Err(())
            }
            Genre::Cle(k) => {
                let msg = format!("`{k}` est un mot réservé de Kova : il ne peut pas servir de nom");
                self.erreur(j.span, msg)
            }
            _ => {
                let msg = format!("nom de {quoi} attendu, trouvé {}", self.decrire());
                self.erreur(j.span, msg)
            }
        }
    }

    /// Un nom de type : majuscule obligatoire.
    fn nom_type(&mut self) -> R<Nom> {
        let j = self.jeton();
        match j.genre {
            Genre::Type => {
                self.avancer();
                Ok(Nom { texte: self.texte(j.span), span: j.span })
            }
            Genre::Ident => self.erreur(j.span, "un nom de type commence par une majuscule"),
            _ => {
                let msg = format!("type attendu, trouvé {}", self.decrire());
                self.erreur(j.span, msg)
            }
        }
    }

    /// `Int` ou `(Int, Float)`.
    fn type_(&mut self) -> R<TypeAst> {
        if !self.est_sym("(") {
            return Ok(TypeAst::Nom(self.nom_type()?));
        }
        let debut = self.avancer().span;
        let mut types = vec![self.type_()?];
        while self.est_sym(",") {
            self.avancer();
            types.push(self.type_()?);
        }
        let fin = self.attendre(")")?;
        if types.len() < 2 {
            return self.erreur(debut.jusqua(fin), "un tuple a au moins 2 éléments");
        }
        Ok(TypeAst::Tuple(types))
    }

    /// `x`, `_` ou `(a, b)`.
    fn motif(&mut self) -> R<Motif> {
        if self.genre() == Genre::Ident && self.texte(self.jeton().span) == "_" {
            return Ok(Motif::Ignorer(self.avancer().span));
        }
        if !self.est_sym("(") {
            return Ok(Motif::Nom(self.nom_valeur("variable")?));
        }
        let debut = self.avancer().span;
        let mut motifs = vec![self.motif()?];
        while self.est_sym(",") {
            self.avancer();
            motifs.push(self.motif()?);
        }
        let fin = self.attendre(")")?;
        if motifs.len() < 2 {
            return self.erreur(debut.jusqua(fin), "un tuple a au moins 2 éléments");
        }
        Ok(Motif::Tuple(motifs, debut.jusqua(fin)))
    }

    /// `x`, `t.0` ou `(a, b)` à gauche de `:=` : on relit l'expression déjà lue.
    fn cible(&mut self, e: Expr) -> R<Cible> {
        let span = e.span;
        match e.genre {
            ExprGenre::Var(nom) => Ok(Cible::Lieu { nom: Nom { texte: nom, span }, champs: vec![], span }),
            ExprGenre::Champ(base, i) => match self.cible(*base)? {
                Cible::Lieu { nom, mut champs, .. } => {
                    champs.push(i);
                    Ok(Cible::Lieu { nom, champs, span })
                }
                Cible::Tuple(..) => self.erreur(span, "on ne peut donner une valeur qu'à une variable"),
            },
            ExprGenre::Tuple(elems) => {
                let cibles = elems.into_iter().map(|e| self.cible(e)).collect::<R<Vec<_>>>()?;
                Ok(Cible::Tuple(cibles, span))
            }
            _ => self.erreur(span, "on ne peut donner une valeur qu'à une variable"),
        }
    }

    // ---- Fichier et fonctions ----

    fn fichier(&mut self) -> Fichier {
        let mut fonctions = vec![];
        loop {
            self.sauter_lignes();
            match self.genre() {
                Genre::Fin => break,
                Genre::Cle("fn") => match self.fonction() {
                    Ok(f) => fonctions.push(f),
                    Err(()) => self.aller_au_prochain_fn(),
                },
                _ => {
                    let msg = format!("`fn` attendu, trouvé {}", self.decrire());
                    let _: R<()> = self.erreur(self.jeton().span, msg);
                    self.aller_au_prochain_fn();
                }
            }
        }
        Fichier { fonctions }
    }

    fn aller_au_prochain_fn(&mut self) {
        self.avancer();
        while !matches!(self.genre(), Genre::Cle("fn") | Genre::Fin) {
            self.avancer();
        }
    }

    fn fonction(&mut self) -> R<Fonction> {
        self.avancer(); // fn
        let nom = self.nom_valeur("fonction")?;
        self.attendre("(")?;
        let mut params = vec![];
        while !self.est_sym(")") {
            let nom = self.nom_valeur("paramètre")?;
            self.attendre(":")?;
            params.push(Param { nom, ty: self.type_()? });
            if !self.est_sym(")") {
                self.attendre(",")?;
            }
        }
        self.avancer(); // )
        let retour = if self.est_sym("->") {
            self.avancer();
            Some(self.type_()?)
        } else {
            None
        };
        self.sauter_lignes();
        let corps = self.bloc()?;
        Ok(Fonction { nom, params, retour, corps })
    }

    // ---- Blocs et instructions ----

    fn bloc(&mut self) -> R<Bloc> {
        let debut = self.attendre("{")?;
        let mut instrs = vec![];
        loop {
            self.sauter_lignes();
            if self.est_sym("}") {
                break;
            }
            // `}` oubliée (souvent : en train de taper) : on garde ce qui a été lu.
            if matches!(self.genre(), Genre::Fin | Genre::Cle("fn")) {
                let span = self.jeton().span;
                self.diags.push(Diag::new(span, "`}` manquante à la fin du bloc"));
                return Ok(Bloc { debut, instrs, fin: span });
            }
            match self.instr() {
                Ok(i) => {
                    instrs.push(i);
                    if !matches!(self.genre(), Genre::Ligne | Genre::Sym("}") | Genre::Fin) {
                        let msg = format!("fin d'instruction attendue, trouvé {}", self.decrire());
                        let _: R<()> = self.erreur(self.jeton().span, msg);
                        self.fin_instruction();
                    }
                }
                Err(()) => self.fin_instruction(),
            }
        }
        let fin = self.avancer().span;
        Ok(Bloc { debut, instrs, fin })
    }

    /// Après une erreur : saute jusqu'à la fin de l'instruction (fin de ligne ou `}` du bloc).
    /// Seules les accolades comptent : le lexer n'émet pas de `Ligne` dans `( )` et `[ ]`.
    fn fin_instruction(&mut self) {
        let mut profondeur = 0;
        loop {
            match self.genre() {
                Genre::Fin => return,
                Genre::Ligne | Genre::Sym("}") if profondeur == 0 => return,
                Genre::Sym("{") => profondeur += 1,
                Genre::Sym("}") => profondeur -= 1,
                _ => {}
            }
            self.avancer();
        }
    }

    fn instr(&mut self) -> R<Instr> {
        let debut = self.jeton().span;
        match self.genre() {
            Genre::Cle(k @ ("let" | "var")) => {
                self.avancer();
                let motif = self.motif()?;
                let ty = if self.est_sym(":") {
                    self.avancer();
                    Some(self.type_()?)
                } else {
                    None
                };
                self.attendre(":=")?;
                Ok(Instr::Declarer { modifiable: k == "var", motif, ty, valeur: self.expression()? })
            }
            Genre::Cle("return") => {
                self.avancer();
                let valeur = if matches!(self.genre(), Genre::Ligne | Genre::Sym("}") | Genre::Fin) {
                    None
                } else {
                    Some(self.expression()?)
                };
                Ok(Instr::Retour { valeur, span: debut })
            }
            Genre::Cle("break") => Ok(Instr::Sortir(self.avancer().span)),
            Genre::Cle("continue") => Ok(Instr::Continuer(self.avancer().span)),
            Genre::Cle("if") => self.si(),
            Genre::Cle("while") => {
                self.avancer();
                let cond = self.expression()?;
                self.sauter_lignes();
                Ok(Instr::TantQue { cond, corps: self.bloc()? })
            }
            Genre::Cle("for") => {
                self.avancer();
                let compteur = self.nom_valeur("variable")?;
                if !self.est_cle("in") {
                    let msg = format!("`in` attendu, trouvé {}", self.decrire());
                    return self.erreur(self.jeton().span, msg);
                }
                self.avancer();
                let e = self.expression()?;
                let ExprGenre::Binaire(op @ (OpBin::Intervalle | OpBin::IntervalleInclus), debut, fin) = e.genre
                else {
                    self.diags.push(
                        Diag::new(e.span, "en J1, `for` ne parcourt que des intervalles")
                            .aide("exemple : `for i in 0..10` (sans 10) ou `for i in 0..=10` (avec 10)"),
                    );
                    return Err(());
                };
                self.sauter_lignes();
                let corps = self.bloc()?;
                Ok(Instr::Boucle { compteur, debut: *debut, fin: *fin, inclus: op == OpBin::IntervalleInclus, corps })
            }
            Genre::Cle("from") => {
                self.avancer();
                let compteur = self.nom_valeur("variable")?;
                self.attendre(":=")?;
                let debut = self.expression()?;
                if !self.est_cle("to") {
                    let msg = format!("`to` attendu, trouvé {}", self.decrire());
                    return self.erreur(self.jeton().span, msg);
                }
                self.avancer();
                let fin = self.expression()?;
                self.sauter_lignes();
                Ok(Instr::Boucle { compteur, debut, fin, inclus: true, corps: self.bloc()? })
            }
            _ => {
                let e = self.expression()?;
                let Some(&(_, op)) = AFFECTATIONS.iter().find(|(s, _)| self.est_sym(s)) else {
                    return Ok(Instr::Expr(e));
                };
                let cible = self.cible(e)?;
                self.avancer();
                Ok(Instr::Affecter { cible, op, valeur: self.expression()? })
            }
        }
    }

    fn si(&mut self) -> R<Instr> {
        self.avancer(); // if
        let cond = self.expression()?;
        self.sauter_lignes();
        let alors = self.bloc()?;
        // `else` peut être à la ligne suivante : on regarde après les fins de ligne.
        let avant = self.pos;
        self.sauter_lignes();
        if !self.est_cle("else") {
            self.pos = avant;
            return Ok(Instr::Si { cond, alors, sinon: None });
        }
        self.avancer();
        self.sauter_lignes();
        let sinon = if self.est_cle("if") {
            // `else if` = un bloc `else` qui contient juste un `if`.
            let debut = self.jeton().span;
            let si = self.si()?;
            Bloc { debut, instrs: vec![si], fin: self.j[self.pos - 1].span }
        } else {
            self.bloc()?
        };
        Ok(Instr::Si { cond, alors, sinon: Some(sinon) })
    }

    /// `if` dans une expression : le `else` est obligatoire (il faut une valeur dans tous les cas).
    fn si_expr(&mut self) -> R<Expr> {
        let debut = self.avancer().span; // if
        let cond = self.expression()?;
        self.sauter_lignes();
        let alors = self.bloc()?;
        self.sauter_lignes();
        if !self.est_cle("else") {
            self.diags.push(
                Diag::new(debut, "un `if` qui donne une valeur a besoin d'un `else`")
                    .aide("ajoute `else { ... }` : la valeur doit exister dans tous les cas"),
            );
            return Err(());
        }
        self.avancer();
        self.sauter_lignes();
        let sinon = if self.est_cle("if") {
            let debut = self.jeton().span;
            let e = self.si_expr()?;
            Bloc { debut, fin: Span::new(e.span.fin, e.span.fin), instrs: vec![Instr::Expr(e)] }
        } else {
            self.bloc()?
        };
        let span = debut.jusqua(sinon.fin);
        Ok(self.expr(span, ExprGenre::Si { cond: Box::new(cond), alors, sinon }))
    }

    // ---- Expressions ----

    fn expression(&mut self) -> R<Expr> {
        self.niveau(0)
    }

    fn niveau(&mut self, n: usize) -> R<Expr> {
        let Some(&(ops, chainable)) = NIVEAUX.get(n) else {
            return self.unaire();
        };
        let mut gauche = self.niveau(n + 1)?;
        let mut deja = false;
        while let Some(&(s, op)) = ops.iter().find(|(s, _)| self.est_sym(s)) {
            if deja && !chainable {
                let span = self.jeton().span;
                self.diags.push(
                    Diag::new(span, format!("`{s}` ne peut pas s'enchaîner"))
                        .aide("pour `a < b < c`, écris `a < b && b < c`"),
                );
                return Err(());
            }
            self.avancer();
            let droite = self.niveau(n + 1)?;
            let span = gauche.span.jusqua(droite.span);
            gauche = self.expr(span, ExprGenre::Binaire(op, Box::new(gauche), Box::new(droite)));
            deja = true;
        }
        Ok(gauche)
    }

    fn unaire(&mut self) -> R<Expr> {
        let op = match self.genre() {
            Genre::Sym("-") => OpUn::Moins,
            Genre::Sym("!") => OpUn::Non,
            _ => return self.primaire(),
        };
        let debut = self.avancer().span;
        let e = self.unaire()?;
        Ok(self.expr(debut.jusqua(e.span), ExprGenre::Unaire(op, Box::new(e))))
    }

    /// Un atome suivi de ses suffixes : `x.to_float()`.
    fn primaire(&mut self) -> R<Expr> {
        let mut e = self.atome()?;
        while self.est_sym(".") {
            self.avancer();
            if let Genre::Entier(i) = self.genre() {
                let fin = self.avancer().span;
                e = self.expr(e.span.jusqua(fin), ExprGenre::Champ(Box::new(e), i as usize));
                continue;
            }
            let nom = self.nom_valeur("méthode")?;
            self.attendre("(")?;
            let args = self.arguments()?;
            let fin = self.avancer().span;
            e = self.expr(e.span.jusqua(fin), ExprGenre::Methode { recu: Box::new(e), nom, args });
        }
        Ok(e)
    }

    /// Les arguments d'un appel, après la `(` ; s'arrête sur la `)` sans la consommer.
    fn arguments(&mut self) -> R<Vec<Expr>> {
        let mut args = vec![];
        while !self.est_sym(")") {
            args.push(self.expression()?);
            if !self.est_sym(")") {
                self.attendre(",")?;
            }
        }
        Ok(args)
    }

    fn atome(&mut self) -> R<Expr> {
        let j = self.jeton();
        let genre = match j.genre {
            Genre::Entier(v) => ExprGenre::Entier(v),
            Genre::Flottant(v) => ExprGenre::Flottant(v),
            Genre::Caractere(c) => ExprGenre::Caractere(c),
            Genre::Cle("true") => ExprGenre::Bool(true),
            Genre::Cle("false") => ExprGenre::Bool(false),
            Genre::Ident => {
                let nom = self.nom_valeur("variable")?;
                if !self.est_sym("(") {
                    return Ok(self.expr(nom.span, ExprGenre::Var(nom.texte)));
                }
                self.avancer();
                let args = self.arguments()?;
                let fin = self.avancer().span;
                return Ok(self.expr(nom.span.jusqua(fin), ExprGenre::Appel { nom, args }));
            }
            Genre::Cle("if") => return self.si_expr(),
            Genre::Sym("(") => {
                self.avancer();
                let e = self.expression()?;
                if !self.est_sym(",") {
                    let fin = self.attendre(")")?;
                    return Ok(Expr { span: j.span.jusqua(fin), ..e });
                }
                // `(a, b)` : un tuple.
                let mut elems = vec![e];
                while self.est_sym(",") {
                    self.avancer();
                    elems.push(self.expression()?);
                }
                let fin = self.attendre(")")?;
                return Ok(self.expr(j.span.jusqua(fin), ExprGenre::Tuple(elems)));
            }
            _ => {
                let msg = format!("expression attendue, trouvé {}", self.decrire());
                return self.erreur(j.span, msg);
            }
        };
        self.avancer();
        Ok(self.expr(j.span, genre))
    }
}
