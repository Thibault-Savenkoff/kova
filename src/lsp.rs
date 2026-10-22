// Language server (`kova lsp`): the editor sends it the text, it answers with
// errors, completion, parameter help, types on hover, definitions (LSP protocol).
//
//   editor ──"here is the text"──► kova lsp ──analyze()──► underlined errors
//
// Same lexer, same parser, same type checker as `kova run`: the editor cannot say
// anything different from the compiler.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use lsp_types::*;

use crate::ast::{Block, ExprKind, File, Function, Pattern, Stmt, Struct, TypeAst};
use crate::diag::Span;
use crate::types::Type;
use crate::lexer::{Kind, lex};
use crate::locale::{self, t};
use crate::{analyze, diag, parser, types};

pub fn serve() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions { trigger_characters: Some(vec![".".into()]), ..Default::default() }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
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
                        let path = file_path(&doc.text_document.uri);
                        let items = docs.get(&doc.text_document.uri).map(|src| {
                            let cursor = offset(src, doc.position);
                            (members(&path, src, cursor).or_else(|| literal_fields(&path, src, cursor))).unwrap_or_else(|| complete(src, cursor))
                        });
                        Response::new_ok(req.id, items)
                    }
                    "textDocument/hover" => {
                        let p: HoverParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position_params;
                        let path = file_path(&doc.text_document.uri);
                        let shown = docs.get(&doc.text_document.uri).and_then(|src| hover(&path, src, offset(src, doc.position)));
                        Response::new_ok(req.id, shown)
                    }
                    "textDocument/definition" => {
                        let p: GotoDefinitionParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position_params;
                        let uri = &doc.text_document.uri;
                        let place = docs.get(uri).and_then(|src| definition(uri, src, offset(src, doc.position)));
                        Response::new_ok(req.id, place.map(GotoDefinitionResponse::Scalar))
                    }
                    "textDocument/inlayHint" => {
                        let p: InlayHintParams = serde_json::from_value(req.params)?;
                        let path = file_path(&p.text_document.uri);
                        Response::new_ok(req.id, docs.get(&p.text_document.uri).map(|src| hints(&path, src)))
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
                let diagnostics = docs.get(&uri).map_or(vec![], |src| errors(&uri, src));
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

/// An error in an imported file is shown on the `import` line that brings it in
/// (or on the first line), with the place where it really is.
fn errors(uri: &Uri, src: &str) -> Vec<Diagnostic> {
    let (_, _, diags, sources) = analyze(&file_path(uri), src);
    diags
        .into_iter()
        .map(|d| {
            let mut message = match d.help {
                Some(help) => format!("{}\n{}{help}", d.message, t!("help_prefix")),
                None => d.message,
            };
            let n = diag::file_number(&sources.text, d.span.start);
            let span = if n == 0 {
                d.span
            } else {
                let path = &sources.files[n].0;
                let (line, col) = diag::line_col(&sources.text, d.span.start);
                message = t!("error_in_import", file = path, line = line, col = col, message = message);
                sources.origins[n].expect("an imported file")
            };
            Diagnostic {
                range: Range::new(position(src, span.start), position(src, span.end)),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("kova".into()),
                message,
                ..Default::default()
            }
        })
        .collect()
}

/// `file:///home/me/my%20code/main.kova` → `/home/me/my code/main.kova`.
fn file_path(uri: &Uri) -> String {
    let s = uri.as_str();
    let s = s.strip_prefix("file://").unwrap_or(s).as_bytes();
    let mut bytes = vec![];
    let mut i = 0;
    while i < s.len() {
        let hex = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (s[i], hex) {
            (b'%', Some(b)) => {
                bytes.push(b);
                i += 3;
            }
            (c, _) => {
                bytes.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `/home/me/my code/main.kova` → `file:///home/me/my%20code/main.kova` (the reverse of `file_path`).
fn file_uri(path: &str) -> Option<Uri> {
    let mut s = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(b as char);
        } else {
            s += &format!("%{b:02X}");
        }
    }
    s.parse().ok()
}

// ---- What the type checker saw: types on hover, definitions, types in grey ----

/// The type checker's table for this text (`None` while it doesn't read: a syntax error).
fn info(path: &str, src: &str) -> Option<(File, types::Info, diag::Sources)> {
    let (file, info, _, sources) = analyze(path, src);
    Some((file, info?, sources))
}

/// The name under the cursor, in this file (the imported files come after it in the text).
fn seen_at<'a>(info: &'a types::Info, src: &str, cursor: usize) -> Option<&'a types::Seen> {
    let under = |s: &Span| s.start <= cursor && cursor < s.end && s.end <= src.len();
    info.seen.iter().filter(|s| under(&s.span)).min_by_key(|s| s.span.end - s.span.start)
}

/// Hovering: what the name is (`rex: Dog`, `fn bark(self) -> String`), otherwise the type
/// of the smallest expression under the cursor (`a + b` → `Int`).
fn hover(path: &str, src: &str, cursor: usize) -> Option<Hover> {
    let (_, info, _) = info(path, src)?;
    let text = match seen_at(&info, src, cursor) {
        Some(s) => s.text.clone(),
        None => {
            let under = |s: &Span| s.start <= cursor && cursor < s.end && s.end <= src.len();
            let id = (0..info.spans.len()).filter(|&i| under(&info.spans[i])).min_by_key(|&i| info.spans[i].end - info.spans[i].start)?;
            match &info.types[id] {
                Type::Error | Type::Unit => return None,
                t => t.text(),
            }
        }
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: format!("```kova\n{text}\n```") }),
        range: None,
    })
}

