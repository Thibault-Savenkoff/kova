// Localisation: every message Kova prints comes from `locales/*.kova` (PLAN 2.13).
//
//   // locales/fr.kova
//   lang fr {
//       already_exists := "`{name}` existe déjà (ligne {line})"
//   }
//
//   t!("already_exists", name = "x", line = 2)   →   "`x` existe déjà (ligne 2)"
//
// `{name}` is replaced by the given value; `\{` and `\}` are plain braces.
// The language: `header { lang := fr }`, otherwise KOVA_LANG, otherwise
// LC_ALL / LC_MESSAGES / LANG, otherwise Windows' settings, otherwise English.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::lexer;

/// `t!("key", name = value, ...)`: the message in the current language.
macro_rules! t {
    ($key:literal $(, $n:ident = $v:expr)* $(,)?) => {
        $crate::locale::text($key, &[$((stringify!($n), &$v as &dyn std::fmt::Display)),*])
    };
}
pub(crate) use t;

/// The known languages: (code, source of the translation file). English first: the default.
pub const LANGUAGES: &[(&str, &str)] =
    &[("en", include_str!("../locales/en.kova")), ("fr", include_str!("../locales/fr.kova"))];

/// The texts the runtime needs (`runtime/kova.rs`), in this order. No `{name}` in them, but in
/// `RUNTIME_HOLES`: the runtime fills those itself.
pub const RUNTIME: &[&str] = &[
    "error_prefix",
    "fault_overflow",
    "fault_division_by_zero",
    "fault_bad_conversion",
    "contract_violated",
    "contract_values",
    "contract_called_from",
    // `KOVA_LEAKS` (the runtime reads them at indexes 7 and 8).
    "leak_allocated",
    "leak_live",
    // runtime/kova.rs: `FAULT_INDEX`.
    "fault_index",
    "fault_not_text",
    "fault_empty_text",
    "fault_exit_code",
    "fault_negative_power",
    "fault_empty_range",
    "fault_bad_date",
    "fault_bad_time",
    "fault_cannot_serve",
    // What `serve` writes when it starts (runtime/kova.rs `kova_listen`).
    "serving",
    "serving_files",
    "this_computer",
    "this_computer_only",
    "this_network",
    "stop_with_ctrl_c",
    // `serve(…) workers(0)`.
    "fault_workers",
    "fault_no_elements",
    "fault_negative_size",
    "fault_bad_key",
    "fault_password_settings",
    // `KOVA_LOG` naming no level (runtime/kova.rs `LOG_UNKNOWN_LEVEL`).
    "log_unknown_level",
    // `extern` (runtime/kova.rs `kova_extern`).
    "fault_int32",
    "fault_text_zero",
    "fault_null_text",
    "fault_null_pointer",
    "fault_extern_interpreted",
    "extern_no_library",
    "extern_no_function",
];

/// The texts of `RUNTIME` whose holes the runtime fills (`kova_listen`), written as `{name}`.
pub const RUNTIME_HOLES: &[(&str, &[&str])] = &[("serving", &["port"]), ("serving_files", &["folder", "port"]), ("log_unknown_level", &["name"]), ("extern_no_library", &["library"]), ("extern_no_function", &["library", "function"])];

/// A piece of a translated text.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Text(String),
    /// `{name}`: replaced by the value given for `name`.
    Hole(String),
}

/// Index in `LANGUAGES`; `usize::MAX` = not chosen yet.
static CURRENT: AtomicUsize = AtomicUsize::new(usize::MAX);

/// `"fr_FR.UTF-8"`, `"fr-ca"` or `"fr"` → the language `fr`, if it is known.
pub fn recognize(value: &str) -> Option<usize> {
    let code = value.split(['_', '.', '@', '-']).next()?.to_lowercase();
    LANGUAGES.iter().position(|(c, _)| *c == code)
}

