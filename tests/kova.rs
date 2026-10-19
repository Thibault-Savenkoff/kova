// Lanceur de tests de Kova (voir docs/PLAN.md, Partie 3, « Tests »).
//
// tests/ok/*.kova     : doit marcher en `run` ET en `build`, avec la même sortie.
//                       Sortie attendue : une ligne `// attendu: ...` par ligne affichée.
//                       Erreur à l'exécution : `// code: 101` et `// stderr: ...` (morceau du message).
//                       stderr doit être identique dans les deux modes.
// tests/erreur/*.kova : doit être refusé ; `// erreur: ...` doit apparaître dans le message.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const KOVA: &str = env!("CARGO_BIN_EXE_kova");

fn fichiers(dossier: &str) -> Vec<PathBuf> {
    let dossier = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(dossier);
    let Ok(entrees) = std::fs::read_dir(&dossier) else { return vec![] };
    let mut v: Vec<PathBuf> = entrees
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "kova"))
        .collect();
    v.sort();
    v
}

/// Les valeurs des commentaires `// <cle>: valeur`, dans l'ordre.
fn attentes(source: &str, cle: &str) -> Vec<String> {
    let prefixe = format!("// {cle}:");
    source
        .lines()
        .filter_map(|l| l.trim().strip_prefix(&prefixe))
        .map(|v| v.strip_prefix(' ').unwrap_or(v).to_string())
        .collect()
}

fn texte(o: &[u8]) -> String {
    String::from_utf8_lossy(o).into_owned()
}

struct Attendu {
    sortie: Vec<String>,
    code: i32,
    stderr: Vec<String>,
}

fn verifier_sortie(nom: &str, mode: &str, out: &Output, a: &Attendu, echecs: &mut Vec<String>) {
    let obtenu: Vec<String> = texte(&out.stdout).lines().map(String::from).collect();
    let stderr = texte(&out.stderr);
    if out.status.code() != Some(a.code) || obtenu != a.sortie || !a.stderr.iter().all(|m| stderr.contains(m.as_str())) {
        echecs.push(format!(
            "{nom} [{mode}] : code {:?} (attendu {})\n  attendu : {:?}\n  obtenu  : {obtenu:?}\n  stderr  : {}",
            out.status.code(),
            a.code,
            a.sortie,
            stderr.trim()
        ));
    }
}