/// Ctrl+click: where the name under the cursor is declared, maybe in an imported file.
fn definition(uri: &Uri, src: &str, cursor: usize) -> Option<Location> {
    let (_, info, sources) = info(&file_path(uri), src)?;
    let decl = seen_at(&info, src, cursor)?.decl;
    let k = sources.file_of(decl.start);
    let (start, text) = (sources.files[k].1, sources.file_text(k));
    let uri = if k == 0 { uri.clone() } else { file_uri(&sources.files[k].0)? };
    Some(Location::new(uri, Range::new(position(text, decl.start - start), position(text, decl.end - start))))
}

/// The types nobody wrote, in grey after the name: `for score in scores` → `score: Int`.
fn hints(path: &str, src: &str) -> Vec<InlayHint> {
    let Some((_, info, _)) = info(path, src) else { return vec![] };
    (info.hints.iter())
        .filter(|(s, t)| s.end <= src.len() && *t != Type::Error)
        .map(|(s, t)| InlayHint {
            position: position(src, s.end),
            label: InlayHintLabel::String(format!(": {}", t.text())),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: None,
            data: None,
        })
        .collect()
}

// ---- Level 3 completion: after `.`, and the fields of `Dog {` ----

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `rex.na|`: the fields and methods of `rex`'s type. `None` when the cursor is not after a `.`.
fn members(path: &str, src: &str, cursor: usize) -> Option<Vec<CompletionItem>> {
    let dot = src[..cursor].trim_end_matches(is_word).strip_suffix('.')?.len();
    // `1.5` being typed: a number, not a method.
    let before = &src[..dot];
    let digits = before.trim_end_matches(|c: char| c.is_ascii_digit());
    if digits.len() < before.len() && !digits.ends_with(is_word) && !digits.ends_with('.') {
        return Some(vec![]);
    }
    // The text without `.na`: `rex` alone reads (and only gives an error the editor won't show).
    let end = src.len() - src[cursor..].trim_start_matches(is_word).len();
    let cut = format!("{before}{}", &src[end..]);
    let Some((file, info, _)) = info(path, &cut) else { return Some(vec![]) };
    // `a + rex.`: the smallest expression ending at the `.` (`rex`, not `a + rex`).
    let id = (0..info.spans.len())
        .filter(|&i| info.spans[i].end == dot && info.spans[i].start < dot)
        .min_by_key(|&i| dot - info.spans[i].start);
    Some(id.map_or(vec![], |id| members_of(&file, &info.types[id], cut.len())))
}