/// The language asked by the environment: the first non-empty variable decides,
/// even if it is unknown (`LANG=C` → English). Same rule in `runtime/kova.rs`.
fn system() -> usize {
    ["KOVA_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .or_else(windows_language)
        .and_then(|s| recognize(&s))
        .unwrap_or(0)
}

/// Windows sets no `LANG`: the language of the user's settings (`fr-FR`), as in
/// runtime/windows.rs.
#[cfg(windows)]
fn windows_language() -> Option<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, n: i32) -> i32;
    }
    let mut name = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    (n > 1).then(|| String::from_utf16_lossy(&name[..n as usize - 1]))
}

#[cfg(not(windows))]
fn windows_language() -> Option<String> {
    None
}

/// Uses `code` (`header { lang := fr }`), or the system's language if `None` or `auto`.
/// Returns true if the language changed.
pub fn choose(code: Option<&str>) -> bool {
    let i = code.and_then(recognize).unwrap_or_else(system);
    CURRENT.swap(i, Ordering::Relaxed) != i
}

pub fn current() -> usize {
    match CURRENT.load(Ordering::Relaxed) {
        usize::MAX => {
            let i = system();
            CURRENT.store(i, Ordering::Relaxed);
            i
        }
        i => i,
    }
}

thread_local! {
    /// True while the catalogs are being read: they are read by Kova's own lexer and
    /// parser, whose messages would ask for the catalogs again.
    static LOADING: Cell<bool> = const { Cell::new(false) };
}

/// The catalog of each language: key → segments.
fn catalogs() -> &'static [HashMap<String, Vec<Segment>>] {
    static CATALOGS: OnceLock<Vec<HashMap<String, Vec<Segment>>>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        LOADING.set(true);
        let all = LANGUAGES.iter().map(|(_, src)| read(src).into_iter().map(|(k, s, _)| (k, s)).collect()).collect();
        LOADING.set(false);
        all
    })
}

/// The entries of a translation file: (key, segments, position of the text).
/// Entries whose text is wrong are left out (the type checker reports them).
pub fn read(src: &str) -> Vec<(String, Vec<Segment>, usize)> {
    let (tokens, _) = lexer::lex(src);
    let (file, ..) = crate::parser::parse(tokens, src);
    file.langs
        .iter()
        .flat_map(|l| &l.entries)
        .filter_map(|(key, text)| Some((key.text.clone(), segments(&src[text.start..text.end]).ok()?, text.start)))
        .collect()
}

/// `"`{name}` exists"` (the token, with its quotes) → its segments.
/// Error: (position in the token, message).
pub fn segments(token: &str) -> Result<Vec<Segment>, (usize, String)> {
    let inner = &token[1..token.len() - 1];
    let mut out = vec![];
    let mut text = String::new();
    let mut it = inner.char_indices();
    while let Some((k, c)) = it.next() {
        match c {
            '\\' => text.push(lexer::escape(&mut it).map_err(|m| (k + 1, m))?),
            '{' => {
                let name: String = it.by_ref().map(|(_, c)| c).take_while(|&c| c != '}').collect();
                let good = name.starts_with(|c: char| c.is_ascii_lowercase())
                    && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                if !good || !inner[k..].contains('}') {
                    return Err((k + 1, t!("hole_bad")));
                }
                out.push(Segment::Text(std::mem::take(&mut text)));
                out.push(Segment::Hole(name));
            }
            '}' => return Err((k + 1, t!("brace_alone"))),
            c => text.push(c),
        }
    }
    out.push(Segment::Text(text));
    out.retain(|s| *s != Segment::Text(String::new()));
    Ok(out)
}

/// The names of the `{holes}` of a text, sorted.
pub fn holes(segments: &[Segment]) -> Vec<&str> {
    let mut v: Vec<&str> = segments.iter().filter_map(|s| if let Segment::Hole(h) = s { Some(h.as_str()) } else { None }).collect();
    v.sort();
    v.dedup();
    v
}