#[test]
fn programmes_ok() {
    let tmp = std::env::temp_dir().join(format!("kova-tests-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut echecs = vec![];

    for f in fichiers("ok") {
        let nom = f.file_name().unwrap().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&f).unwrap();
        let attendu = Attendu {
            sortie: attentes(&source, "attendu"),
            code: attentes(&source, "code").first().map_or(0, |c| c.parse().unwrap()),
            stderr: attentes(&source, "stderr"),
        };

        // Mode run
        let run = Command::new(KOVA).arg("run").arg(&f).output().unwrap();
        verifier_sortie(&nom, "run", &run, &attendu, &mut echecs);

        // Mode build : compile, puis lance l'exécutable produit.
        let exe = tmp.join(nom.trim_end_matches(".kova"));
        let out = Command::new(KOVA).arg("build").arg(&f).arg("-o").arg(&exe).output().unwrap();
        if !out.status.success() {
            echecs.push(format!("{nom} [build] : compilation ratée\n  {}", texte(&out.stderr).trim()));
            continue;
        }
        let out = Command::new(&exe).output().unwrap();
        verifier_sortie(&nom, "build", &out, &attendu, &mut echecs);
        if out.stderr != run.stderr {
            echecs.push(format!(
                "{nom} : stderr différent\n  run   : {}\n  build : {}",
                texte(&run.stderr).trim(),
                texte(&out.stderr).trim()
            ));
        }
    }

    let _ = std::fs::remove_dir_all(&tmp);
    assert!(echecs.is_empty(), "\n{}\n", echecs.join("\n\n"));
}

#[test]
fn programmes_refuses() {
    let mut echecs = vec![];
    for f in fichiers("erreur") {
        let nom = f.file_name().unwrap().to_string_lossy().into_owned();
        let attendues = attentes(&std::fs::read_to_string(&f).unwrap(), "erreur");
        assert!(!attendues.is_empty(), "{nom} : il manque `// erreur: ...`");

        let out = Command::new(KOVA).arg("run").arg(&f).output().unwrap();
        let stderr = texte(&out.stderr);
        if out.status.success() {
            echecs.push(format!("{nom} : accepté alors qu'il devait être refusé"));
        } else if let Some(manque) = attendues.iter().find(|a| !stderr.contains(a.as_str())) {
            echecs.push(format!("{nom} : message attendu « {manque} »\n  obtenu : {}", stderr.trim()));
        }
    }
    assert!(echecs.is_empty(), "\n{}\n", echecs.join("\n\n"));
}

#[test]
fn ligne_de_commande() {
    let out = Command::new(KOVA).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(texte(&out.stdout).trim(), format!("kova {}", env!("CARGO_PKG_VERSION")));

    let out = Command::new(KOVA).arg("n'importe-quoi").output().unwrap();
    assert_eq!(out.status.code(), Some(2), "commande inconnue → usage + code 2");

    let out = Command::new(KOVA).args(["run", "fichier-qui-n-existe-pas.kova"]).output().unwrap();
    assert!(texte(&out.stderr).contains("impossible de lire"));

    assert_eq!(
        attentes("// attendu: 3\nx\n  // attendu:  a b\n// erreur: z", "attendu"),
        ["3", " a b"]
    );
}

/// Parle à `kova lsp` comme le fait l'éditeur : ouvre un fichier faux, reçoit l'erreur,
/// demande la complétion, puis arrête proprement le serveur.
#[test]
fn serveur_de_langage() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let mut serveur = Command::new(KOVA).arg("lsp").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut entree = serveur.stdin.take().unwrap();
    let mut sortie = BufReader::new(serveur.stdout.take().unwrap());
    let mut envoyer = |msg: serde_json::Value| {
        let texte = msg.to_string();
        write!(entree, "Content-Length: {}\r\n\r\n{texte}", texte.len()).unwrap();
        entree.flush().unwrap();
    };
    let mut recevoir = || -> serde_json::Value {
        let mut longueur = 0;
        loop {
            let mut ligne = String::new();
            sortie.read_line(&mut ligne).unwrap();
            if ligne == "\r\n" {
                break;
            }
            if let Some(n) = ligne.strip_prefix("Content-Length: ") {
                longueur = n.trim().parse().unwrap();
            }
        }
        let mut corps = vec![0; longueur];
        sortie.read_exact(&mut corps).unwrap();
        serde_json::from_slice(&corps).unwrap()
    };
    let uri = "file:///tmp/essai.kova";

    envoyer(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}));
    assert!(recevoir()["result"]["capabilities"]["completionProvider"].is_object());
    envoyer(serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));

    let texte = "fn main() {\n    let a: Int := 1\n    a := 2\n}\n";
    envoyer(serde_json::json!({"jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": uri, "languageId": "kova", "version": 1, "text": texte}}}));
    let diag = recevoir();
    assert_eq!(diag["method"], "textDocument/publishDiagnostics");
    let erreur = &diag["params"]["diagnostics"][0];
    assert!(erreur["message"].as_str().unwrap().contains("`a` est déclarée avec let"), "{diag}");
    assert_eq!(erreur["range"]["start"], serde_json::json!({"line": 2, "character": 4}));

    envoyer(serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "textDocument/completion",
        "params": {"textDocument": {"uri": uri}, "position": {"line": 2, "character": 4}}}));
    let reponse = recevoir();
    let labels: Vec<&str> = reponse["result"].as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"a") && labels.contains(&"main") && labels.contains(&"print"), "{labels:?}");

    envoyer(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown"}));
    assert_eq!(recevoir()["id"], 3);
    envoyer(serde_json::json!({"jsonrpc": "2.0", "method": "exit"}));
    assert!(serveur.wait().unwrap().success());
}