/// The fields and methods of a value of type `t`. `root`: the length of the edited file, the
/// first in the text: what the other files hide is not offered.
fn members_of(file: &File, t: &Type, root: usize) -> Vec<CompletionItem> {
    let item = |label: &str, kind, detail: String| CompletionItem {
        label: label.into(),
        kind: Some(kind),
        detail: Some(detail),
        ..Default::default()
    };
    let owner = t.owner();
    let hidden = |name: &str| (file.private.iter()).any(|(n, s)| *n == types::method_name(&owner, name) && s.start > root);
    let mut items = vec![];
    if let Type::Struct(d) = t {
        let fields = d.fields.iter().filter(|(n, _)| !hidden(n));
        items.extend(fields.map(|(n, ft)| item(n, CompletionItemKind::FIELD, format!("{n}: {}", ft.text()))));
    }
    if let Type::Tuple(ts) = t {
        items.extend(ts.iter().enumerate().map(|(i, ft)| item(&i.to_string(), CompletionItemKind::FIELD, ft.text())));
    }
    let interface = |name: &str| file.interfaces.iter().find(|i| i.name.text == name);
    let method = |m: &Function| item(&m.name.text, CompletionItemKind::METHOD, types::heading_text(m));
    match t {
        Type::Struct(_) | Type::Enum(_) => {
            if let Some(s) = file.structs.iter().find(|s| s.name.text == owner) {
                let own = s.methods.iter().filter(|m| m.receiver.is_some() && !hidden(&m.name.text));
                items.extend(own.map(method));
                // The default methods of its interfaces it doesn't write itself.
                for i in s.interfaces.iter().filter_map(|n| interface(&n.text)) {
                    let defaults = i.methods.iter().filter(|(m, body)| *body && !s.methods.iter().any(|x| x.name.text == m.name.text));
                    items.extend(defaults.map(|(m, _)| method(m)));
                }
            }
        }
        Type::Any(i) => items.extend(interface(i).into_iter().flat_map(|i| &i.methods).map(|(m, _)| method(m))),
        Type::List(_) => items.extend(types::LIST_METHODS.iter().map(|m| item(m, CompletionItemKind::METHOD, String::new()))),
        Type::Map(..) => items.extend(types::MAP_METHODS.iter().map(|m| item(m, CompletionItemKind::METHOD, String::new()))),
        _ => {}
    }
    if matches!(t, Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::Any(_)) {
        items.push(item("clone", CompletionItemKind::METHOD, t.text()));
    }
    for (receiver, name, result, optional) in types::METHODS {
        if receiver == t {
            let result = if *optional { format!("{}?", result.text()) } else { result.text() };
            items.push(item(name, CompletionItemKind::METHOD, format!("-> {result}")));
        }
    }
    if *t == Type::String {
        for name in types::STRING_METHODS {
            let (params, result) = types::string_method(name).expect("a String method");
            let params: Vec<String> = params.iter().map(Type::text).collect();
            let params = if params.is_empty() { String::new() } else { format!("({})", params.join(", ")) };
            items.push(item(name, CompletionItemKind::METHOD, format!("{params} -> {}", result.text())));
        }
    }
    items
}

/// `Dog { na|`: the fields of `Dog` not written yet. `None` when the cursor is not where a
/// field's name goes.
fn literal_fields(path: &str, src: &str, cursor: usize) -> Option<Vec<CompletionItem>> {
    let tokens = lex(src).0;
    let mut before: Vec<_> = tokens.iter().take_while(|t| t.span.end <= cursor && t.kind != Kind::Eof).collect();
    // The name being typed.
    if before.last().is_some_and(|t| t.span.end == cursor && t.kind == Kind::Ident) {
        before.pop();
    }
    if !matches!(before.last()?.kind, Kind::Sym("{" | ",") | Kind::Newline) {
        return None;
    }
    let (mut depth, mut written) = (0, vec![]);
    for (k, t) in before.iter().enumerate().rev() {
        match t.kind {
            Kind::Sym("}" | ")" | "]") => depth += 1,
            Kind::Sym("{" | "(" | "[") if depth > 0 => depth -= 1,
            Kind::Sym(":=") if depth == 0 && k > 0 && before[k - 1].kind == Kind::Ident => {
                written.push(&src[before[k - 1].span.start..before[k - 1].span.end]);
            }
            Kind::Sym("{") => return struct_fields(path, src, &before[..k], &written),
            Kind::Sym("(" | "[") => return None,
            _ => {}
        }
    }
    None
}

