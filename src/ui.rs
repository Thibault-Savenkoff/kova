// `import ui` (PLAN Part 4 "Native windows: `import ui`"): the module is Kova source made for
// each program, with its state and event types and its `view` / `update` functions written in.
// The checker, ownership, KIR and both backends then see ordinary Kova; the only primitive is
// `ui.show(title, text, width, height)`, which shows a view and waits for a click or a change
// (`StrOp::UiShow`; a size of 0 = fit the content), and `ui.typed`, the text a field now holds.

use crate::ast::{File, Mode};
use crate::diag::Diag;
use crate::locale::t;

/// The module's own names: a `view` or `update` function cannot take one (inside the module,
/// `text` is the module's).
const OWN: [&str; 19] = [
    "text", "button", "input", "column", "row", "run", "group", "View", "Color", "rgb", "red", "green", "blue", "orange", "yellow", "purple", "gray", "black", "white",
];

const SOURCE: &str = r#"import ui
IMPORTS
public:

struct View {
private:
    text: String
    events: List<EVENT>
    typed: List<EVENT>

    fn styled(self, key: String, value: Int) -> View {
        return View { text := "S{key}{value}:{self.text}", events := self.events.clone, typed := self.typed.clone }
    }

public:

    fn color(self, c: Color) -> View {
        return self.styled("c", c.r * 65536 + c.g * 256 + c.b)
    }

    fn size(self, pixels: Int) -> View
        require pixels > 0 else "a size is at least 1 pixel"
    {
        return self.styled("s", pixels)
    }

    fn bold(self) -> View {
        return self.styled("b", 1)
    }

    fn margin(self, pixels: Int) -> View
        require pixels >= 0 else "a margin cannot be negative"
    {
        return self.styled("m", pixels)
    }
}

struct Color {
private:
    r: Int
    g: Int
    b: Int
}

fn rgb(r: Int, g: Int, b: Int) -> Color
    require r >= 0 && r <= 255 else "red goes from 0 to 255"
    require g >= 0 && g <= 255 else "green goes from 0 to 255"
    require b >= 0 && b <= 255 else "blue goes from 0 to 255"
{
    return Color { r := r, g := g, b := b }
}

fn red -> Color {
    return rgb(220, 38, 38)
}

fn green -> Color {
    return rgb(22, 163, 74)
}

fn blue -> Color {
    return rgb(37, 99, 235)
}

fn orange -> Color {
    return rgb(234, 88, 12)
}

fn yellow -> Color {
    return rgb(202, 138, 4)
}

fn purple -> Color {
    return rgb(147, 51, 234)
}

fn gray -> Color {
    return rgb(107, 114, 128)
}

fn black -> Color {
    return rgb(0, 0, 0)
}

fn white -> Color {
    return rgb(255, 255, 255)
}

enum Align {
    Left
    Top
    Center
    Right
    Bottom
}

fn text(s: String) -> View {
    return View { text := "T{s.length}:{s}", events := [], typed := [] }
}

fn button(label: String, event: EVENT) -> View {
    return View { text := "B{label.length}:{label}", events := [event.clone], typed := [] }
}

fn input(value: String, changed: {String -> EVENT}) -> View {
    let typed: String := ui.typed
    return View { text := "I{value.length}:{value}", events := [], typed := [changed(typed)] }
}

fn column(items: List<View>) align(a: Align := Align.Left) -> View {
    return group("C", items, a)
}

fn row(items: List<View>) align(a: Align := Align.Top) -> View {
    return group("R", items, a)
}

fn run(title: String, state: STATE) width(w: Int := 0) height(h: Int := 0) {
    var s: STATE := state.clone
    while true {
        let v: View := VIEW(s)
        let clicked: Int := ui.show(title, v.text, w, h)
        if clicked = -1 {
            return
        }
        if clicked >= 0 {
            UPDATE(var s, v.events[clicked].clone)
        } else {
            // A field changed: the view again, now that `ui.typed` gives its text.
            let again: View := VIEW(s)
            UPDATE(var s, again.typed[-2 - clicked].clone)
        }
    }
}

private:

fn group(kind: String, items: List<View>, a: Align) -> View {
    let place: String := match a {
        Left => "L"
        Top => "T"
        Center => "C"
        Right => "R"
        Bottom => "B"
    }
    var out: String := "{kind}{place}{items.length}:"
    var events: List<EVENT> := []
    var typed: List<EVENT> := []
    for v in items {
        out := out + v.text
        for e in v.events {
            events.add(e.clone)
        }
        for e in v.typed {
            typed.add(e.clone)
        }
    }
    return View { text := out, events := events, typed := typed }
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
