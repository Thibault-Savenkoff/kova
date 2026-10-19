// Serveur de langage (`kova lsp`) : l'éditeur lui envoie le texte, il répond avec
// les erreurs, la complétion et l'aide sur les paramètres (protocole LSP).
//
//   éditeur ──« voici le texte »──► kova lsp ──analyser()──► erreurs soulignées
//
// Même lexer, même parser, même typage que `kova run` : l'éditeur ne peut pas dire
// autre chose que le compilateur.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use lsp_types::*;

use crate::ast::{Bloc, Fichier, Fonction, Instr, Motif, TypeAst};
use crate::lexer::{Genre, lexer};
use crate::{analyser, parser, types};

pub fn lancer() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connexion, fils) = Connection::stdio();
    let capacites = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions::default()),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".into(), ",".into()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    connexion.initialize(serde_json::to_value(capacites)?)?;

    // Le texte de chaque fichier ouvert, tel que dans l'éditeur (même pas sauvegardé).
    let mut docs: HashMap<Uri, String> = HashMap::new();
    for msg in &connexion.receiver {
        match msg {
            Message::Request(req) => {
                if connexion.handle_shutdown(&req)? {
                    break;
                }
                let reponse = match req.method.as_str() {
                    "textDocument/completion" => {
                        let p: CompletionParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position;
                        let items = docs.get(&doc.text_document.uri).map(|src| completer(src, decalage(src, doc.position)));
                        Response::new_ok(req.id, items)
                    }
                    "textDocument/signatureHelp" => {
                        let p: SignatureHelpParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position_params;
                        let aide = docs.get(&doc.text_document.uri).and_then(|src| signature(src, decalage(src, doc.position)));
                        Response::new_ok(req.id, aide)
                    }
                    _ => Response::new_err(req.id, ErrorCode::MethodNotFound as i32, "pas encore géré".into()),
                };
                connexion.sender.send(reponse.into())?;
            }
            Message::Notification(n) => {
                let uri = match n.method.as_str() {
                    "textDocument/didOpen" => {
                        let p: DidOpenTextDocumentParams = serde_json::from_value(n.params)?;
                        docs.insert(p.text_document.uri.clone(), p.text_document.text);
                        p.text_document.uri
                    }
                    "textDocument/didChange" => {
                        // Synchronisation « FULL » : chaque changement contient tout le texte.
                        let mut p: DidChangeTextDocumentParams = serde_json::from_value(n.params)?;
                        if let Some(c) = p.content_changes.pop() {
                            docs.insert(p.text_document.uri.clone(), c.text);
                        }
                        p.text_document.uri
                    }
                    "textDocument/didClose" => {
                        let p: DidCloseTextDocumentParams = serde_json::from_value(n.params)?;
                        docs.remove(&p.text_document.uri);
                        p.text_document.uri
                    }
                    _ => continue,
                };
                let diagnostics = docs.get(&uri).map_or(vec![], |src| erreurs(src));
                let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
                connexion.sender.send(Notification::new("textDocument/publishDiagnostics".into(), params).into())?;
            }
            Message::Response(_) => {}
        }
    }
    // Le fil qui écrit vers l'éditeur ne s'arrête qu'une fois la connexion fermée.
    drop(connexion);
    fils.join()?;
    Ok(())
}

// ---- Positions : l'éditeur compte en (ligne, caractère UTF-16), le compilateur en octets ----

fn position(src: &str, octet: usize) -> Position {
    let avant = &src[..octet.min(src.len())];
    let debut_ligne = avant.rfind('\n').map_or(0, |i| i + 1);
    Position {
        line: avant.matches('\n').count() as u32,
        character: avant[debut_ligne..].encode_utf16().count() as u32,
    }
}

fn decalage(src: &str, p: Position) -> usize {
    let debut_ligne: usize = src.split_inclusive('\n').take(p.line as usize).map(str::len).sum();
    let mut unites = 0;
    for (i, c) in src[debut_ligne..].char_indices() {
        if unites >= p.character as usize || c == '\n' {
            return debut_ligne + i;
        }
        unites += c.len_utf16();
    }
    src.len()
}

fn erreurs(src: &str) -> Vec<Diagnostic> {
    analyser(src)
        .2
        .into_iter()
        .map(|d| Diagnostic {
            range: Range::new(position(src, d.span.debut), position(src, d.span.fin)),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("kova".into()),
            message: match d.aide {
                Some(aide) => format!("{}\naide : {aide}", d.message),
                None => d.message,
            },
            ..Default::default()
        })
        .collect()
}

