// Language server (`kova lsp`): the editor sends it the text, it answers with
// errors, completion and parameter help (LSP protocol).
//
//   editor ──"here is the text"──► kova lsp ──analyze()──► underlined errors
//
// Same lexer, same parser, same type checker as `kova run`: the editor cannot say
// anything different from the compiler.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use lsp_types::*;

use crate::ast::{Block, ExprKind, File, Function, Pattern, Stmt, TypeAst};
use crate::lexer::{Kind, lex};
use crate::locale::{self, t};
use crate::{analyze, parser, types};

pub fn serve() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions::default()),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".into(), ",".into()]),
            ..Default::default()
        }),
        ..Default::default()
    };
    connection.initialize(serde_json::to_value(capabilities)?)?;

    // The text of each open file, as in the editor (even unsaved).
    let mut docs: HashMap<Uri, String> = HashMap::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
                let response = match req.method.as_str() {
                    "textDocument/completion" => {
                        let p: CompletionParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position;
                        let items = docs.get(&doc.text_document.uri).map(|src| complete(src, offset(src, doc.position)));
                        Response::new_ok(req.id, items)
                    }
                    "textDocument/signatureHelp" => {
                        let p: SignatureHelpParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position_params;
                        let help = docs.get(&doc.text_document.uri).and_then(|src| signature(src, offset(src, doc.position)));
                        Response::new_ok(req.id, help)
                    }
                    _ => Response::new_err(req.id, ErrorCode::MethodNotFound as i32, t!("not_handled")),
                };
                connection.sender.send(response.into())?;
            }
            Message::Notification(n) => {
                let uri = match n.method.as_str() {
                    "textDocument/didOpen" => {
                        let p: DidOpenTextDocumentParams = serde_json::from_value(n.params)?;
                        docs.insert(p.text_document.uri.clone(), p.text_document.text);
                        p.text_document.uri
                    }
                    "textDocument/didChange" => {
                        // "FULL" sync: each change contains the whole text.
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
                let diagnostics = docs.get(&uri).map_or(vec![], |src| errors(src));
                let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
                connection.sender.send(Notification::new("textDocument/publishDiagnostics".into(), params).into())?;
            }
            Message::Response(_) => {}
        }
    }
    // The thread writing to the editor only stops once the connection is closed.
    drop(connection);
    io_threads.join()?;
    Ok(())
}

// ---- Positions: the editor counts in (line, UTF-16 character), the compiler in bytes ----

fn position(src: &str, byte: usize) -> Position {
    let before = &src[..byte.min(src.len())];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    Position {
        line: before.matches('\n').count() as u32,
        character: before[line_start..].encode_utf16().count() as u32,
    }
}

fn offset(src: &str, p: Position) -> usize {
    let line_start: usize = src.split_inclusive('\n').take(p.line as usize).map(str::len).sum();
    let mut units = 0;
    for (i, c) in src[line_start..].char_indices() {
        if units >= p.character as usize || c == '\n' {
            return line_start + i;
        }
        units += c.len_utf16();
    }
    src.len()
}

fn errors(src: &str) -> Vec<Diagnostic> {
    analyze(src)
        .2
        .into_iter()
        .map(|d| Diagnostic {
            range: Range::new(position(src, d.span.start), position(src, d.span.end)),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("kova".into()),
            message: match d.help {
                Some(help) => format!("{}\n{}{help}", d.message, t!("help_prefix")),
                None => d.message,
            },
            ..Default::default()
        })
        .collect()
}

// ---- Level 2 completion: variables and functions visible at this place ----

fn tree(src: &str) -> File {
    parser::parse(lex(src).0, src).0
}

/// `fn square(x: Int) -> Int`, and the position of each parameter in this text.
fn heading(f: &Function) -> (String, Vec<[u32; 2]>) {
    let mut text = format!("fn {}", f.name.text);
    let mut params = vec![];
    // `self` is not an argument at the call (`rex.rename(n)`): it gets no position.
    if let Some(r) = f.receiver {
        text += &format!("({}", r.text());
    }
    for (i, p) in f.params.iter().enumerate() {
        text += if i > 0 || f.receiver.is_some() { ", " } else { "(" };
        let start = text.encode_utf16().count() as u32;
        text += &format!("{}: {}", p.name.text, p.ty_text());
        params.push([start, text.encode_utf16().count() as u32]);
    }
    if !f.params.is_empty() || f.receiver.is_some() {
        text += ")";
    }
    if let Some(r) = &f.ret {
        text += &format!(" -> {}", r.text());
    }
    (text, params)
}