/// The fields of the struct named by the last tokens of `before` (`Dog`, `geo.Point`), minus `written`.
fn struct_fields(path: &str, src: &str, before: &[&crate::lexer::Token], written: &[&str]) -> Option<Vec<CompletionItem>> {
    let text = |k: usize| &src[before[k].span.start..before[k].span.end];
    let k = before.len().checked_sub(1).filter(|&k| before[k].kind == Kind::Type)?;
    let (mut name, mut first) = (text(k).to_string(), k);
    if k >= 2 && before[k - 1].kind == Kind::Sym(".") && before[k - 2].kind == Kind::Ident {
        name = format!("{}.{name}", text(k - 2));
        first = k - 2;
    }
    // `struct Dog {`: a declaration, not a value.
    if first > 0 && matches!(before[first - 1].kind, Kind::Keyword("struct" | "enum")) {
        return None;
    }
    let (file, _, _, _) = analyze(path, src);
    // `geo.Point` → `geometry.Point`.
    if let Some((alias, rest)) = name.split_once('.')
        && let Some(imp) = file.imports.iter().find(|i| i.alias.text == alias)
    {
        name = format!("{}.{rest}", imp.module.text);
    }
    let s = file.structs.iter().find(|s| s.name.text == name && s.variants.is_none())?;
    let hidden = |n: &str| (file.private.iter()).any(|(p, at)| *p == types::method_name(&name, n) && at.start > src.len());
    let fields = s.fields.iter().filter(|f| !written.contains(&f.name.text.as_str()) && !hidden(&f.name.text));
    Some(
        fields
            .map(|f| CompletionItem {
                label: f.name.text.clone(),
                kind: Some(CompletionItemKind::FIELD),
                detail: Some(format!("{}: {}", f.name.text, f.ty_text())),
                insert_text: Some(format!("{} := ", f.name.text)),
                ..Default::default()
            })
            .collect(),
    )
}

// ---- Level 2 completion: variables and functions visible at this place ----

fn tree(src: &str) -> File {
    parser::parse(lex(src).0, src).0
}

