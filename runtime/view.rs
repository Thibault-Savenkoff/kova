// A view as JSON, for the systems that draw it outside the program: a page (runtime/web.js) and an
// Android app (runtime/android/Main.java).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::shared::{UiAlign, UiMenu, UiNode, UiOp};

/// What `ui.show` hands over.
pub fn message(title: &str, view: &UiNode, menus: &[UiMenu], width: i32, height: i32, acts: &[i64]) -> String {
    let mut o = String::from("{\"title\":");
    quoted(&mut o, title);
    o.push_str(",\"shape\":");
    quoted(&mut o, &crate::shared::ui_shape(view));
    o.push_str(",\"view\":");
    node(&mut o, view);
    o.push_str(",\"menus\":");
    list(&mut o, menus, |o, m| {
        o.push_str("{\"title\":");
        quoted(o, &m.title);
        o.push_str(",\"items\":");
        list(o, &m.items, |o, (label, k, key)| {
            o.push('[');
            quoted(o, label);
            o.push_str(&format!(",{},", or_null(*k)));
            quoted(o, key);
            o.push(']');
        });
        o.push('}');
    });
    o.push_str(&format!(",\"width\":{width},\"height\":{height},\"acts\":"));
    list(&mut o, acts, |o, a| o.push_str(&format!("{a}")));
    o.push('}');
    o
}

/// A site's answer as `status\nurl\nname: value\n…\n\nbody`.
pub fn answer_of(b: &[u8]) -> crate::http::Answer {
    let end = b.windows(2).position(|w| w == b"\n\n").unwrap_or(b.len().saturating_sub(2));
    let head = String::from_utf8_lossy(&b[..end]).into_owned();
    let data = b[(end + 2).min(b.len())..].to_vec();
    let mut lines = head.split('\n');
    let status = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let url = String::from(lines.next().unwrap_or(""));
    let headers = lines.filter_map(|l| l.split_once(": ")).map(|(n, v)| (String::from(n), String::from(v))).collect();
    crate::http::Answer { status, url, body: String::from_utf8_lossy(&data).into_owned(), data, headers }
}

pub fn quoted(o: &mut String, s: &str) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 32 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
}

pub fn list<T>(o: &mut String, items: &[T], mut each: impl FnMut(&mut String, &T)) {
    o.push('[');
    for (i, x) in items.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        each(o, x);
    }
    o.push(']');
}

pub fn texts(o: &mut String, items: &[String]) {
    list(o, items, |o, s| quoted(o, s));
}

fn numbers(o: &mut String, items: &[f32]) {
    list(o, items, |o, v| o.push_str(&format!("{v}")));
}

pub fn or_null(n: Option<usize>) -> String {
    n.map_or("null".into(), |n| format!("{n}"))
}

fn operation(o: &mut String, op: &UiOp) {
    o.push('[');
    match op {
        UiOp::Clear(c) => o.push_str(&format!("\"C\",{c}")),
        UiOp::Rect { x, y, w, h, color, outline } => o.push_str(&format!("\"R\",{x},{y},{w},{h},{color},{outline}")),
        UiOp::Circle { x, y, r, color, outline } => o.push_str(&format!("\"O\",{x},{y},{r},{color},{outline}")),
        UiOp::Line { x, y, x2, y2, color, thickness } => o.push_str(&format!("\"L\",{x},{y},{x2},{y2},{color},{thickness}")),
        UiOp::Text { x, y, size, color, text } => {
            o.push_str(&format!("\"T\",{x},{y},{size},{color},"));
            quoted(o, text);
        }
        UiOp::Image { x, y, w, h, path } => {
            o.push_str(&format!("\"I\",{x},{y},{w},{h},"));
            quoted(o, path);
        }
        UiOp::Gpu => o.push_str("\"g\""),
        UiOp::GlClear(c) => o.push_str(&format!("\"c\",{c}")),
        UiOp::GlShader(vertex, fragment) => {
            o.push_str("\"s\",");
            quoted(o, vertex);
            o.push(',');
            quoted(o, fragment);
        }
        UiOp::GlUniform(name, values) => {
            o.push_str("\"u\",");
            quoted(o, name);
            o.push(',');
            numbers(o, values);
        }
        UiOp::GlTriangles(sizes, data) => {
            o.push_str("\"v\",");
            list(o, sizes, |o, s| o.push_str(&format!("{s}")));
            o.push(',');
            numbers(o, data);
        }
        UiOp::GlTexture(width, height, pixels) => {
            o.push_str(&format!("\"x\",{width},{height},"));
            list(o, pixels, |o, p| o.push_str(&format!("{p}")));
        }
    }
    o.push(']');
}