/// The text of `key` in language `lang` (English if missing), with its holes filled.
pub fn text_in(lang: usize, key: &str, args: &[(&str, &dyn Display)]) -> String {
    if LOADING.get() {
        return key.to_string();
    }
    let cats = catalogs();
    let Some(segments) = cats[lang].get(key).or_else(|| cats[0].get(key)) else {
        debug_assert!(false, "unknown message key `{key}`");
        return key.to_string();
    };
    segments
        .iter()
        .map(|s| match s {
            Segment::Text(t) => t.clone(),
            Segment::Hole(h) => match args.iter().find(|(n, _)| n == h) {
                Some((_, v)) => v.to_string(),
                None => {
                    debug_assert!(false, "`{key}`: no value for `{{{h}}}`");
                    format!("{{{h}}}")
                }
            },
        })
        .collect()
}

/// The text of `key` in the current language.
pub fn text(key: &str, args: &[(&str, &dyn Display)]) -> String {
    text_in(current(), key, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filling() {
        let s = segments(r#""`{x}` is {x}, \{not\} \\ {y_2}""#).unwrap();
        assert_eq!(holes(&s), ["x", "y_2"]);
        assert_eq!(s[0], Segment::Text("`".into()));
        assert_eq!(s[4], Segment::Text(", {not} \\ ".into()));
        assert!(segments(r#""a { b""#).is_err());
        assert!(segments(r#""a {B}""#).is_err());
        assert!(segments(r#""a }""#).is_err());
        assert_eq!(recognize("fr_FR.UTF-8"), Some(1));
        assert_eq!(recognize("en-us"), Some(0));
        assert_eq!(recognize("C"), None);
        assert_eq!(text_in(1, "already_exists", &[("name", &"x"), ("line", &2)]), "`x` existe déjà (ligne 2)");
    }

    /// The embedded files are correct Kova, and every language has the same keys with the same holes.
    #[test]
    fn complete_catalogs() {
        for (code, src) in LANGUAGES {
            let (tokens, mut diags) = lexer::lex(src);
            let (file, n, d) = crate::parser::parse(tokens, src);
            diags.extend(d);
            diags.extend(crate::types::check(&file, n, src).1);
            assert!(diags.is_empty(), "{code}.kova: {:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
            assert_eq!(file.langs.len(), 1, "{code}.kova: one `lang` block");
            assert_eq!(file.langs[0].name.text, *code);
        }
        let cats = catalogs();
        for (i, (code, _)) in LANGUAGES.iter().enumerate() {
            for (key, segs) in &cats[0] {
                let other = cats[i].get(key).unwrap_or_else(|| panic!("`{key}` missing in {code}.kova"));
                assert_eq!(holes(segs), holes(other), "`{key}`: different holes in {code}.kova");
            }
            assert_eq!(cats[i].len(), cats[0].len(), "{code}.kova has extra keys");
        }
        for key in RUNTIME {
            let allowed = RUNTIME_HOLES.iter().find(|(k, _)| k == key).map_or(&[][..], |(_, h)| *h);
            assert!(holes(&cats[0][*key]).iter().all(|h| allowed.contains(h)), "`{key}` is used by the runtime: no holes but {allowed:?}");
        }
        assert_eq!(RUNTIME[0], "error_prefix", "the runtime reads the prefix at index 0");
        assert_eq!(RUNTIME[7..9], ["leak_allocated", "leak_live"], "the runtime reads them at 7 and 8");
        assert_eq!(RUNTIME[29], "log_unknown_level", "the runtime reads it at 29");
        assert_eq!(RUNTIME[35..37], ["extern_no_library", "extern_no_function"], "the runtime reads them at 35 and 36");
        let (_, values, _) = crate::types::SETTINGS.iter().find(|(k, ..)| *k == "lang").unwrap();
        let codes: Vec<&str> = LANGUAGES.iter().map(|(c, _)| *c).collect();
        assert_eq!(values[1..], codes[..], "`header {{ lang }}` offers `auto` then every language");
    }
}