/// `fn square(x: Int) -> Int`, and the position of each parameter in this text.
fn heading(f: &Function) -> (String, Vec<[u32; 2]>) {
    let mut text = format!("fn {}{}", f.name.text, crate::types::generics_text(f));
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
    for (name, text) in types::LIBRARY {
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(text.to_string()),
            ..Default::default()
        });
    }
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
            detail: Some(format!("struct {} {{ {} }}", written(st), fields.join(", "))),
            ..Default::default()
        });
    }
    let inside = |b: &Block| b.open.start < cursor && cursor <= b.close.start;
    let bodies = file.functions.iter().map(|f| (None, &f.params[..], &f.body));
    let methods = (file.structs.iter())
        .flat_map(|s| s.methods.iter().map(move |m| (m.receiver.map(|_| written(s)), &m.params[..], &m.body)));
    // In a default method, `self` is a `Self`.
    let defaults = (file.interfaces.iter())
        .flat_map(|i| i.methods.iter().map(|(m, _)| (m.receiver.map(|_| "Self".to_string()), &m.params[..], &m.body)));
    let test_bodies = file.tests.iter().map(|t| (None, &[][..], &t.body));
    if let Some((owner, params, body)) = bodies.chain(methods).chain(defaults).chain(test_bodies).find(|(_, _, b)| inside(b)) {
        let mut vars: Vec<(String, String)> = params.iter().map(|p| (p.name.text.clone(), p.ty_text())).collect();
        if let Some(ty) = owner {
            vars.insert(0, ("self".into(), ty));
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

/// `Stack<T>`: a struct's name with its type parameters.
fn written(s: &Struct) -> String {
    let names: Vec<&str> = s.generics.iter().map(|g| g.name.text.as_str()).collect();
    if names.is_empty() { s.name.text.clone() } else { format!("{}<{}>", s.name.text, names.join(", ")) }
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
                    (ExprKind::StructLit { ty: name, types, .. }, None) if !types.is_empty() => {
                        Some(TypeAst::Generic(name.clone(), types.clone()))
                    }
                    (ExprKind::StructLit { ty: name, .. } | ExprKind::Variant { ty: name, .. }, None) => {
                        Some(TypeAst::Name(name.clone()))
                    }
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
            Stmt::Each { name, value, body, .. } if inside(body) => {
                vars.push((name.text.clone(), String::new()));
                if let Some(v) = value {
                    vars.push((v.text.clone(), String::new()));
                }
                visible(body, cursor, vars);
            }
            Stmt::Match(m) => {
                for arm in m.arms.iter().filter(|a| inside(&a.body)) {
                    if let Some((_, binds)) = arm.case.variant() {
                        binds.into_iter().for_each(|p| pattern_names(p, None, vars));
                    }
                    visible(&arm.body, cursor, vars);
                }
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
    } else if let Some((_, text)) = types::LIBRARY.iter().find(|(n, _)| *n == name) {
        // `fn read_file(path: String) -> …`: the parameter is between the parentheses.
        let (open, close) = (text.find('(').map_or(0, |i| i + 1), text.find(')').unwrap_or(0));
        (text.to_string(), if close > open { vec![[open as u32, close as u32]] } else { vec![] })
    } else {
        let file = tree(src);
        let methods = file.structs.iter().flat_map(|s| &s.methods).chain(file.interfaces.iter().flat_map(|i| i.methods.iter().map(|(m, _)| m)));
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
    fn completion_in_a_default_method() {
        let src = "interface A {\n  fn s(self) -> Int\n  fn d(self, n: Int) -> Int {\n    HERE\n  }\n}\nfn main {}";
        assert_eq!(names(&complete(src, src.find("HERE").unwrap())), ["self", "n"]);
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
    fn match_completion() {
        let src = "enum S { C(r: Float), D }\nfn main {\n  let s := S.D\n  match s {\n    C(radius) => \n    D => {}\n  }\n}";
        assert_eq!(names(&complete(src, src.find("=> ").unwrap() + 3)), ["s", "radius"]);
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

    fn shown(src: &str, at: &str) -> Option<String> {
        let h = hover("/none/main.kova", src, src.find(at).unwrap())?;
        let HoverContents::Markup(m) = h.contents else { panic!() };
        Some(m.value.trim_start_matches("```kova\n").trim_end_matches("\n```").to_string())
    }

    const DOGS: &str = "struct Dog {\n  name: String\n  age: Int\n  fn bark(self) -> String { return \"woof\" }\n}\nfn main {\n  let rex := Dog { name := \"Rex\", age := 3 }\n  let ages: List<Int> := [1, 2]\n  for a in ages {\n    print(a + rex.age)\n  }\n  print(rex.bark)\n}";

    #[test]
    fn hover_types() {
        assert_eq!(shown(DOGS, "rex :=").as_deref(), Some("rex: Dog"));
        assert_eq!(shown(DOGS, "a + ").as_deref(), Some("a: Int"));
        assert_eq!(shown(DOGS, "+ rex").as_deref(), Some("Int")); // the sum
        assert_eq!(shown(DOGS, "age)").as_deref(), Some("age: Int"));
        assert_eq!(shown(DOGS, "bark)").as_deref(), Some("fn bark(self) -> String"));
        assert_eq!(shown(DOGS, "Dog {").as_deref(), Some("struct Dog"));
        // Still there with an error elsewhere.
        let broken = DOGS.replace("print(rex.bark)", "print(rex.color)");
        assert_eq!(shown(&broken, "rex :=").as_deref(), Some("rex: Dog"));
    }

    #[test]
    fn grey_types() {
        let labels: Vec<String> = hints("/none/main.kova", DOGS)
            .into_iter()
            .map(|h| match h.label {
                InlayHintLabel::String(s) => format!("{}:{s}", h.position.line),
                _ => panic!(),
            })
            .collect();
        assert_eq!(labels, ["8:: Int"]);
    }

    #[test]
    fn definition_in_another_file() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ok/modules/main.kova");
        let src = std::fs::read_to_string(path).unwrap();
        let uri = file_uri(path).unwrap();
        let place = definition(&uri, &src, src.find("distance(a").unwrap()).unwrap();
        assert!(place.uri.as_str().ends_with("/tests/ok/modules/geometry.kova"));
        let geometry = std::fs::read_to_string(path.replace("main.kova", "geometry.kova")).unwrap();
        let line = geometry.lines().nth(place.range.start.line as usize).unwrap();
        assert!(line.contains("fn distance"), "{line}");
        // A variable: its `let`.
        let place = definition(&uri, &src, src.find("a, o)").unwrap()).unwrap();
        assert_eq!(place.uri, uri);
        assert_eq!(place.range.start, position(&src, src.find("a := geo").unwrap()));
    }

    #[test]
    fn uri_round_trip() {
        let path = "/home/me/my code/é.kova";
        assert_eq!(file_uri(path).unwrap().as_str(), "file:///home/me/my%20code/%C3%A9.kova");
        assert_eq!(file_path(&file_uri(path).unwrap()), path);
    }

    fn labels(items: Option<Vec<CompletionItem>>) -> Vec<String> {
        items.unwrap().into_iter().map(|i| i.label).collect()
    }

    #[test]
    fn completion_after_a_dot() {
        let at = |src: &str, cursor: usize| labels(members("/none/main.kova", src, cursor));
        let src = DOGS.replace("print(rex.bark)", "print(rex.ba)");
        assert_eq!(at(&src, src.find("ba)").unwrap() + 2), ["name", "age", "bark", "clone"]);
        // Right after the `.`, and on a List.
        let src = DOGS.replace("print(rex.bark)", "ages.");
        assert_eq!(at(&src, src.find("ages.").unwrap() + 5), ["length", "add", "remove", "clone"]);
        let src = "fn main {\n  let s: String := \"a\"\n  print(s.)\n}";
        let found = at(src, src.find(".)").unwrap() + 1);
        assert!(found.contains(&"trim".to_string()) && found.contains(&"to_int".to_string()), "{found:?}");
        // `1.5` being typed; not after a dot.
        let src = "fn main {\n  let x: Float := 1.\n}";
        assert!(at(src, src.find("1.").unwrap() + 2).is_empty());
        assert!(members("/none/main.kova", DOGS, DOGS.find("rex :=").unwrap() + 2).is_none());
    }

    #[test]
    fn completion_of_literal_fields() {
        let at = |src: &str| labels(literal_fields("/none/main.kova", src, src.len()));
        let src = DOGS.replace("{ name := \"Rex\", age := 3 }", "{ name := \"Rex\", ");
        let src = &src[..src.find("Rex\", ").unwrap() + 6];
        assert_eq!(at(src), ["age"]);
        assert_eq!(at(&format!("{src}a")), ["age"]);
        // While typing a value, or a declaration: not fields.
        assert!(literal_fields("/none/main.kova", src, src.find("Rex").unwrap()).is_none());
        assert!(literal_fields("/none/main.kova", "struct Dog {\n  ", 15).is_none());
        // A struct from another file, through its alias.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ok/modules/main.kova");
        let src = "import geometry as geo\nfn main {\n  let p := geo.Point { ";
        assert_eq!(labels(literal_fields(path, src, src.len())), ["x", "y"]);
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