// ---- Complétion niveau 2 : variables et fonctions visibles à cet endroit ----

fn arbre(src: &str) -> Fichier {
    parser::parser(lexer(src).0, src).0
}

/// `fn carre(x: Int) -> Int`, et la position de chaque paramètre dans ce texte.
fn entete(f: &Fonction) -> (String, Vec<[u32; 2]>) {
    let mut texte = format!("fn {}(", f.nom.texte);
    let mut params = vec![];
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 {
            texte += ", ";
        }
        let debut = texte.encode_utf16().count() as u32;
        texte += &format!("{}: {}", p.nom.texte, p.ty.texte());
        params.push([debut, texte.encode_utf16().count() as u32]);
    }
    texte += ")";
    if let Some(r) = &f.retour {
        texte += &format!(" -> {}", r.texte());
    }
    (texte, params)
}

fn completer(src: &str, curseur: usize) -> Vec<CompletionItem> {
    let fichier = arbre(src);
    let mut items: Vec<CompletionItem> = types::PREDEFINIES
        .iter()
        .map(|&nom| CompletionItem {
            label: nom.into(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("fn {nom}(valeur)")),
            ..Default::default()
        })
        .collect();
    for f in &fichier.fonctions {
        items.push(CompletionItem {
            label: f.nom.texte.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(entete(f).0),
            ..Default::default()
        });
    }
    let dedans = |b: &Bloc| b.debut.debut < curseur && curseur <= b.fin.debut;
    let corps = fichier.fonctions.iter().map(|f| (&f.params[..], &f.corps));
    let corps_tests = fichier.tests.iter().map(|t| (&[][..], &t.corps));
    if let Some((params, corps)) = corps.chain(corps_tests).find(|(_, c)| dedans(c)) {
        let mut vars: Vec<(String, String)> = params.iter().map(|p| (p.nom.texte.clone(), p.ty.texte())).collect();
        visibles(corps, curseur, &mut vars);
        for (nom, ty) in vars {
            items.push(CompletionItem {
                label: nom,
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some(ty),
                ..Default::default()
            });
        }
    }
    items
}

/// Ajoute à `vars` les variables de `b` visibles au curseur : déclarées avant lui,
/// dans ce bloc ou dans un bloc qui contient le curseur.
fn visibles(b: &Bloc, curseur: usize, vars: &mut Vec<(String, String)>) {
    let dedans = |b: &Bloc| b.debut.debut < curseur && curseur <= b.fin.debut;
    for i in &b.instrs {
        match i {
            // Visible seulement après sa valeur : dans `let x: Int := x + 1`, le 2e `x` n'est pas lui.
            Instr::Declarer { motif, ty, valeur, .. } if valeur.span.fin <= curseur => {
                noms_motif(motif, ty.as_ref(), vars);
            }
            Instr::Si { alors, sinon, .. } => {
                for b in std::iter::once(alors).chain(sinon).filter(|b| dedans(b)) {
                    visibles(b, curseur, vars);
                }
            }
            Instr::TantQue { corps, .. } if dedans(corps) => visibles(corps, curseur, vars),
            Instr::Boucle { compteur, corps, .. } if dedans(corps) => {
                vars.push((compteur.texte.clone(), "Int".into()));
                visibles(corps, curseur, vars);
            }
            _ => {}
        }
    }
}

/// Les noms d'un motif avec leur type écrit : `let (a, b): (Int, Float)` → `a: Int`, `b: Float`.
fn noms_motif(m: &Motif, ty: Option<&TypeAst>, vars: &mut Vec<(String, String)>) {
    match (m, ty) {
        (Motif::Nom(n), _) => vars.push((n.texte.clone(), ty.map_or(String::new(), TypeAst::texte))),
        (Motif::Ignorer(_), _) => {}
        (Motif::Tuple(ms, _), Some(TypeAst::Tuple(ts))) if ms.len() == ts.len() => {
            for (m, t) in ms.iter().zip(ts) {
                noms_motif(m, Some(t), vars);
            }
        }
        (Motif::Tuple(ms, _), _) => ms.iter().for_each(|m| noms_motif(m, None, vars)),
    }
}

// ---- Aide sur les paramètres : la bulle `carre(x: Int) -> Int` pendant qu'on tape l'appel ----

