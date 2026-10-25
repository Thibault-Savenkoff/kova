// CSV (PLAN Part 4 "The library after Python, step 2"), shared by `kova run` (src/interp.rs) and
// the runtime of `kova build` (runtime/kova.rs). `List<Book>.from_csv(text)` reads a table whose
// first line names the columns; `books.to_csv` writes one. The compiler describes `Book` to these
// functions as a text (`spec`): `s:title,i:pages,?f:price`, a field per name, its kind first, `?`
// for an optional. Only `core` and `alloc`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::shared;

/// A field of the struct read or written: its name, its kind (`i` Int, `f` Float, `b` Bool, `c`
/// Char, `s` String, `d` Date, `t` DateTime), optional or not.
pub struct Field<'a> {
    pub name: &'a str,
    pub kind: u8,
    pub optional: bool,
}

/// The fields of a `spec` (src/kir.rs writes it).
pub fn fields(spec: &str) -> Vec<Field<'_>> {
    spec.split(',').filter(|f| !f.is_empty()).map(|f| {
        let optional = f.starts_with('?');
        let f = f.trim_start_matches('?');
        Field { name: &f[2..], kind: f.as_bytes()[0], optional }
    }).collect()
}

/// A slot of a row read: a number as it is laid out (a Float as its bits), or a text that becomes
/// a new String.
pub enum Slot {
    Int(i64),
    Text(String),
}

/// Why a text gives no table: `CsvError`'s variants (src/types.rs `csv_error`).
#[derive(Debug, PartialEq)]
pub enum Problem {
    /// A `"` never closed, on the line where its cell starts.
    NotCsv(usize),
    /// No column for this field.
    Missing(usize),
    /// The cell of this field on this line is not one of its kind.
    WrongKind(usize, usize),
}

impl Problem {
    /// One number for src/kir.rs, below 0: minus (1 + variant + 4 × (field + 4096 × line)).
    pub fn code(&self) -> i64 {
        let (variant, field, line) = match *self {
            Problem::NotCsv(line) => (0, 0, line),
            Problem::Missing(field) => (1, field, 0),
            Problem::WrongKind(line, field) => (2, field, line),
        };
        -(1 + variant + 4 * (field as i64 + 4096 * line as i64))
    }
}

/// The separator of a table: `,`, `;` or a tab, the one found most often in its first line
/// outside quotes (`,` if none).
fn separator(text: &str) -> char {
    let (mut counts, mut quoted) = ([0; 3], false);
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' if !quoted => break,
            ',' | ';' | '\t' if !quoted => counts[[',', ';', '\t'].iter().position(|&s| s == c).unwrap()] += 1,
            _ => {}
        }
    }
    let most = (0..3).max_by_key(|&i| (counts[i], 3 - i)).unwrap();
    [',', ';', '\t'][most]
}

/// The rows of a table, each with the line where it starts (from 1), and its separator. Empty
/// lines are left out; `\r\n` ends a line like `\n`; a cell between quotes may hold separators,
/// line ends and `""` (one `"`). A leading byte order mark (Excel's "CSV UTF-8") is skipped.
pub fn rows(text: &str) -> Result<(Vec<(usize, Vec<String>)>, char), Problem> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let sep = separator(text);
    let (mut rows, mut row, mut cell) = (vec![], vec![], String::new());
    let (mut line, mut start, mut quoted, mut opened) = (1, 1, false, 0);
    let mut chars = text.chars().peekable();
    // `cell` was between quotes: an empty one still counts.
    let mut was_quoted = false;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                c => {
                    line += (c == '\n') as usize;
                    cell.push(c);
                }
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() && !was_quoted => {
                (quoted, was_quoted, opened) = (true, true, line);
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                if !row.is_empty() || !cell.is_empty() || was_quoted {
                    row.push(core::mem::take(&mut cell));
                    rows.push((start, core::mem::take(&mut row)));
                }
                line += 1;
                (start, was_quoted) = (line, false);
            }
            c if c == sep => {
                row.push(core::mem::take(&mut cell));
                was_quoted = false;
            }
            c => cell.push(c),
        }
    }
    if quoted {
        return Err(Problem::NotCsv(opened));
    }
    if !row.is_empty() || !cell.is_empty() || was_quoted {
        row.push(cell);
        rows.push((start, row));
    }
    Ok((rows, sep))
}

