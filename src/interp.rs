// Interpréteur (`kova run`) : exécute le KIR directement, instruction par instruction.

use std::io::{BufWriter, Stdout, Write};

use crate::kir::*;

/// Lance `main`. En cas d'erreur d'exécution : affiche le message et renvoie le code 101.
pub fn executer(p: &Programme, fichier: &str) -> i32 {
    let mut sortie = BufWriter::new(std::io::stdout());
    let resultat = appeler(p, p.principale, &[], &mut sortie, fichier);
    sortie.flush().unwrap();
    match resultat {
        Ok(_) => 0,
        Err(message) => {
            eprint!("{message}");
            101
        }
    }
}

/// `Err` = le message de l'erreur d'exécution, tel qu'il sera affiché.
type Resultat = Result<Vec<i64>, String>;

// ponytail: récursion Rust = la pile de l'interpréteur ; une récursion infinie
// plante sans joli message (pareil en natif). À traiter avec les erreurs de pile.
fn appeler(p: &Programme, num: usize, args: &[i64], sortie: &mut BufWriter<Stdout>, fichier: &str) -> Resultat {
    let panne = |p: Panne, lieu: &Lieu| texte_panne(p, fichier, *lieu);
    let f = &p.fonctions[num];
    // Toutes les cases tiennent dans 64 bits ; un Float y est rangé bit à bit.
    let mut locaux = vec![0i64; f.locaux.len()];
    locaux[..args.len()].copy_from_slice(args);
    let mut bloc = 0;
    loop {
        let b = &f.blocs[bloc];
        for inst in &b.instrs {
            match inst {
                Inst::Const(d, v) => locaux[*d] = *v,
                Inst::Copie(d, s) => locaux[*d] = locaux[*s],
                Inst::Binaire(d, op, a, b, lieu) => {
                    locaux[*d] = calculer(*op, locaux[*a], locaux[*b]).map_err(|e| panne(e, lieu))?;
                }
                Inst::Neg(d, a, lieu) => {
                    locaux[*d] = locaux[*a].checked_neg().ok_or_else(|| panne(Panne::Depassement, lieu))?;
                }
                Inst::Non(d, a) => locaux[*d] = 1 - locaux[*a],
                Inst::Appel(d, g, a) => {
                    let vals: Vec<i64> = a.iter().map(|&l| locaux[l]).collect();
                    let r = appeler(p, *g, &vals, sortie, fichier)?;
                    for (&d, v) in d.iter().zip(r) {
                        locaux[d] = v;
                    }
                }
                Inst::Flottant(d, op, a, b, lieu) => {
                    locaux[*d] = calculer_float(*op, float(locaux[*a]), float(locaux[*b])).map_err(|e| panne(e, lieu))?;
                }
                Inst::NegFlottant(d, a) => locaux[*d] = bits(-float(locaux[*a])),
                Inst::Convertir(d, c, a, lieu) => {
                    locaux[*d] = match c {
                        Conversion::EntierVersFlottant => bits(locaux[*a] as f64),
                        Conversion::FlottantVersEntier => {
                            let x = float(locaux[*a]);
                            // Faux pour NaN aussi : toute comparaison avec NaN est fausse.
                            if !(x >= -9223372036854775808.0 && x < 9223372036854775808.0) {
                                return Err(panne(Panne::ConversionImpossible, lieu));
                            }
                            x as i64
                        }
                    }
                }
                Inst::Afficher { valeur, genre, ligne } => {
                    let _ = write!(sortie, "{}", texte(locaux[*valeur], *genre));
                    if *ligne {
                        let _ = writeln!(sortie);
                    }
                }
                Inst::Echec(morceaux) => {
                    return Err(morceaux
                        .iter()
                        .map(|m| match m {
                            Morceau::Texte(t) => t.clone(),
                            Morceau::Valeur(l, genre) => texte(locaux[*l], *genre),
                            Morceau::Fichier => fichier.to_string(),
                        })
                        .collect());
                }
            }
        }
        match &b.fin {
            Fin::Saut(s) => bloc = *s,
            Fin::Si(c, oui, non) => bloc = if locaux[*c] != 0 { *oui } else { *non },
            Fin::Retour(v) => return Ok(v.iter().map(|&l| locaux[l]).collect()),
            Fin::Inatteignable => unreachable!("bloc inatteignable atteint dans `{}`", f.nom),
        }
    }
}

fn texte(v: i64, genre: Affichage) -> String {
    match genre {
        Affichage::Int => v.to_string(),
        Affichage::Bool => (v != 0).to_string(),
        Affichage::Float => texte_float(float(v)),
        Affichage::Char => char::from_u32(v as u32).unwrap().to_string(),
    }
}

fn float(v: i64) -> f64 {
    f64::from_bits(v as u64)
}
fn bits(x: f64) -> i64 {
    x.to_bits() as i64
}

/// Les opérations sur les Float ; les comparaisons donnent 0 ou 1.
fn calculer_float(op: Op, a: f64, b: f64) -> Result<i64, Panne> {
    Ok(match op {
        Op::Plus => bits(a + b),
        Op::Moins => bits(a - b),
        Op::Fois => bits(a * b),
        // `b == 0.0` est vrai aussi pour -0.0.
        Op::Divise if b == 0.0 => return Err(Panne::DivisionParZero),
        Op::Divise => bits(a / b),
        Op::Egal => (a == b) as i64,
        Op::Different => (a != b) as i64,
        Op::Inf => (a < b) as i64,
        Op::InfEgal => (a <= b) as i64,
        Op::Sup => (a > b) as i64,
        Op::SupEgal => (a >= b) as i64,
        Op::Reste => unreachable!("`%` refusé sur les Float par le typage"),
    })
}

/// Les opérations entières, avec les mêmes règles qu'en natif (`codegen.rs`).
fn calculer(op: Op, a: i64, b: i64) -> Result<i64, Panne> {
    let depasse = Panne::Depassement;
    Ok(match op {
        Op::Plus => a.checked_add(b).ok_or(depasse)?,
        Op::Moins => a.checked_sub(b).ok_or(depasse)?,
        Op::Fois => a.checked_mul(b).ok_or(depasse)?,
        Op::Divise if b == 0 => return Err(Panne::DivisionParZero),
        Op::Divise => a.checked_div(b).ok_or(depasse)?,
        Op::Reste if b == 0 => return Err(Panne::DivisionParZero),
        // -9223372036854775808 % -1 vaut 0 (mathématiquement juste, pas une erreur).
        Op::Reste => a.wrapping_rem(b),
        Op::Egal => (a == b) as i64,
        Op::Different => (a != b) as i64,
        Op::Inf => (a < b) as i64,
        Op::InfEgal => (a <= b) as i64,
        Op::Sup => (a > b) as i64,
        Op::SupEgal => (a >= b) as i64,
    })
}