fn signature(src: &str, curseur: usize) -> Option<SignatureHelp> {
    // On remonte les jetons depuis le curseur jusqu'à la `(` ouverte de l'appel en cours,
    // en comptant les `,` : ça marche même si la ligne n'est pas finie.
    let jetons = lexer(src).0;
    let avant: Vec<_> = jetons.iter().take_while(|j| j.span.fin <= curseur).collect();
    let (mut profondeur, mut virgules) = (0, 0);
    let mut nom = None;
    for (k, j) in avant.iter().enumerate().rev() {
        match j.genre {
            Genre::Sym(")") => profondeur += 1,
            Genre::Sym("(") if profondeur > 0 => profondeur -= 1,
            Genre::Sym("(") => {
                nom = avant.get(k.wrapping_sub(1)).filter(|p| p.genre == Genre::Ident).map(|p| &src[p.span.debut..p.span.fin]);
                break;
            }
            Genre::Sym(",") if profondeur == 0 => virgules += 1,
            Genre::Sym("{" | "}") | Genre::Ligne => return None,
            _ => {}
        }
    }
    let nom = nom?;

    let (label, params) = if types::PREDEFINIES.contains(&nom) {
        let label = format!("fn {nom}(valeur)");
        let debut = label.find('(').unwrap() as u32 + 1;
        (label, vec![[debut, debut + 6]])
    } else {
        entete(arbre(src).fonctions.iter().find(|f| f.nom.texte == nom)?)
    };
    let parameters = params.into_iter().map(|p| ParameterInformation { label: ParameterLabel::LabelOffsets(p), documentation: None });
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: None,
            parameters: Some(parameters.collect()),
            active_parameter: Some(virgules),
        }],
        active_signature: Some(0),
        active_parameter: Some(virgules),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noms(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().filter(|i| i.kind == Some(CompletionItemKind::VARIABLE)).map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn positions_utf16() {
        let src = "fn carré() {\n  é😀x\n}";
        let x = src.find('x').unwrap();
        assert_eq!(position(src, x), Position::new(1, 5)); // é = 1 unité, 😀 = 2
        assert_eq!(decalage(src, Position::new(1, 5)), x);
        assert_eq!(decalage(src, Position::new(1, 99)), src.find("x\n").unwrap() + 1);
    }

    #[test]
    fn completion_variables_visibles() {
        let src = "fn f(n: Int) {\n  let a: Int := 1\n  if a = 1 {\n    let b: Bool := true\n    ICI\n  }\n  let c: Int := 2\n}\nfn main() {}";
        let ici = src.find("ICI").unwrap();
        assert_eq!(noms(&completer(src, ici)), ["n", "a", "b"]);
        // Après le `if` : `b` n'est plus visible, `c` pas encore (on est avant).
        let apres_si = src.find("  let c").unwrap();
        assert_eq!(noms(&completer(src, apres_si)), ["n", "a"]);
        // Les fonctions sont toujours proposées.
        assert!(completer(src, ici).iter().any(|i| i.label == "main"));
    }

    #[test]
    fn completion_dans_un_test() {
        let src = "test \"essai\" {\n  let a: Int := 1\n  check ICI\n}\nfn main() {}";
        assert_eq!(noms(&completer(src, src.find("ICI").unwrap())), ["a"]);
    }

    #[test]
    fn completion_pendant_la_frappe() {
        // `}` pas encore tapée, instruction pas finie : on propose quand même `total`.
        let src = "fn main() {\n  var total: Int := 0\n  from i := 1 to 3 {\n    total := to";
        assert_eq!(noms(&completer(src, src.len())), ["total", "i"]);
    }

    #[test]
    fn completion_motif_tuple() {
        let src = "fn main() {\n  let (a, (b, _)): (Int, (Float, Bool)) := (1, (2.0, true))\n  ICI\n}";
        let items = completer(src, src.find("ICI").unwrap());
        let vars: Vec<(&str, &str)> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::VARIABLE))
            .map(|i| (i.label.as_str(), i.detail.as_deref().unwrap()))
            .collect();
        assert_eq!(vars, [("a", "Int"), ("b", "Float")]);
    }

    #[test]
    fn aide_parametres() {
        let src = "fn carre(x: Int, y: Bool) -> Int { return x }\nfn main() { print(carre(1, ";
        let aide = signature(src, src.len()).unwrap();
        assert_eq!(aide.signatures[0].label, "fn carre(x: Int, y: Bool) -> Int");
        assert_eq!(aide.active_parameter, Some(1));
        let aide = signature(src, src.len() - 4).unwrap(); // juste après `carre(`
        assert_eq!(aide.active_parameter, Some(0));
        let aide = signature(src, src.find("carre(1").unwrap()).unwrap(); // dans `print(`
        assert_eq!(aide.signatures[0].label, "fn print(valeur)");
    }
}