/// `List<Book>.from_csv(text)`: the slots of every row, one after the other, as a `List<Book>`
/// lays them out (an optional is two slots: 1 and its value, or 0 and 0). A cell missing at the
/// end of a row counts as empty; cells after the last column read are left out.
pub fn records(text: &str, spec: &str) -> Result<Vec<Slot>, Problem> {
    let fields = fields(spec);
    let (rows, sep) = rows(text)?;
    let Some(((_, header), rows)) = rows.split_first() else {
        return match fields.is_empty() {
            true => Ok(vec![]),
            false => Err(Problem::Missing(0)),
        };
    };
    let mut columns = vec![];
    for (k, f) in fields.iter().enumerate() {
        match header.iter().position(|h| h.trim() == f.name) {
            Some(c) => columns.push(c),
            None => return Err(Problem::Missing(k)),
        }
    }
    let mut slots = vec![];
    for (line, row) in rows {
        for (k, f) in fields.iter().enumerate() {
            let cell = row.get(columns[k]).map_or("", String::as_str);
            let wrong = || Problem::WrongKind(*line, k);
            if f.optional {
                if cell.trim().is_empty() {
                    slots.extend([Slot::Int(0), Slot::Int(0)]);
                    continue;
                }
                slots.push(Slot::Int(1));
            }
            slots.push(match f.kind {
                b's' => Slot::Text(cell.to_string()),
                b'c' => {
                    let mut cs = cell.chars();
                    match (cs.next(), cs.next()) {
                        (Some(c), None) => Slot::Int(c as i64),
                        _ => return Err(wrong()),
                    }
                }
                b'i' => Slot::Int(shared::parse_int(cell.trim()).ok_or_else(wrong)?),
                b'f' => {
                    let t = cell.trim();
                    // `3,5`: a French spreadsheet's decimal comma, when `,` is not the separator.
                    let comma = (sep != ',' && t.matches(',').count() == 1 && !t.contains('.')).then(|| t.replace(',', "."));
                    let x = shared::parse_float(t).or_else(|| comma.and_then(|t| shared::parse_float(&t))).ok_or_else(wrong)?;
                    Slot::Int(x.to_bits() as i64)
                }
                b'b' => match cell.trim() {
                    "true" => Slot::Int(1),
                    "false" => Slot::Int(0),
                    _ => return Err(wrong()),
                },
                kind => Slot::Int(shared::parse_date(cell.trim(), kind == b't').ok_or_else(wrong)?),
            });
        }
    }
    Ok(slots)
}

/// A cell as a table writes it: between quotes if it holds the separator, a `"`, a line end, or
/// spaces at either end (`"` doubled inside).
fn cell(s: &str, sep: char, out: &mut String) {
    let quoted = s.contains([sep, '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ');
    if quoted {
        out.push('"');
        out.push_str(&s.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(s);
    }
}

/// `rows.to_csv`: each row on its own line, ended by `\n`.
pub fn write_rows(rows: &[Vec<String>], sep: char) -> String {
    let mut out = String::new();
    for row in rows {
        for (k, c) in row.iter().enumerate() {
            if k > 0 {
                out.push(sep);
            }
            cell(c, sep, &mut out);
        }
        out.push('\n');
    }
    out
}

/// `books.to_csv`: a first line naming the fields, then a line per element. `slots` are the
/// List's slots, `text` reads a String slot. A number is written like `print` writes it, `none`
/// as an empty cell.
pub fn write_records(spec: &str, slots: &[i64], text: &dyn Fn(i64) -> String, sep: char) -> String {
    let fields = fields(spec);
    let mut rows = vec![fields.iter().map(|f| f.name.to_string()).collect::<Vec<_>>()];
    let width: usize = fields.iter().map(|f| 1 + f.optional as usize).sum();
    for element in slots.chunks(width.max(1)).filter(|_| width > 0) {
        let mut at = 0;
        let mut row = vec![];
        for f in &fields {
            if f.optional {
                at += 1;
                if element[at - 1] == 0 {
                    row.push(String::new());
                    at += 1;
                    continue;
                }
            }
            let v = element[at];
            at += 1;
            row.push(match f.kind {
                b's' => text(v),
                b'c' => char::from_u32(v as u32).unwrap_or('?').to_string(),
                b'i' => format!("{v}"),
                b'f' => shared::float_text(f64::from_bits(v as u64)),
                b'b' => (if v != 0 { "true" } else { "false" }).to_string(),
                kind => shared::date_text(v, kind == b't'),
            });
        }
        rows.push(row);
    }
    write_rows(&rows, sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(text: &str) -> Vec<Vec<String>> {
        rows(text).unwrap().0.into_iter().map(|(_, r)| r).collect()
    }

    #[test]
    fn reading() {
        assert_eq!(cells("a,b\n1,2\n"), [["a", "b"], ["1", "2"]]);
        assert_eq!(cells("\u{feff}a;b\r\n1;\"x;\"\"y\"\"\"\r\n\r\n"), [["a", "b"], ["1", "x;\"y\""]]);
        assert_eq!(cells("a\tb\n\"deux\nlignes\"\tz"), [["a", "b"], ["deux\nlignes", "z"]]);
        assert_eq!(cells("a,\"\"\n"), [["a", ""]]);
        assert_eq!(rows("a,b\n1,\"2\n3").err(), Some(Problem::NotCsv(2)));
        let r = records("nom;prix;n\nvis;3,5;\nclou; 2 ;4", "s:nom,f:prix,?i:n").unwrap();
        let ints: Vec<i64> = r.iter().map(|s| if let Slot::Int(n) = s { *n } else { -1 }).collect();
        assert_eq!(ints, [-1, 3.5f64.to_bits() as i64, 0, 0, -1, 2f64.to_bits() as i64, 1, 4]);
        assert_eq!(records("a,b\n1,2", "i:a,i:c").err(), Some(Problem::Missing(1)));
        assert_eq!(records("a,b\n1,2\nx,3", "i:a,i:b").err(), Some(Problem::WrongKind(3, 0)));
    }

    #[test]
    fn writing() {
        let rows = vec![vec!["a b".to_string(), "x,y".into(), "dit \"oui\"".into(), " lead".into()]];
        assert_eq!(write_rows(&rows, ','), "a b,\"x,y\",\"dit \"\"oui\"\"\",\" lead\"\n");
        let text = |_| "vis".to_string();
        assert_eq!(write_records("s:nom,?f:prix,b:ok", &[7, 1, 2.5f64.to_bits() as i64, 1, 7, 0, 0, 0], &text, ';'), "nom;prix;ok\nvis;2.5;true\nvis;;false\n");
    }
}
