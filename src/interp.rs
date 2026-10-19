// Interpréteur (`kova run`) : exécute le KIR directement, instruction par instruction.

use std::io::{BufWriter, Stdout, Write};

use crate::kir::*;

/// Lance `main`. En cas d'erreur d'exécution : affiche le message et renvoie le code 101.
pub fn executer(p: &Programme, fichier: &str) -> i32 {
    let mut sortie = BufWriter::new(std::io::stdout());
    let resultat = appeler(p, p.principale, &[], &mut sortie);
    sortie.flush().unwrap();
    match resultat {
        Ok(_) => 0,
        Err((panne, lieu)) => {
            eprint!("{}", texte_panne(panne, fichier, lieu));
            101
        }
    }
}

type Resultat = Result<Vec<i64>, (Panne, Lieu)>;

// ponytail: récursion Rust = la pile de l'interpréteur ; une récursion infinie
// plante sans joli message (pareil en natif). À traiter avec les erreurs de pile.
fn appeler(p: &Programme, num: usize, args: &[i64], sortie: &mut BufWriter<Stdout>) -> Resultat {
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
                    locaux[*d] = calculer(*op, locaux[*a], locaux[*b]).map_err(|e| (e, *lieu))?;
                }
                Inst::Neg(d, a, lieu) => {
                    locaux[*d] = locaux[*a].checked_neg().ok_or((Panne::Depassement, *lieu))?;
                }
                Inst::Non(d, a) => locaux[*d] = 1 - locaux[*a],
                Inst::Appel(d, g, a) => {
                    let vals: Vec<i64> = a.iter().map(|&l| locaux[l]).collect();
                    let r = appeler(p, *g, &vals, sortie)?;
                    for (&d, v) in d.iter().zip(r) {
                        locaux[d] = v;
                    }
                }
                Inst::Afficher { valeur, booleen, ligne } => {
                    let v = locaux[*valeur];
                    let fin = if *ligne { "\n" } else { "" };
                    let _ = if *booleen {
                        write!(sortie, "{}{fin}", v != 0)
                    } else {
                        write!(sortie, "{v}{fin}")
                    };
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
