// Language server (`kova lsp`): the editor sends it the text, it answers with
// errors, completion, parameter help, types on hover, definitions, renaming, quick fixes
// and test buttons (LSP protocol).
//
//   editor ──"here is the text"──► kova lsp ──analyze()──► underlined errors
//
// Same lexer, same parser, same type checker as `kova run`: the editor cannot say
// anything different from the compiler.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use lsp_types::*;

use crate::ast::{Block, Expr, ExprKind, File, Function, Part, Pattern, Stmt, Struct, TypeAst};
use crate::diag::Span;
use crate::types::Type;
use crate::lexer::{KEYWORDS, Kind, Token, lex};
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
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        rename_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
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
                // A message that cannot be read gets an error back; the server keeps going.
                let id = req.id.clone();
                let answer = || -> Result<Response, serde_json::Error> {
                    Ok(match req.method.as_str() {
                    "textDocument/completion" => {
                        let p: CompletionParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position;
                        let path = file_path(&doc.text_document.uri);
                        let items = docs.get(&doc.text_document.uri).map(|src| completion(&path, src, offset(src, doc.position)));
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
                    "textDocument/codeAction" => {
                        let p: CodeActionParams = serde_json::from_value(req.params)?;
                        let uri = &p.text_document.uri;
                        Response::new_ok(req.id, docs.get(uri).map(|src| add_cases(uri, src, p.range)))
                    }
                    "textDocument/rename" => {
                        let p: RenameParams = serde_json::from_value(req.params)?;
                        let doc = &p.text_document_position;
                        let uri = &doc.text_document.uri;
                        match docs.get(uri).map(|src| rename(uri, src, offset(src, doc.position), &p.new_name)) {
                            Some(Err(why)) => Response::new_err(req.id, ErrorCode::RequestFailed as i32, why),
                            edit => Response::new_ok(req.id, edit.and_then(Result::ok).flatten()),
                        }
                    }
                    // On save (the extension turns `editor.formatOnSave` on): `kova fmt`.
                    "textDocument/formatting" => {
                        let p: DocumentFormattingParams = serde_json::from_value(req.params)?;
                        Response::new_ok(req.id, docs.get(&p.text_document.uri).map(|src| layout(src)))
                    }
                    // Not in the LSP protocol: the extension asks it to put a ▶ before each `test`.
                    "kova/tests" => {
                        let uri: Uri = serde_json::from_value(req.params["textDocument"]["uri"].clone())?;
                        Response::new_ok(req.id, docs.get(&uri).map(|src| tests(src)))
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
                    })
                };
                let response = answer().unwrap_or_else(|e| Response::new_err(id, ErrorCode::InvalidParams as i32, e.to_string()));
                connection.sender.send(response.into())?;
            }
            Message::Notification(n) => {
                let uri = match n.method.as_str() {
                    "textDocument/didOpen" => {
                        let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(n.params) else { continue };
                        docs.insert(p.text_document.uri.clone(), p.text_document.text);
                        p.text_document.uri
                    }
                    "textDocument/didChange" => {
                        // "FULL" sync: each change contains the whole text.
                        let Ok(mut p) = serde_json::from_value::<DidChangeTextDocumentParams>(n.params) else { continue };
                        if let Some(c) = p.content_changes.pop() {
                            docs.insert(p.text_document.uri.clone(), c.text);
                        }
                        p.text_document.uri
                    }
                    "textDocument/didClose" => {
                        let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(n.params) else { continue };
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
    // Windows: `/c:/Users/…` → `c:/Users/…`.
    if bytes.len() > 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        bytes.remove(0);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `/home/me/my code/main.kova` → `file:///home/me/my%20code/main.kova` (the reverse of `file_path`).
fn file_uri(path: &str) -> Option<Uri> {
    let path = if cfg!(windows) { path.replace('\\', "/") } else { path.to_string() };
    let mut s = String::from("file://");
    // Windows: `c:/Users/…` → `file:///c%3A/Users/…`.
    if path.as_bytes().get(1) == Some(&b':') {
        s.push('/');
    }
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
/// `at` in `g.put(7) at(1)` or `fn put(v: Int) at(i: Int)`: the message is known by its first part.
fn first_part(src: &str, cursor: usize) -> usize {
    let tokens = lex(src).0;
    let Some(mut k) = tokens.iter().position(|t| t.span.start <= cursor && cursor < t.span.end && t.kind == Kind::Ident) else {
        return cursor;
    };
    while let Some((previous, _)) = previous_part(&tokens, k) {
        k = previous;
    }
    tokens[k].span.start
}

/// The part before the one named at `tokens[k]` (`put` before `at` in `put(7) at(`), with its
/// number of arguments, if `tokens[k - 1]` is its `)`.
fn previous_part(tokens: &[Token], k: usize) -> Option<(usize, u32)> {
    if k < 2 || tokens[k - 1].kind != Kind::Sym(")") {
        return None;
    }
    let (mut depth, mut args) = (0, 1);
    let open = (0..k - 1).rev().find(|&j| {
        match tokens[j].kind {
            Kind::Sym(")") => depth += 1,
            Kind::Sym("(") if depth == 0 => return true,
            Kind::Sym("(") => depth -= 1,
            Kind::Sym(",") if depth == 0 => args += 1,
            _ => {}
        }
        false
    })?;
    (open >= 1 && tokens[open - 1].kind == Kind::Ident).then_some((open - 1, args))
}

fn seen_at<'a>(info: &'a types::Info, src: &str, cursor: usize) -> Option<&'a types::Seen> {
    let under = |s: &Span| s.start <= cursor && cursor < s.end && s.end <= src.len();
    info.seen.iter().filter(|s| under(&s.span)).min_by_key(|s| s.span.end - s.span.start)
}

/// Hovering: what the name is (`rex: Dog`, `fn bark(self) -> String`), otherwise the type
/// of the smallest expression under the cursor (`a + b` → `Int`).
fn hover(path: &str, src: &str, cursor: usize) -> Option<Hover> {
    let (_, info, _) = info(path, src)?;
    let text = match seen_at(&info, src, first_part(src, cursor)) {
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
    location(uri, &sources, seen_at(&info, src, first_part(src, cursor))?.decl)
}

/// Where `span` is: in this file (`uri`) or in a file it imports.
fn location(uri: &Uri, sources: &diag::Sources, span: Span) -> Option<Location> {
    let k = sources.file_of(span.start);
    let (start, text) = (sources.files[k].1, sources.file_text(k));
    let uri = if k == 0 { uri.clone() } else { file_uri(&sources.files[k].0)? };
    Some(Location::new(uri, Range::new(position(text, span.start - start), position(text, span.end - start))))
}

/// F2: the new name everywhere the name under the cursor is used, in this file and the files it
/// imports. `Err`: why the new name is refused.
fn rename(uri: &Uri, src: &str, cursor: usize, new: &str) -> Result<Option<WorkspaceEdit>, String> {
    let Some((_, info, sources)) = info(&file_path(uri), src) else { return Ok(None) };
    let Some(seen) = seen_at(&info, src, cursor) else { return Ok(None) };
    // `geo.distance`: only `distance` changes.
    let written = &sources.text[seen.span.start..seen.span.end];
    let old = written.rsplit('.').next().unwrap_or(written);
    if old == "self" {
        return Err(t!("rename_self"));
    }
    let capital = |w: &str| w.starts_with(char::is_uppercase);
    let first = new.chars().next();
    if !new.chars().all(is_word) || first.is_none_or(|c| c.is_ascii_digit()) || capital(new) != capital(old) || KEYWORDS.contains(&new) {
        return Err(t!("rename_invalid", name = new));
    }
    let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
    let mut done = vec![];
    for s in info.seen.iter().filter(|s| s.decl == seen.decl) {
        let span = Span::new(s.span.end.saturating_sub(old.len()), s.span.end);
        if done.contains(&span) || sources.text.get(span.start..span.end) != Some(old) {
            continue;
        }
        done.push(span);
        if let Some(place) = location(uri, &sources, span) {
            changes.entry(place.uri).or_default().push(TextEdit::new(place.range, new.into()));
        }
    }
    Ok(Some(WorkspaceEdit { changes: Some(changes), ..Default::default() }))
}

/// The light bulb on a `match` that forgets cases: "Add the missing cases", each as `Case => {}`.
fn add_cases(uri: &Uri, src: &str, range: Range) -> Vec<CodeActionOrCommand> {
    let (from, to) = (offset(src, range.start), offset(src, range.end));
    let Some((_, info, _)) = info(&file_path(uri), src) else { return vec![] };
    let around = (info.matches.iter())
        .filter(|(s, cases)| !cases.is_empty() && s.end <= src.len() && s.start <= to && from <= s.end)
        .min_by_key(|(s, _)| s.end - s.start);
    let Some((span, cases)) = around else { return vec![] };
    let close = span.end - 1;
    let line = src[..close].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = src[line..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let arms: String = cases.iter().map(|c| format!("{indent}    {c} => {{}}\n")).collect();
    // The `}` alone on its line: the cases go above it; `match s { … }` on one line: before it.
    let (at, text) = if src[line..close].trim().is_empty() { (line, arms) } else { (close, format!("\n{arms}{indent}")) };
    let edit = TextEdit::new(Range::new(position(src, at), position(src, at)), text);
    vec![CodeActionOrCommand::CodeAction(CodeAction {
        title: t!("action_add_cases"),
        kind: Some(CodeActionKind::QUICKFIX),
        edit: Some(WorkspaceEdit { changes: Some(HashMap::from([(uri.clone(), vec![edit])])), ..Default::default() }),
        is_preferred: Some(true),
        ..Default::default()
    })]
}

/// The whole text laid out by `kova fmt`, as one change; none if it is already, or doesn't read.
fn layout(src: &str) -> Vec<TextEdit> {
    match crate::format::format(src) {
        Some(tidy) if tidy != src => vec![TextEdit::new(Range::new(Position::new(0, 0), position(src, src.len())), tidy)],
        _ => vec![],
    }
}

/// Each `test`: its name and where it is written. The extension puts a ▶ in the margin there,
/// which runs `kova test file "name"`.
fn tests(src: &str) -> Vec<serde_json::Value> {
    (tree(src).tests.iter())
        .map(|t| serde_json::json!({ "name": t.name, "range": Range::new(position(src, t.span.start), position(src, t.span.end)) }))
        .collect()
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
fn completion(path: &str, src: &str, cursor: usize) -> Vec<CompletionItem> {
    let mut items = (import_files(path, src, cursor))
        .or_else(|| members(path, src, cursor))
        .or_else(|| literal_fields(path, src, cursor))
        .or_else(|| match_cases(path, src, cursor))
        .or_else(|| promised(path, src, cursor))
        .unwrap_or_else(|| complete(src, cursor));
    parts_snippets(&mut items);
    items
}

/// A keyword message `put(…) at(…)` is offered as `put($1) at($2)`: the cursor goes to each argument.
fn parts_snippets(items: &mut [CompletionItem]) {
    for item in items.iter_mut().filter(|i| i.insert_text.is_none() && i.label.contains("(…)")) {
        // `put`, ` at`, ``: each piece after the first follows an argument.
        let pieces = item.label.split("(…)").enumerate();
        item.insert_text = Some(pieces.map(|(i, piece)| if i == 0 { piece.into() } else { format!("(${i}){piece}") }).collect());
        item.insert_text_format = Some(InsertTextFormat::SNIPPET);
        // No `(` is typed: the parameter bubble is asked for, it then follows the cursor.
        item.command = Some(Command::new(String::new(), "editor.action.triggerParameterHints".into(), None));
    }
}

fn members(path: &str, src: &str, cursor: usize) -> Option<Vec<CompletionItem>> {
    let dot = src[..cursor].trim_end_matches(is_word).strip_suffix('.')?.len();
    let before = &src[..dot];
    if let Some(items) = statics(path, src, &before[before.trim_end_matches(|c: char| is_word(c) || c == '.').len()..]) {
        return Some(items);
    }
    // `1.5` being typed: a number, not a method.
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

/// `geo.` → what the module `geometry` shows; `Dog.` or `geo.Shape.` → the type's static
/// functions and variants. `None` when `chain` (`geo`, `geo.Shape`, `rex.owner`) is a value.
fn statics(path: &str, src: &str, chain: &str) -> Option<Vec<CompletionItem>> {
    let parts: Vec<&str> = chain.split('.').collect();
    let module = tree(src).imports.into_iter().find(|i| i.alias.text == parts[0]).map(|i| i.module.text);
    let name = match (&module, parts.as_slice()) {
        (Some(m), [_]) => m.clone(),
        (Some(m), [_, ty]) if ty.starts_with(char::is_uppercase) => format!("{m}.{ty}"),
        (None, [ty]) if ty.starts_with(char::is_uppercase) => ty.to_string(),
        _ => return None,
    };
    let (file, _, _, _) = analyze(path, src);
    let hidden = |n: &str| file.private.iter().any(|(p, at)| p == n && at.start > src.len());
    let item = |label: &str, kind, detail: String| CompletionItem {
        label: label.into(),
        kind: Some(kind),
        detail: Some(detail),
        ..Default::default()
    };
    let mut items = vec![];
    if parts.len() == 1 && module.is_some() {
        // Its own names: `geometry.distance`, not the methods `geometry.Point.length`.
        let prefix = format!("{name}.");
        let own = |n: &str| n.strip_prefix(&prefix).is_some_and(|rest| !rest.contains('.')) && !hidden(n);
        for f in file.functions.iter().filter(|f| own(&f.name.text)) {
            items.push(item(&f.name.text[prefix.len()..], CompletionItemKind::FUNCTION, types::heading_text(f).replacen(&prefix, "", 1)));
        }
        for s in file.structs.iter().filter(|s| own(&s.name.text)) {
            let (kind, word) = if s.variants.is_some() { (CompletionItemKind::ENUM, "enum") } else { (CompletionItemKind::STRUCT, "struct") };
            items.push(item(&s.name.text[prefix.len()..], kind, format!("{word} {}", s.name.text)));
        }
        for i in file.interfaces.iter().filter(|i| own(&i.name.text)) {
            items.push(item(&i.name.text[prefix.len()..], CompletionItemKind::INTERFACE, format!("interface {}", i.name.text)));
        }
        return Some(items);
    }
    let s = file.structs.iter().find(|s| s.name.text == name)?;
    let statics = s.methods.iter().filter(|m| m.receiver.is_none() && !hidden(&types::method_name(&name, &m.name.text)));
    items.extend(statics.map(|m| item(&m.name.text, CompletionItemKind::FUNCTION, types::heading_text(m))));
    for v in s.variants.iter().flatten() {
        let fields: Vec<String> = v.fields.iter().map(|f| format!("{}: {}", f.name.text, f.ty_text())).collect();
        let shown = if fields.is_empty() { v.name.text.clone() } else { format!("{}({})", v.name.text, fields.join(", ")) };
        items.push(item(&v.name.text, CompletionItemKind::ENUM_MEMBER, shown));
    }
    Some(items)
}

/// `import ge|`: the `.kova` files next to this one, not imported yet.
fn import_files(path: &str, src: &str, cursor: usize) -> Option<Vec<CompletionItem>> {
    let line = src[..cursor].rsplit('\n').next().unwrap_or("");
    let typed = line.trim_start().strip_prefix("import ")?.trim_start();
    if !typed.chars().all(is_word) {
        return None;
    }
    let me = std::path::Path::new(path);
    let taken: Vec<String> = tree(src).imports.into_iter().map(|i| i.module.text).filter(|m| m != typed).collect();
    let files = std::fs::read_dir(me.parent()?).ok()?.flatten().map(|e| e.path());
    let mut names: Vec<String> = files
        .filter(|p| p.extension().is_some_and(|x| x == "kova") && p != me)
        .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
        .filter(|n| !taken.contains(n))
        .collect();
    names.sort();
    Some(names.into_iter().map(|n| CompletionItem { label: n, kind: Some(CompletionItemKind::FILE), ..Default::default() }).collect())
}

/// In `match shape {`, where a case goes: the cases not handled yet.
fn match_cases(path: &str, src: &str, cursor: usize) -> Option<Vec<CompletionItem>> {
    let (before, k) = opened(src, cursor)?;
    let brace = before[k].span.start;
    // Without the name being typed, the `match` reads.
    let start = src[..cursor].trim_end_matches(is_word).len();
    let end = src.len() - src[cursor..].trim_start_matches(is_word).len();
    let (_, info, _) = info(path, &format!("{}{}", &src[..start], &src[end..]))?;
    // The `match` whose `{` this is: the first `{` after its subject.
    let first_brace = |span: &Span| before.iter().find(|t| t.kind == Kind::Sym("{") && t.span.start > span.start).map(|t| t.span.start);
    let (_, cases) = info.matches.iter().find(|(span, _)| first_brace(span) == Some(brace))?;
    Some(
        cases
            .iter()
            .map(|c| CompletionItem {
                label: c.clone(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                insert_text: Some(format!("{c} => ")),
                ..Default::default()
            })
            .collect(),
    )
}

/// In `struct Dog : Animal {`, where a method goes (maybe after `fn`): the methods its interfaces
/// promise that it doesn't write yet, as a heading to fill.
fn promised(path: &str, src: &str, cursor: usize) -> Option<Vec<CompletionItem>> {
    let start = src[..cursor].trim_end_matches(is_word).len();
    let head = src[..start].trim_end();
    let after_fn = head.strip_suffix("fn").filter(|h| !h.ends_with(is_word) && head.len() < start);
    let (before, k) = opened(src, after_fn.map_or(cursor, str::len))?;
    // Back to `struct` on the same heading: `struct Dog<T> : Animal, Display {`.
    let at = (0..k).rev().take_while(|&j| !matches!(before[j].kind, Kind::Sym("{" | "}"))).find(|&j| before[j].kind == Kind::Keyword("struct"))?;
    let name = &src[before.get(at + 1)?.span.start..before[at + 1].span.end];
    let (file, _, _, _) = analyze(path, src);
    let st = file.structs.iter().find(|s| s.name.text == name)?;
    let mut items = vec![];
    for i in st.interfaces.iter().filter_map(|n| file.interfaces.iter().find(|i| i.name.text == n.text)) {
        for (m, _) in i.methods.iter().filter(|(m, _)| !st.methods.iter().any(|x| x.name.text == m.name.text)) {
            // `Self` is this type.
            let self_is = |w: &str| if w.trim_end_matches(|c: char| !is_word(c)) == "Self" { w.replacen("Self", name, 1) } else { w.to_string() };
            let heading: String = types::heading_text(m).split_inclusive(|c: char| !is_word(c)).map(self_is).collect();
            let insert = if after_fn.is_some() { &heading["fn ".len()..] } else { &heading };
            items.push(CompletionItem {
                label: m.name.text.clone(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some(heading.clone()),
                insert_text: Some(format!("{insert} {{\n\t$0\n}}")),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
    }
    Some(items)
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
        Type::List(e) => items.extend(list_methods(&e.text())),
        Type::Map(k, v) => {
            let (k, v) = (k.text(), v.text());
            let details = [("length", "-> Int".to_string()), ("contains", format!("(key: {k}) -> Bool")), ("remove", format!("(key: {k}) -> {v}?")), ("keys", format!("-> List<{k}>")), ("values", format!("-> List<{v}>"))];
            items.extend(details.into_iter().map(|(m, d)| item(m, CompletionItemKind::METHOD, d)));
        }
        Type::Set(e) => {
            let e = e.text();
            let details = [("length", "-> Int".to_string()), ("add", format!("(value: {e})")), ("contains", format!("(value: {e}) -> Bool")), ("remove", format!("(value: {e}) -> Bool"))];
            items.extend(details.into_iter().map(|(m, d)| item(m, CompletionItemKind::METHOD, d)));
        }
        _ => {}
    }
    if matches!(t, Type::Struct(_) | Type::Enum(_) | Type::Optional(_) | Type::Result(..) | Type::Tuple(_) | Type::Any(_) | Type::List(_) | Type::Map(..) | Type::Set(_)) {
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
    for name in types::NUMBER_METHODS {
        if let Some((params, result)) = types::number_method(t, name) {
            let params: Vec<String> = params.iter().map(Type::text).collect();
            let params = if params.is_empty() { String::new() } else { format!("({})", params.join(", ")) };
            items.push(item(name, CompletionItemKind::METHOD, format!("{params} -> {}", result.text())));
        }
    }
    items
}

/// The methods of a `List<T>` (`elem` = `T`): those taking a block write it, the cursor in its body.
fn list_methods(elem: &str) -> Vec<CompletionItem> {
    let block = |name: &str, detail: String, start: &str| CompletionItem {
        label: name.into(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(detail),
        insert_text: Some(format!("{name}{start} {{ x: {elem} => $0 }}")),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    };
    let plain = |name: &str, detail: String| CompletionItem {
        label: name.into(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(detail),
        ..Default::default()
    };
    let mut fold = block("fold", format!("(start: A) {{A, {elem} -> A}} -> A"), "");
    fold.insert_text = Some(format!("fold(${{1:0}}) {{ acc: ${{2:Int}}, x: {elem} => $0 }}"));
    vec![
        plain("length", "-> Int".into()),
        plain("add", format!("(x: move {elem})")),
        plain("remove", format!("(index: Int) -> {elem}")),
        block("each", format!("{{{elem}}}"), ""),
        block("filter", format!("{{{elem} -> Bool}} -> List<{elem}>"), ""),
        block("map", format!("{{{elem} -> U}} -> List<U>"), ""),
        block("any", format!("{{{elem} -> Bool}} -> Bool"), ""),
        block("all", format!("{{{elem} -> Bool}} -> Bool"), ""),
        fold,
        plain("sorted", format!("-> List<{elem}>  ·  sorted {{ a: {elem}, b: {elem} => a < b }}")),
    ]
}

/// The tokens before the cursor, without the name being typed, and the innermost `{` still open
/// (its place among them). `None` when no new name starts here (after `{`, `,` or a new line).
fn opened(src: &str, cursor: usize) -> Option<(Vec<Token>, usize)> {
    let mut before: Vec<Token> = lex(src).0.into_iter().take_while(|t| t.span.end <= cursor && t.kind != Kind::Eof).collect();
    if before.last().is_some_and(|t| t.span.end == cursor && matches!(t.kind, Kind::Ident | Kind::Type)) {
        before.pop();
    }
    if !matches!(before.last()?.kind, Kind::Sym("{" | ",") | Kind::Newline) {
        return None;
    }
    let mut depth = 0;
    for (k, t) in before.iter().enumerate().rev() {
        match t.kind {
            Kind::Sym("}" | ")" | "]") => depth += 1,
            Kind::Sym("{" | "(" | "[") if depth > 0 => depth -= 1,
            Kind::Sym("{") => return Some((before, k)),
            Kind::Sym("(" | "[") => return None,
            _ => {}
        }
    }
    None
}

/// `Dog { na|`: the fields of `Dog` not written yet. `None` when the cursor is not where a
/// field's name goes.
fn literal_fields(path: &str, src: &str, cursor: usize) -> Option<Vec<CompletionItem>> {
    let (before, k) = opened(src, cursor)?;
    let (mut depth, mut written) = (0, vec![]);
    for j in k + 1..before.len() {
        match before[j].kind {
            Kind::Sym("{" | "(" | "[") => depth += 1,
            Kind::Sym("}" | ")" | "]") => depth -= 1,
            Kind::Sym(":=") if depth == 0 && before[j - 1].kind == Kind::Ident => {
                written.push(&src[before[j - 1].span.start..before[j - 1].span.end]);
            }
            _ => {}
        }
    }
    struct_fields(path, src, &before[..k], &written)
}

/// The fields of the struct named by the last tokens of `before` (`Dog`, `geo.Point`), minus `written`.
fn struct_fields(path: &str, src: &str, before: &[Token], written: &[&str]) -> Option<Vec<CompletionItem>> {
    let text = |k: usize| &src[before[k].span.start..before[k].span.end];
    let k = before.len().checked_sub(1).filter(|&k| before[k].kind == Kind::Type)?;
    let (mut name, mut first) = (text(k).to_string(), k);
    if k >= 2 && before[k - 1].kind == Kind::Sym(".") && before[k - 2].kind == Kind::Ident {
        name = format!("{}.{name}", text(k - 2));
        first = k - 2;
    }
    // `struct Dog {` and `fn make -> Dog {` (or `-> Dog or Fault {`): not a value.
    if first > 0 && matches!(before[first - 1].kind, Kind::Keyword("struct" | "enum" | "or") | Kind::Sym("->")) {
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
    let mut text = String::from("fn");
    let mut params = vec![];
    // `fn put(value: Int) at(index: Int)`: each part writes its own parameters.
    for (i, (part, ps)) in f.parts().into_iter().enumerate() {
        text += &format!(" {part}");
        let mut open = false;
        if i == 0 {
            text += &crate::types::generics_text(f);
            // `self` is not an argument at the call (`rex.rename(n)`): it gets no position.
            if let Some(r) = f.receiver {
                text += &format!("({}", r.text());
                open = true;
            }
        }
        for p in ps {
            text += if open { ", " } else { "(" };
            open = true;
            let start = text.encode_utf16().count() as u32;
            text += &format!("{}: {}", p.name.text, p.ty_text());
            params.push([start, text.encode_utf16().count() as u32]);
        }
        if open {
            text += ")";
        }
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
    // The other keywords come with the snippets (`let`, `if`, `for`…).
    for word in ["true", "false", "none", "break", "continue", "else"] {
        items.push(CompletionItem { label: word.into(), kind: Some(CompletionItemKind::KEYWORD), ..Default::default() });
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
        // `scores.each { x: Int => … }`: inside a block written in this statement.
        let exprs: Vec<&Expr> = match s {
            Stmt::Declare { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr(value) => vec![value],
            Stmt::Return { value, .. } => value.iter().collect(),
            Stmt::Check(c) => vec![&c.cond],
            Stmt::If { cond, .. } | Stmt::While { cond, .. } => vec![cond],
            Stmt::Loop { start, end, .. } => vec![start, end],
            Stmt::Each { list, .. } => vec![list],
            Stmt::Match(m) => vec![&m.subject],
            Stmt::Break(_) | Stmt::Continue(_) => vec![],
        };
        for e in exprs {
            in_expr(e, cursor, vars);
        }
        match s {
            // Visible only after its value: in `let x: Int := x + 1`, the 2nd `x` isn't it.
            Stmt::Declare { pattern, ty, value, .. } if value.span.end < cursor => {
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

/// The variables visible at the cursor inside `e`: a block's parameters and what it declares,
/// or those of an `if` / `match` giving a value.
fn in_expr(e: &Expr, cursor: usize, vars: &mut Vec<(String, String)>) {
    if !(e.span.start < cursor && cursor <= e.span.end) {
        return;
    }
    let mut each = |es: &mut dyn Iterator<Item = &Expr>| es.for_each(|x| in_expr(x, cursor, vars));
    match &e.kind {
        ExprKind::Block { params, body } => {
            vars.extend(params.iter().map(|p| (p.name.text.clone(), p.ty.text())));
            visible(body, cursor, vars);
        }
        ExprKind::If { cond, then, otherwise } => {
            in_expr(cond, cursor, vars);
            for b in [then, otherwise].into_iter().filter(|b| b.open.start < cursor && cursor <= b.close.start) {
                visible(b, cursor, vars);
            }
        }
        ExprKind::Match(m) => {
            in_expr(&m.subject, cursor, vars);
            for arm in m.arms.iter().filter(|a| a.body.open.start <= cursor && cursor <= a.body.close.end) {
                if let Some((_, binds)) = arm.case.variant() {
                    binds.into_iter().for_each(|p| pattern_names(p, None, vars));
                }
                visible(&arm.body, cursor, vars);
            }
        }
        ExprKind::Call { args, .. } | ExprKind::Static { args, .. } | ExprKind::Variant { args, .. } | ExprKind::Tuple(args) | ExprKind::List(args) => {
            each(&mut args.iter())
        }
        ExprKind::Method { receiver, args, .. } => each(&mut std::iter::once(&**receiver).chain(args)),
        ExprKind::Mark(_, x) | ExprKind::Unary(_, x) | ExprKind::Field(x, _) | ExprKind::Try(x) | ExprKind::Outcome { value: x, .. } => {
            in_expr(x, cursor, vars)
        }
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => each(&mut [&**a, &**b].into_iter()),
        ExprKind::MapLit(pairs) => each(&mut pairs.iter().flat_map(|(k, v)| [k, v])),
        ExprKind::StructLit { fields, .. } => each(&mut fields.iter().map(|(_, v)| v)),
        ExprKind::Interp(parts) => each(&mut parts.iter().filter_map(|p| if let Part::Code(x) = p { Some(x) } else { None })),
        _ => {}
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
    let before: Vec<Token> = tokens.into_iter().take_while(|t| t.span.end <= cursor).collect();
    let (mut depth, mut commas) = (0, 0);
    let mut name = None;
    for (k, t) in before.iter().enumerate().rev() {
        match t.kind {
            Kind::Sym(")") => depth += 1,
            Kind::Sym("(") if depth > 0 => depth -= 1,
            Kind::Sym("(") => {
                name = before.get(k.wrapping_sub(1)).filter(|p| p.kind == Kind::Ident).map(|p| (k - 1, &src[p.span.start..p.span.end]));
                break;
            }
            Kind::Sym(",") if depth == 0 => commas += 1,
            Kind::Sym("{" | "}") | Kind::Newline => return None,
            _ => {}
        }
    }
    let (mut k, name) = name?;
    // `grid.put(3) at(`: the parts already written, each with its arguments.
    let mut parts = vec![name];
    while let Some((previous, args)) = previous_part(&before, k) {
        k = previous;
        parts.insert(0, &src[before[k].span.start..before[k].span.end]);
        commas += args;
    }

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
        // The function whose parts start with those written: `put` already shows `put(…) at(…)`.
        let starts = |f: &&Function| {
            let names: Vec<&str> = f.parts().into_iter().map(|(n, _)| n).collect();
            names.len() >= parts.len() && names[..parts.len()] == parts[..]
        };
        heading(file.functions.iter().chain(methods).filter(starts).min_by_key(|f| f.breaks.len())?)
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
    fn completion_inside_a_block() {
        let src = "fn main {\n  let l: List<Int> := [1]\n  var total: Int := 0\n  l.each { x: Int =>\n    let y: Int := x\n    HERE\n  }\n  let after: Int := 1\n}";
        let here = src.find("HERE").unwrap();
        assert_eq!(names(&complete(src, here)), ["l", "total", "x", "y"]);
        // While its value is typed, a variable is not offered to itself.
        let src = "fn main {\n  let a: Int := 1\n  let n: Int := a\n}";
        assert_eq!(names(&complete(src, src.find(":= a").unwrap() + 4)), ["a"]);
        // After `.` on a List: `filter` writes its block.
        let src = "fn main {\n  let l: List<Int> := [1]\n  l.\n}";
        let items = members("/none/main.kova", src, src.find("l.\n").unwrap() + 2).unwrap();
        let filter = items.iter().find(|i| i.label == "filter").unwrap();
        assert_eq!(filter.insert_text.as_deref(), Some("filter { x: Int => $0 }"));
        assert_eq!(filter.detail.as_deref(), Some("{Int -> Bool} -> List<Int>"));
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
        let windows = "c:/Users/me/é.kova";
        assert_eq!(file_uri(windows).unwrap().as_str(), "file:///c%3A/Users/me/%C3%A9.kova");
        assert_eq!(file_path(&file_uri(windows).unwrap()), windows);
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
        assert_eq!(at(&src, src.find("ages.").unwrap() + 5), ["length", "add", "remove", "each", "filter", "map", "any", "all", "fold", "sorted", "clone"]);
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
        let src = "struct Dog {\n  a: Int\n}\nfn make -> Dog {\n  ";
        assert!(literal_fields("/none/main.kova", src, src.len()).is_none());
        // A struct from another file, through its alias.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ok/modules/main.kova");
        let src = "import geometry as geo\nfn main {\n  let p := geo.Point { ";
        assert_eq!(labels(literal_fields(path, src, src.len())), ["x", "y"]);
    }

    const MODULES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ok/modules/main.kova");

    #[test]
    fn completion_of_statics_and_modules() {
        let at = |path: &str, src: &str| labels(members(path, src, src.len()));
        let geo = at(MODULES, "import geometry as geo\nfn main {\n  geo.");
        for name in ["distance", "Point", "Animal", "Box"] {
            assert!(geo.contains(&name.to_string()), "{name} in {geo:?}");
        }
        assert!(!geo.iter().any(|n| n == "helper" || n.contains('.')), "{geo:?}");
        assert!(at(MODULES, "import geometry as geo\nfn main {\n  geo.Point.").contains(&"origin".to_string()));
        let src = "enum Light { Red, Green }\nfn main {\n  let l := Light.";
        assert_eq!(at("/none/main.kova", src), ["Red", "Green"]);
    }

    #[test]
    fn completion_of_imports() {
        let src = "import geometry\nimport ";
        assert_eq!(labels(import_files(MODULES, src, src.len())), ["shapes"]);
        assert!(import_files(MODULES, "fn main {", 9).is_none());
    }

    const SHAPES: &str = "enum S { C(radius: Float), D, E }\nfn main {\n    let s := S.D\n    match s {\n        D => {}\n    }\n}\n";

    #[test]
    fn completion_of_match_cases() {
        let src = SHAPES.replace("D => {}\n", "D => {}\n        C");
        let items = match_cases("/none/main.kova", &src, src.find("        C").unwrap() + 9).unwrap();
        let shown: Vec<(&str, &str)> = items.iter().map(|i| (i.label.as_str(), i.insert_text.as_deref().unwrap())).collect();
        assert_eq!(shown, [("C(radius)", "C(radius) => "), ("E", "E => ")]);
        // In the body of a case: not a case.
        let src = SHAPES.replace("D => {}", "D => {\n        \n        }");
        assert!(match_cases("/none/main.kova", &src, src.find("{\n        \n").unwrap() + 10).is_none());
    }

    #[test]
    fn missing_cases_quick_fix() {
        let uri = file_uri("/none/main.kova").unwrap();
        let actions = add_cases(&uri, SHAPES, Range::new(Position::new(3, 10), Position::new(3, 10)));
        let [CodeActionOrCommand::CodeAction(a)] = actions.as_slice() else { panic!("{actions:?}") };
        let edit = &a.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
        assert_eq!(edit.new_text, "        C(radius) => {}\n        E => {}\n");
        assert_eq!(edit.range.start, Position::new(5, 0));
        // What it writes reads, on an enum and on an optional.
        let fixed = |src: &str, edit: &TextEdit| {
            let at = offset(src, edit.range.start);
            format!("{}{}{}", &src[..at], edit.new_text, &src[at..])
        };
        assert!(analyze("/none/main.kova", &fixed(SHAPES, edit)).2.is_empty());
        let optional = "fn main {\n    let n: Int? := 3\n    match n {\n    }\n}\n";
        let actions = add_cases(&uri, optional, Range::new(Position::new(2, 10), Position::new(2, 10)));
        let [CodeActionOrCommand::CodeAction(a)] = actions.as_slice() else { panic!("{actions:?}") };
        let edit = &a.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
        assert_eq!(edit.new_text, "        none => {}\n        value => {}\n");
        let diags = analyze("/none/main.kova", &fixed(optional, edit)).2;
        assert!(diags.is_empty(), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
        // All handled: no light bulb.
        let all = SHAPES.replace("D => {}", "_ => {}");
        assert!(add_cases(&uri, &all, Range::new(Position::new(3, 10), Position::new(3, 10))).is_empty());
    }

    #[test]
    fn completion_of_promised_methods() {
        let src = "interface A {\n  fn same(self, other: Self) -> Bool\n  fn done(self) -> Int { return 1 }\n}\nstruct Dog : A {\n  fn done(self) -> Int { return 2 }\n  ";
        let items = promised("/none/main.kova", src, src.len()).unwrap();
        let shown: Vec<(&str, &str)> = items.iter().map(|i| (i.label.as_str(), i.insert_text.as_deref().unwrap())).collect();
        assert_eq!(shown, [("same", "fn same(self, other: Dog) -> Bool {\n\t$0\n}")]);
        let src = format!("{src}fn sa");
        let items = promised("/none/main.kova", &src, src.len()).unwrap();
        assert_eq!(items[0].insert_text.as_deref(), Some("same(self, other: Dog) -> Bool {\n\t$0\n}"));
    }

    #[test]
    fn renaming() {
        let uri = file_uri("/none/main.kova").unwrap();
        let edits = |src: &str, at: &str, new: &str| {
            let edit = rename(&uri, src, src.find(at).unwrap(), new)?.unwrap();
            let mut ranges: Vec<_> = edit.changes.unwrap()[&uri].iter().map(|e| (e.range.start.line, e.range.start.character)).collect();
            ranges.sort();
            Ok::<_, String>(ranges)
        };
        assert_eq!(edits(DOGS, "rex :=", "max"), Ok(vec![(6, 6), (9, 14), (11, 8)]));
        assert_eq!(edits(DOGS, "age: Int", "years"), Ok(vec![(2, 2), (6, 34), (9, 18)]));
        assert_eq!(edits(DOGS, "Dog {\n", "Hound"), Ok(vec![(0, 7), (6, 13)]));
        // A variant: in its enum, where it is made, and in the `match` cases.
        assert_eq!(edits(SHAPES, "D, E", "Dot"), Ok(vec![(0, 27), (2, 15), (4, 8)]));
        assert!(edits(DOGS, "rex :=", "Max").is_err());
        assert!(edits(DOGS, "rex :=", "while").is_err());
        let uses_self = DOGS.replace("return \"woof\"", "return self.name");
        assert!(edits(&uses_self, "self.name", "me").is_err());
        // A function of another file: renamed there and at the call, `geo.` kept.
        let src = std::fs::read_to_string(MODULES).unwrap();
        let main = file_uri(MODULES).unwrap();
        let edit = rename(&main, &src, src.find("distance(a").unwrap(), "length").unwrap().unwrap();
        let changes = edit.changes.unwrap();
        assert_eq!(changes.len(), 2);
        let call = &changes[&main][0];
        assert_eq!(call.range.start, position(&src, src.find("distance(a").unwrap()));
        assert_eq!(call.range.end, position(&src, src.find("(a, o)").unwrap()));
    }

    #[test]
    fn formatting() {
        let edits = layout("fn main {\nprint( 1 )\n}");
        assert_eq!(edits[0].new_text, "fn main {\n    print(1)\n}\n");
        assert_eq!(edits[0].range.end, Position::new(2, 1));
        assert!(layout("fn main {\n    print(1)\n}\n").is_empty());
    }

    #[test]
    fn test_buttons() {
        let src = "test \"one\" {\n  check 1 = 1\n}\ntest \"two\" {\n}\n";
        let names: Vec<_> = tests(src).iter().map(|t| (t["range"]["start"]["line"].clone(), t["name"].clone())).collect();
        assert_eq!(names, [(serde_json::json!(0), serde_json::json!("one")), (serde_json::json!(3), serde_json::json!("two"))]);
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

    #[test]
    fn keyword_messages() {
        let src = "struct Grid {\n  n: Int\n  fn put(var self, value: Int) at(index: Int) {}\n}\nfn main {\n  var g: Grid := Grid { n := 1 }\n  g.put(1) at(";
        let help = signature(src, src.len()).unwrap();
        assert_eq!(help.signatures[0].label, "fn put(var self, value: Int) at(index: Int)");
        assert_eq!(help.active_parameter, Some(1));
        // While the first part is typed, the bubble already shows the whole message.
        let help = signature(src, src.find("put(1").unwrap() + 4).unwrap();
        assert_eq!(help.active_parameter, Some(0));
        // Hovering `at`, at the call or in the declaration, shows the whole message.
        let src = format!("{src}2)\n}}");
        let shown = |at: usize| match hover("/none/main.kova", &src, at).unwrap().contents {
            HoverContents::Markup(m) => m.value,
            _ => String::new(),
        };
        let heading = "```kova\nfn put(var self, value: Int) at(index: Int)\n```";
        assert_eq!(shown(src.rfind("at(2").unwrap()), heading);
        assert_eq!(shown(src.find("at(index").unwrap() + 1), heading);
        // Offered after `g.` and as a function, the cursor going to each argument.
        let src = "struct Grid {\n  n: Int\n  fn put(var self, value: Int) at(index: Int) {}\n}\nfn between(x: Int) upto(y: Int) {}\nfn main {\n  var g: Grid := Grid { n := 1 }\n  g.\n  \n}";
        let insert = |items: Vec<CompletionItem>, label: &str| items.into_iter().find(|i| i.label == label).and_then(|i| i.insert_text);
        let after_dot = completion("/none/main.kova", src, src.find("g.\n").unwrap() + 2);
        assert_eq!(insert(after_dot, "put(…) at(…)").as_deref(), Some("put($1) at($2)"));
        let blank = completion("/none/main.kova", src, src.rfind("  \n").unwrap() + 2);
        assert_eq!(insert(blank, "between(…) upto(…)").as_deref(), Some("between($1) upto($2)"));
    }
}
