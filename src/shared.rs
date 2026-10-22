// Code used both by `kova run` (src/interp.rs) and by the runtime of `kova build` (runtime/),
// so that the two print and compute the same thing from one copy. Only `core` and `alloc`
// here: the runtime has no `std`.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

/// The capital (`upper`) or small letter of `c`, for the Latin, Greek and Cyrillic alphabets;
/// any other character stays. Rust's own `to_uppercase` knows all of Unicode, but its tables
/// are in `std`, which the runtime does not have; and the C runtime (`runtime/runtime.c`, still
/// used on some systems) has the same table.
pub fn case_of(c: char, upper: bool) -> char {
    let c = c as u32;
    let pairs = |c: u32, even_upper: bool| {
        let ranges: &[(u32, u32)] = if even_upper { &[(0x100, 0x12F), (0x132, 0x137), (0x14A, 0x177)] } else { &[(0x139, 0x148), (0x179, 0x17E)] };
        ranges.iter().any(|&(a, b)| (a..=b).contains(&c))
    };
    let code = if upper {
        match c {
            0x61..=0x7A | 0x3B1..=0x3C9 | 0x430..=0x44F if c != 0x3C2 => c - 32,
            0xE0..=0xFE if c != 0xF7 => c - 32,
            0xFF => 0x178,
            0x3C2 => 0x3A3,
            0x450..=0x45F => c - 80,
            _ if pairs(c, true) && c % 2 == 1 => c - 1,
            _ if pairs(c, false) && c % 2 == 0 => c - 1,
            _ => c,
        }
    } else {
        match c {
            0x41..=0x5A | 0x391..=0x3A9 | 0x410..=0x42F if c != 0x3A2 => c + 32,
            0xC0..=0xDE if c != 0xD7 => c + 32,
            0x178 => 0xFF,
            0x400..=0x40F => c + 80,
            _ if pairs(c, true) && c % 2 == 0 => c + 1,
            _ if pairs(c, false) && c % 2 == 1 => c + 1,
            _ => c,
        }
    };
    char::from_u32(code).expect("a letter of the table")
}

/// `s.to_float`: an optional `-`, digits, then maybe `.` and digits, then maybe `e`, a sign and
/// digits; `None` if infinite.
pub fn parse_float(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let d = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > d
    };
    if b.first() == Some(&b'-') {
        i += 1;
    }
    if !digits(&mut i) {
        return None;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return None;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return None;
        }
    }
    let v: f64 = if i == b.len() { s.parse().ok()? } else { return None };
    v.is_finite().then_some(v)
}

/// `s.to_int`: an optional `-` then digits only, within the range of an `Int`
/// (`kova run` and `kova build` alike).
pub fn parse_int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A Float the way Kova shows it: `3.0`, `0.30000000000000004`, `1e16`, `NaN`, `-inf`.
///
/// The shortest digits that give back exactly the number (on a tie, the even digit,
/// like Python). Scientific notation below 0.0001 and from 10^16 on.
pub fn float_text(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    let a = x.abs();
    let (digits, exp) = shortest(a);
    let digits: String = digits.iter().map(|d| (b'0' + d) as char).collect();
    let n = digits.len() as i32;
    let body = if a == 0.0 || (1e-4..1e16).contains(&a) {
        if exp >= 0 {
            let int_part: String =
                (0..=exp).map(|i| digits.as_bytes().get(i as usize).map_or('0', |&c| c as char)).collect();
            let decimals = if n > exp + 1 { &digits[exp as usize + 1..] } else { "0" };
            format!("{int_part}.{decimals}")
        } else {
            format!("0.{}{digits}", "0".repeat((-exp - 1) as usize))
        }
    } else if n > 1 {
        format!("{}.{}e{exp}", &digits[..1], &digits[1..])
    } else {
        format!("{digits}e{exp}")
    };
    format!("{}{body}", if x.is_sign_negative() { "-" } else { "" })
}

/// The shortest digits of `a` (≥ 0) that give back `a`, and the exponent of the first one.
/// Rust's `{:e}` finds how many are needed; on a tie between two of that many, it may take the
/// higher one, so the number rounded to that many (the even digit on a tie) is taken when it
/// also gives back `a`. Checked against the exact digits on 8.6 million Floats (2026-10-22).
fn shortest(a: f64) -> (Vec<u8>, i32) {
    let digits = |s: &str| {
        let (m, e) = s.split_once('e').unwrap();
        (m.bytes().filter(|&c| c != b'.').map(|c| c - b'0').collect::<Vec<u8>>(), e.parse::<i32>().unwrap())
    };
    let (d, e) = digits(&format!("{a:e}"));
    let rounded = format!("{a:.*e}", d.len() - 1);
    if rounded.parse::<f64>() == Ok(a) {
        let (mut r, e) = digits(&rounded);
        while r.len() > 1 && r.last() == Some(&0) {
            r.pop();
        }
        return (r, e);
    }
    (d, e)
}

/// Element `i` of a Map or a Set goes out of its index (`s`, its slots: `s[0]` = the number of
/// elements in it, then (key's number, position + 1) pairs): its place is emptied, the next keys
/// moved back towards their own place, as linear probing needs, and the positions after it move
/// back by one (index made by `find_in_index` in src/kir.rs).
pub fn unindex(s: &mut [i64], i: i64) {
    let cap = s.len() as i64 / 2 - 1;
    if i >= s[0] {
        return; // not in the index yet
    }
    let mut j = 0;
    for k in 1..=cap as usize {
        if s[2 * k + 1] == i + 1 {
            j = k as i64;
        }
        if s[2 * k + 1] > i + 1 {
            s[2 * k + 1] -= 1;
        }
    }
    s[0] -= 1;
    let mut k = j % cap + 1;
    while s[2 * k as usize + 1] != 0 {
        let home = s[2 * k as usize] % cap + 1;
        // `k` stays if its own place lies after `j` (cyclically, up to `k`).
        let stays = if j < k { j < home && home <= k } else { j < home || home <= k };
        if !stays {
            s[2 * j as usize] = s[2 * k as usize];
            s[2 * j as usize + 1] = s[2 * k as usize + 1];
            j = k;
        }
        k = k % cap + 1;
    }
    s[2 * j as usize] = 0;
    s[2 * j as usize + 1] = 0;
}

/// A number for a text (FNV-1a), for the index of a Map or a Set (`StrOp::Hash`).
pub fn hash(text: &str) -> i64 {
    text.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)) as i64
}

/// `random(from..to)`, `to` excluded, `from < to` checked before by src/kir.rs; `state` =
/// splitmix64's, started from `KOVA_SEED` (so that a run can be made again) or from the clock.
/// Draws again above the last whole multiple of the range's size, so that each number is as likely.
pub fn random(state: &mut u64, from: i64, to: i64) -> i64 {
    let mut next = || {
        *state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = *state;
        z = (z ^ z >> 30).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ z >> 27).wrapping_mul(0x94D049BB133111EB);
        z ^ z >> 31
    };
    let n = (to as u64).wrapping_sub(from as u64);
    let limit = u64::MAX - u64::MAX % n;
    let x = loop {
        let x = next();
        if x < limit {
            break x;
        }
    };
    (from as u64).wrapping_add(x % n) as i64
}

/// `KOVA_SEED`'s value, if it is a number (it must start with a digit).
pub fn seed(value: Option<&str>) -> Option<u64> {
    value.filter(|s| s.starts_with(|c: char| c.is_ascii_digit())).and_then(|s| s.parse().ok())
}
