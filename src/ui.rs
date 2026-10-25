// `import ui` (PLAN Part 4 "Native windows: `import ui`"): the module is Kova source made for
// each program, with its state and event types and its `view` / `update` functions written in.
// The checker, ownership, KIR and both backends then see ordinary Kova; the only primitive is
// `ui.show(title, text)`, which shows a view and waits for a click (`StrOp::UiShow`).

use crate::ast::{File, Mode};
use crate::diag::Diag;
use crate::locale::t;

/// The module's own names: a `view` or `update` function cannot take one (inside the module,
/// `text` is the module's).
const OWN: [&str; 7] = ["text", "button", "column", "row", "run", "group", "View"];

const SOURCE: &str = r#"import ui
IMPORTS
public:

struct View {
private:
    text: String
    events: List<EVENT>
}

fn text(s: String) -> View {
    return View { text := "T{s.length}:{s}", events := [] }
}

fn button(label: String, event: EVENT) -> View {
    return View { text := "B{label.length}:{label}", events := [event.clone] }
}

fn column(items: List<View>) -> View {
    return group("C", items)
}

fn row(items: List<View>) -> View {
    return group("R", items)
}

fn run(title: String, state: STATE) {
    var s: STATE := state.clone
    while true {
        let v: View := VIEW(s)
        let clicked: Int := ui.show(title, v.text)
        if clicked < 0 {
            return
        }
        UPDATE(var s, v.events[clicked].clone)
    }
}

private:

fn group(kind: String, items: List<View>) -> View {
    var out: String := "{kind}{items.length}:"
    var events: List<EVENT> := []
    for v in items {
        out := out + v.text
        for e in v.events {
            events.add(e.clone)
        }
    }
    return View { text := out, events := events }
}
"#;

/// The `ui` module for the program whose files are `files` (the root first), or the errors that
/// stop it, on the program's own lines. `None` when no file imports `ui`.
pub fn module(files: &[&File]) -> Option<Result<String, Vec<Diag>>> {
    let import = files.iter().flat_map(|f| &f.imports).find(|i| i.module.text == "ui")?;
    let runs: Vec<_> = files.iter().flat_map(|f| &f.ui_runs).collect();
    let Some(run) = runs.first() else {
        return Some(Err(vec![Diag::new(import.module.span, t!("ui_run_missing"))]));
    };
    let mut errors: Vec<Diag> = runs[1..].iter().map(|r| Diag::new(r.call, t!("ui_run_twice"))).collect();
    let find = |name: &str| files.iter().flat_map(|f| &f.functions).find(|f| f.name.text == name);
    for n in [&run.view, &run.update] {
        if OWN.contains(&n.text.as_str()) {
            errors.push(Diag::new(n.span, t!("ui_name_taken", name = n.text)));
        } else if find(&n.text).is_none() {
            errors.push(Diag::new(n.span, t!("function_unknown", name = n.text)));
        }
    }
    if !errors.is_empty() {
        return Some(Err(errors));
    }
    let (view, update) = (find(&run.view.text)?, find(&run.update.text)?);
    // `fn view(app: App) -> ui.View` and `fn update(app: var App, e: Event)`.
    let state = match (view.params.as_slice(), &view.ret) {
        ([p], Some(r)) if p.mode == Mode::Read && r.text() == "ui.View" && view.breaks.is_empty() => p.ty.text(),
        _ => return Some(Err(vec![Diag::new(run.view.span, t!("ui_view_shape", name = run.view.text))])),
    };
    let event = match update.params.as_slice() {
        [s, e] if s.mode == Mode::Var && s.ty.text() == state && e.mode == Mode::Read && update.ret.is_none() && update.breaks.is_empty() => e.ty.text(),
        _ => return Some(Err(vec![Diag::new(run.update.span, t!("ui_update_shape", name = run.update.text, state = state))])),
    };
    let mut imports = vec![];
    let (state, event) = (local(&state, &mut imports), local(&event, &mut imports));
    let imports: String = imports.iter().map(|m| format!("import {m}\n")).collect();
    let source = (SOURCE.replace("IMPORTS\n", &imports).replace("STATE", &state).replace("EVENT", &event))
        .replace("VIEW", &run.view.text)
        .replace("UPDATE", &run.update.text);
    Some(Ok(source))
}

/// A type as the module writes it: `shapes.circle.Event` becomes `circle.Event`, with
/// `import shapes.circle` added.
fn local(ty: &str, imports: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String, imports: &mut Vec<String>| {
        match word.rsplit_once('.') {
            Some((module, name)) => {
                let alias = module.rsplit('.').next().unwrap_or(module);
                if !imports.iter().any(|m| m == module) {
                    imports.push(module.to_string());
                }
                out.push_str(&format!("{alias}.{name}"));
            }
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in ty.chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            word.push(c);
        } else {
            flush(&mut word, &mut out, imports);
            out.push(c);
        }
    }
    flush(&mut word, &mut out, imports);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_types() {
        let mut imports = vec![];
        assert_eq!(super::local("List<shapes.circle.Event>", &mut imports), "List<circle.Event>");
        assert_eq!(super::local("App", &mut imports), "App");
        assert_eq!(imports, ["shapes.circle"]);
    }
}