pub fn node(o: &mut String, n: &UiNode) {
    let side = |a: &UiAlign| *a as u8;
    match n {
        UiNode::Text(t) => {
            o.push_str("[\"T\",");
            quoted(o, t);
        }
        UiNode::Button(t, k) | UiNode::Input(t, k) => {
            o.push_str(if matches!(n, UiNode::Button(..)) { "[\"B\"," } else { "[\"I\"," });
            quoted(o, t);
            o.push_str(&format!(",{k}"));
        }
        UiNode::Column(parts, a) | UiNode::Row(parts, a) => {
            o.push_str(&format!("[\"{}\",{},", if matches!(n, UiNode::Row(..)) { 'R' } else { 'C' }, side(a)));
            list(o, parts, node);
        }
        UiNode::Image(path, width) => {
            o.push_str("[\"P\",");
            quoted(o, path);
            o.push_str(&format!(",{width}"));
        }
        UiNode::Styled(style, inner) => {
            // On Android, `.markdown` is a wrapper of its own: the other systems read `S` alone.
            #[cfg(android)]
            let marked = style.marks;
            #[cfg(not(android))]
            let marked = false;
            if marked {
                o.push_str("[\"m\",");
            }
            let color = style.color.map_or("null".into(), |c| format!("{c}"));
            o.push_str(&format!("[\"S\",{color},{},{},", style.size, style.bold));
            node(o, inner);
            if marked {
                o.push(']');
            }
        }
        // A right click on a page, a long press on Android.
        UiNode::Context(inner, items) => {
            o.push_str("[\"J\",");
            list(o, items, |o, (label, number)| {
                o.push('[');
                quoted(o, label);
                o.push_str(&format!(",{number}]"));
            });
            o.push(',');
            node(o, inner);
        }
        UiNode::Spaced(m, inner) => {
            o.push_str(&format!("[\"M\",{m},"));
            node(o, inner);
        }
        UiNode::Canvas(c) => {
            o.push_str(&format!("[\"D\",{},{},{},{},{},", c.width, c.height, c.number, or_null(c.mouse), or_null(c.key)));
            list(o, &c.ops, operation);
        }
        UiNode::Every(ms, k, inner) => {
            o.push_str(&format!("[\"E\",{ms},{k},"));
            node(o, inner);
        }
        UiNode::Check(label, on, k) => {
            o.push_str("[\"X\",");
            quoted(o, label);
            o.push_str(&format!(",{on},{k}"));
        }
        UiNode::Slider(value, min, max, k) => o.push_str(&format!("[\"W\",{value},{min},{max},{k}")),
        UiNode::Choice(items, selected, k) => {
            o.push_str("[\"O\",");
            texts(o, items);
            o.push_str(&format!(",{selected},{k}"));
        }
        UiNode::Table(t) => {
            o.push_str("[\"G\",");
            texts(o, &t.headers);
            o.push(',');
            list(o, &t.rows, |o, r| texts(o, r));
            o.push_str(&format!(",{},{},{}", t.selected, t.number, t.deletable));
        }
        // The phone's own screens (iPhone and Android): a color behind a view, a title, a page
        // over a view, buttons in the top bar, a search field, tabs, and a few small controls.
        #[cfg(mobile)]
        UiNode::Boxed(color, round, inner) => {
            o.push_str(&format!("[\"b\",{color},{round},"));
            node(o, inner);
        }
        #[cfg(mobile)]
        UiNode::Titled(title, inner) => {
            o.push_str("[\"t\",");
            quoted(o, title);
            o.push(',');
            node(o, inner);
        }
        #[cfg(mobile)]
        UiNode::Pushed(main, page, back) => {
            o.push_str(&format!("[\"u\",{back},"));
            node(o, main);
            o.push(',');
            node(o, page);
        }
        #[cfg(mobile)]
        UiNode::Barred(inner, buttons) => {
            o.push_str("[\"r\",");
            list(o, buttons, |o, (label, k)| {
                o.push('[');
                quoted(o, label);
                o.push_str(&format!(",{k}]"));
            });
            o.push(',');
            node(o, inner);
        }
        #[cfg(mobile)]
        UiNode::Searched(inner, text, k) => {
            o.push_str("[\"s\",");
            quoted(o, text);
            o.push_str(&format!(",{k},"));
            node(o, inner);
        }
        #[cfg(mobile)]
        UiNode::Tabs(parts, icons) => {
            o.push_str("[\"H\",");
            texts(o, icons);
            o.push(',');
            list(o, parts, node);
        }
        #[cfg(mobile)]
        UiNode::Date(text, k) | UiNode::Search(text, k) => {
            o.push_str(if matches!(n, UiNode::Date(..)) { "[\"d\"," } else { "[\"q\"," });
            quoted(o, text);
            o.push_str(&format!(",{k}"));
        }
        #[cfg(mobile)]
        UiNode::Progress(value, total) => o.push_str(&format!("[\"p\",{value},{total}")),
        #[cfg(mobile)]
        UiNode::Stepper(value, min, max, k) => o.push_str(&format!("[\"e\",{value},{min},{max},{k}")),
        #[cfg(mobile)]
        UiNode::Spinner => o.push_str("[\"z\""),
        // Only an iPhone has UIKit views to put in a window.
        #[cfg(ios)]
        UiNode::Native(..) => o.push_str("[\"T\",\"\""),
    }
    o.push(']');
}