fn complete(src: &str, cursor: usize) -> Vec<CompletionItem> {
    let file = tree(src);
    if let Some(h) = &file.header
        && h.open.start < cursor
        && cursor <= h.close.start
    {
        return complete_header(&src[..cursor]);
    }
    let mut items: Vec<CompletionItem> = types::BUILTINS
        .iter()
        .map(|&name| CompletionItem {
            label: name.into(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(format!("fn {name}(value)")),
            ..Default::default()
        })
        .collect();
    for f in &file.functions {
        items.push(CompletionItem {
            label: f.name.text.clone(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(heading(f).0),
            ..Default::default()
        });
    }
    for st in &file.structs {
        let fields: Vec<String> = st.fields.iter().map(|f| format!("{}: {}", f.name.text, f.ty.text())).collect();
        items.push(CompletionItem {
            label: st.name.text.clone(),
            kind: Some(CompletionItemKind::STRUCT),
            detail: Some(format!("struct {} {{ {} }}", st.name.text, fields.join(", "))),
            ..Default::default()
        });
    }
    let inside = |b: &Block| b.open.start < cursor && cursor <= b.close.start;
    let bodies = file.functions.iter().map(|f| (None, &f.params[..], &f.body));
    let methods = file.structs.iter().flat_map(|s| s.methods.iter().map(move |m| (m.receiver.map(|_| &s.name.text), &m.params[..], &m.body)));
    let test_bodies = file.tests.iter().map(|t| (None, &[][..], &t.body));
    if let Some((owner, params, body)) = bodies.chain(methods).chain(test_bodies).find(|(_, _, b)| inside(b)) {
        let mut vars: Vec<(String, String)> = params.iter().map(|p| (p.name.text.clone(), p.ty_text())).collect();
        if let Some(ty) = owner {
            vars.insert(0, ("self".into(), ty.clone()));
        }
        visible(body, cursor, &mut vars);
        for (name, ty) in vars {
            items.push(CompletionItem {
                label: name,
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some(ty),
                ..Default::default()
            });
        }
    }
    items
}

/// Inside the `header`: the keys, or the values of the key after `:=`.
fn complete_header(before: &str) -> Vec<CompletionItem> {
    let line = before.rsplit('\n').next().unwrap_or("");
    if let Some((key, _)) = line.split_once(":=") {
        let Some((_, values, help)) = types::SETTINGS.iter().find(|(k, ..)| *k == key.trim()) else {
            return vec![];
        };
        return values
            .iter()
            .enumerate()
            .map(|(i, v)| CompletionItem {
                label: v.to_string(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(if i == 0 { t!("completion_default", help = locale::text(help, &[])) } else { locale::text(help, &[]) }),
                ..Default::default()
            })
            .collect();
    }
    types::SETTINGS
        .iter()
        .map(|(key, values, help)| CompletionItem {
            label: key.to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some(t!("completion_setting", values = values.join(" | "), help = locale::text(help, &[]))),
            insert_text: Some(format!("{key} := ")),
            ..Default::default()
        })
        .collect()
}

/// Adds to `vars` the variables of `b` visible at the cursor: declared before it,
/// in this block or in a block containing the cursor.
fn visible(b: &Block, cursor: usize, vars: &mut Vec<(String, String)>) {
    let inside = |b: &Block| b.open.start < cursor && cursor <= b.close.start;
    for s in &b.stmts {
        match s {
            // Visible only after its value: in `let x: Int := x + 1`, the 2nd `x` isn't it.
            Stmt::Declare { pattern, ty, value, .. } if value.span.end <= cursor => {
                // `let rex := Dog { ... }`: the type is the literal's.
                let written = match (&value.kind, ty) {
                    (ExprKind::StructLit { ty: name, .. }, None) => Some(TypeAst::Name(name.clone())),
                    _ => None,
                };
                pattern_names(pattern, ty.as_ref().or(written.as_ref()), vars);
            }
            Stmt::If { then, otherwise, .. } => {
                for b in std::iter::once(then).chain(otherwise).filter(|b| inside(b)) {
                    visible(b, cursor, vars);
                }
            }
            Stmt::While { body, .. } if inside(body) => visible(body, cursor, vars),
            Stmt::Loop { counter, body, .. } if inside(body) => {
                vars.push((counter.text.clone(), "Int".into()));
                visible(body, cursor, vars);
            }
            _ => {}
        }
    }
}

/// The names of a pattern with their written type: `let (a, b): (Int, Float)` → `a: Int`, `b: Float`.
fn pattern_names(p: &Pattern, ty: Option<&TypeAst>, vars: &mut Vec<(String, String)>) {
    match (p, ty) {
        (Pattern::Name(n), _) => vars.push((n.text.clone(), ty.map_or(String::new(), TypeAst::text))),
        (Pattern::Ignore(_), _) => {}
        (Pattern::Tuple(ps, _), Some(TypeAst::Tuple(ts))) if ps.len() == ts.len() => {
            for (p, t) in ps.iter().zip(ts) {
                pattern_names(p, Some(t), vars);
            }
        }
        (Pattern::Tuple(ps, _), _) => ps.iter().for_each(|p| pattern_names(p, None, vars)),
    }
}

// ---- Parameter help: the `square(x: Int) -> Int` bubble while typing the call ----

fn signature(src: &str, cursor: usize) -> Option<SignatureHelp> {
    // Walk back the tokens from the cursor to the open `(` of the current call,
    // counting the `,`: works even if the line is not finished.
    let tokens = lex(src).0;
    let before: Vec<_> = tokens.iter().take_while(|t| t.span.end <= cursor).collect();
    let (mut depth, mut commas) = (0, 0);
    let mut name = None;
    for (k, t) in before.iter().enumerate().rev() {
        match t.kind {
            Kind::Sym(")") => depth += 1,
            Kind::Sym("(") if depth > 0 => depth -= 1,
            Kind::Sym("(") => {
                name = before.get(k.wrapping_sub(1)).filter(|p| p.kind == Kind::Ident).map(|p| &src[p.span.start..p.span.end]);
                break;
            }
            Kind::Sym(",") if depth == 0 => commas += 1,
            Kind::Sym("{" | "}") | Kind::Newline => return None,
            _ => {}
        }
    }
    let name = name?;

    let (label, params) = if types::BUILTINS.contains(&name) {
        let label = format!("fn {name}(value)");
        let start = label.find('(').unwrap() as u32 + 1;
        (label, vec![[start, start + 5]])
    } else {
        let file = tree(src);
        let methods = file.structs.iter().flat_map(|s| &s.methods);
        heading(file.functions.iter().chain(methods).find(|f| f.name.text == name)?)
    };
    let parameters = params.into_iter().map(|p| ParameterInformation { label: ParameterLabel::LabelOffsets(p), documentation: None });
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: None,
            parameters: Some(parameters.collect()),
            active_parameter: Some(commas),
        }],
        active_signature: Some(0),
        active_parameter: Some(commas),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().filter(|i| i.kind == Some(CompletionItemKind::VARIABLE)).map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn utf16_positions() {
        let src = "fn carré {\n  é😀x\n}";
        let x = src.find('x').unwrap();
        assert_eq!(position(src, x), Position::new(1, 5)); // é = 1 unit, 😀 = 2
        assert_eq!(offset(src, Position::new(1, 5)), x);
        assert_eq!(offset(src, Position::new(1, 99)), src.find("x\n").unwrap() + 1);
    }

    #[test]
    fn completion_of_visible_variables() {
        let src = "fn f(n: Int) {\n  let a: Int := 1\n  if a = 1 {\n    let b: Bool := true\n    HERE\n  }\n  let c: Int := 2\n}\nfn main {}";
        let here = src.find("HERE").unwrap();
        assert_eq!(names(&complete(src, here)), ["n", "a", "b"]);
        // After the `if`: `b` is no longer visible, `c` not yet (we are before it).
        let after_if = src.find("  let c").unwrap();
        assert_eq!(names(&complete(src, after_if)), ["n", "a"]);
        // Functions are always offered.
        assert!(complete(src, here).iter().any(|i| i.label == "main"));
    }

    #[test]
    fn header_completion() {
        let src = "header {\n    mode := b\n    \n}\nfn main {}";
        let labels = |items: Vec<CompletionItem>| items.into_iter().map(|i| i.label).collect::<Vec<_>>();
        assert_eq!(labels(complete(src, src.find(" b").unwrap() + 2)), ["run", "build"]);
        assert_eq!(labels(complete(src, src.find("    \n").unwrap() + 4)), ["mode", "lock", "lang"]);
        // `}` not typed yet.
        let src = "header {\n  lock := ";
        assert_eq!(labels(complete(src, src.len())), ["false", "true"]);
    }

    #[test]
    fn completion_in_a_test() {
        let src = "test \"sample\" {\n  let a: Int := 1\n  check HERE\n}\nfn main {}";
        assert_eq!(names(&complete(src, src.find("HERE").unwrap())), ["a"]);
    }

    #[test]
    fn completion_while_typing() {
        // `}` not typed yet, statement unfinished: `total` is offered anyway.
        let src = "fn main {\n  var total: Int := 0\n  from i := 1 to 3 {\n    total := to";
        assert_eq!(names(&complete(src, src.len())), ["total", "i"]);
    }

    #[test]
    fn tuple_pattern_completion() {
        let src = "fn main {\n  let (a, (b, _)): (Int, (Float, Bool)) := (1, (2.0, true))\n  HERE\n}";
        let items = complete(src, src.find("HERE").unwrap());
        let vars: Vec<(&str, &str)> = items
            .iter()
            .filter(|i| i.kind == Some(CompletionItemKind::VARIABLE))
            .map(|i| (i.label.as_str(), i.detail.as_deref().unwrap()))
            .collect();
        assert_eq!(vars, [("a", "Int"), ("b", "Float")]);
    }

    #[test]
    fn parameter_help() {
        let src = "fn square(x: Int, y: Bool) -> Int { return x }\nfn main { print(square(1, ";
        let help = signature(src, src.len()).unwrap();
        assert_eq!(help.signatures[0].label, "fn square(x: Int, y: Bool) -> Int");
        assert_eq!(help.active_parameter, Some(1));
        let help = signature(src, src.len() - 4).unwrap(); // right after `square(`
        assert_eq!(help.active_parameter, Some(0));
        let help = signature(src, src.find("square(1").unwrap()).unwrap(); // inside `print(`
        assert_eq!(help.signatures[0].label, "fn print(value)");
    }
}
