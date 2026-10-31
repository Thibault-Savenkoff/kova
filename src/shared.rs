// Code used both by `kova run` (src/interp.rs) and by the runtime of `kova build` (runtime/),
// so that the two print and compute the same thing from one copy. Only `core` and `alloc`
// here: the runtime has no `std`.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

/// The capital (`upper`) or small letter of `c`, for the Latin, Greek and Cyrillic alphabets;
/// any other character stays. Rust's own `to_uppercase` knows all of Unicode, but its tables
/// are in `std`, which the runtime does not have.
/// `c.is_letter` and the others: the number of each is what `char_test` receives.
pub const CHAR_TESTS: [&str; 7] = ["is_letter", "is_digit", "is_space", "is_upper", "is_lower", "to_upper", "to_lower"];

pub fn char_test(c: i64, test: i64) -> i64 {
    let Some(c) = u32::try_from(c).ok().and_then(char::from_u32) else { return 0 };
    match test {
        0 => c.is_alphabetic() as i64,
        1 => c.is_ascii_digit() as i64,
        2 => c.is_whitespace() as i64,
        3 => c.is_uppercase() as i64,
        4 => c.is_lowercase() as i64,
        5 => case_of(c, true) as i64,
        _ => case_of(c, false) as i64,
    }
}

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

/// The maths Rust's `core` doesn't have (`kir::Math`, in its order), from `libm`: musl's, in Rust,
/// with no instruction of the processor, so the same digits in `kova run` and `kova build` on
/// every system (PLAN Part 4 "Maths").
pub fn math(op: i64, x: f64, y: f64) -> f64 {
    match op {
        0 => libm::sin(x),
        1 => libm::cos(x),
        2 => libm::tan(x),
        3 => libm::asin(x),
        4 => libm::acos(x),
        5 => libm::atan(x),
        6 => ln(x),
        7 => log10(x),
        8 => exp(x),
        _ => pow(x, y),
    }
}

// `exp`, `ln`, `log10` and `pow` exactly rounded (the Float nearest to the true value, like
// Python's on Linux), where `libm`'s may be one last digit off (`1.0.exp` = 2.7182818284590455):
// computed with about 32 digits ("double-double": a Float plus the small Float it misses), then
// rounded. The rare cases (NaN, infinities, results near the limits or tiny) go to `libm`.

/// `hi + lo`, with `|lo|` under half a last digit of `hi`.
#[derive(Clone, Copy)]
struct Dd(f64, f64);

/// `a + b` exactly (Knuth).
fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let v = s - a;
    Dd(s, (a - (s - v)) + (b - v))
}

/// `a * b` exactly (Dekker: each cut into two halves of 26 bits).
fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    let split = |x: f64| {
        let t = 134217729.0 * x;
        let hi = t - (t - x);
        (hi, x - hi)
    };
    let ((ah, al), (bh, bl)) = (split(a), split(b));
    Dd(p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
}

impl Dd {
    fn add(self, o: Dd) -> Dd {
        let s = two_sum(self.0, o.0);
        let t = two_sum(self.1, o.1);
        let s = two_sum(s.0, s.1 + t.0);
        two_sum(s.0, s.1 + t.1)
    }
    fn neg(self) -> Dd {
        Dd(-self.0, -self.1)
    }
    fn mul(self, o: Dd) -> Dd {
        let p = two_prod(self.0, o.0);
        two_sum(p.0, p.1 + (self.0 * o.1 + self.1 * o.0))
    }
    fn div(self, o: Dd) -> Dd {
        let q = self.0 / o.0;
        let r = self.add(o.mul(Dd(q, 0.0)).neg());
        let q2 = r.0 / o.0;
        let r = r.add(o.mul(Dd(q2, 0.0)).neg());
        two_sum(q, q2).add(Dd(r.0 / o.0, 0.0))
    }
    fn round(self) -> f64 {
        self.0 + self.1
    }
}

// Written from Python's `decimal` with 80 digits.
/// `e^(j/32)` for j from -12 to 12.
const EXP_TABLE: [Dd; 25] = [
    Dd(0.6872892787909722, -3.7088003061371396e-17),
    Dd(0.7091061824373984, -1.2868055655346304e-17),
    Dd(0.7316156289466418, 8.35576468031604e-18),
    Dd(0.7548396019890073, -9.844076038651084e-18),
    Dd(0.7788007830714049, -1.0231869534531498e-17),
    Dd(0.8035225736890608, -3.661886830920417e-17),
    Dd(0.8290291181804004, -2.7604408719539223e-17),
    Dd(0.8553453273074225, 1.7204900005057594e-17),
    Dd(0.8824969025845955, -5.224526916735663e-17),
    Dd(0.9105103613800342, -3.325048324577564e-17),
    Dd(0.9394130628134758, -2.152447043447057e-17),
    Dd(0.9692332344763441, -4.801151707083219e-17),
    Dd(1.0, 0.0),
    Dd(1.0317434074991028, -8.944417741043132e-17),
    Dd(1.0644944589178593, 1.0872888143211957e-16),
    Dd(1.0982851403078258, 9.070644949793751e-17),
    Dd(1.1331484530668263, -5.370737708558031e-18),
    Dd(1.1691184461695043, 6.945488167320411e-17),
    Dd(1.2062302494209807, 3.9295715071105525e-17),
    Dd(1.2445201077660952, -7.440512295261056e-17),
    Dd(1.2840254166877414, 8.968972781793724e-17),
    Dd(1.3247847587288655, 9.422682377542367e-17),
    Dd(1.3668379411737963, 5.1449446596411544e-17),
    Dd(1.4102260349257107, -4.1758810273684196e-17),
    Dd(1.4549914146182013, 8.517923078996071e-17),
];
/// `ln(1 + j/64)` for j from -19 to 27.
const LN_TABLE: [Dd; 47] = [
    Dd(-0.3522205935893521, -5.7233316949182485e-18),
    Dd(-0.33024168687057687, 1.0828321637483858e-17),
    Dd(-0.3087354816496133, 1.6199186085148102e-17),
    Dd(-0.2876820724517809, -2.607160616442564e-17),
    Dd(-0.26706278524904525, 7.32891532732017e-18),
    Dd(-0.24686007793152578, -1.361743371748368e-17),
    Dd(-0.22705745063534608, -9.551415762738488e-18),
    Dd(-0.2076393647782445, -1.2053243216686129e-17),
    Dd(-0.18859116980755003, 7.432164219196925e-18),
    Dd(-0.16989903679539747, 4.868008764439071e-19),
    Dd(-0.15154989812720093, -5.1669593684615594e-18),
    Dd(-0.13353139262452263, 3.664457663660085e-18),
    Dd(-0.1158318155251217, -4.338484369808096e-18),
    Dd(-0.09844007281325252, 4.439009633675136e-18),
    Dd(-0.0813456394539524, -5.07707635593117e-18),
    Dd(-0.06453852113757118, 6.470486661692933e-18),
    Dd(-0.048009219186360606, -1.4390903347292205e-18),
    Dd(-0.0317486983145803, -3.0382263084680858e-18),
    Dd(-0.015748356968139168, -1.0021578630528974e-18),
    Dd(0.0, 0.0),
    Dd(0.015504186535965254, -3.278321022892429e-19),
    Dd(0.030771658666753687, 1.0431732029005968e-18),
    Dd(0.0458095360312942, 1.902959866474257e-18),
    Dd(0.06062462181643484, 2.6424025938726934e-18),
    Dd(0.07522342123758753, -5.930604196293241e-18),
    Dd(0.08961215868968714, -5.4268129336647135e-18),
    Dd(0.10379679368164356, 5.47772415726659e-18),
    Dd(0.11778303565638346, -1.1971685747593677e-18),
    Dd(0.13157635778871926, 1.1123000879729588e-17),
    Dd(0.1451820098444979, 8.242418783022475e-18),
    Dd(0.15860503017663857, 1.1257003872182592e-17),
    Dd(0.17185025692665923, -6.0224538210113705e-18),
    Dd(0.184922338494012, 3.0236614153574064e-18),
    Dd(0.19782574332991987, 1.2821194372980142e-17),
    Dd(0.21056476910734964, -4.249405314729895e-18),
    Dd(0.22314355131420976, -9.091270597324799e-18),
    Dd(0.2355660713127669, -2.3943371495187355e-18),
    Dd(0.24783616390458127, -1.2432209578702523e-17),
    Dd(0.25995752443692605, 2.069806938978935e-17),
    Dd(0.27193371548364176, 7.83319637697442e-19),
    Dd(0.2837681731306446, -2.032665581126656e-17),
    Dd(0.2954642128938359, -2.16461086040599e-17),
    Dd(0.3070250352949119, -1.2319916200101964e-17),
    Dd(0.3184537311185346, 2.7114779367326236e-17),
    Dd(0.329753286372468, 2.122020616196946e-18),
    Dd(0.3409265869705932, 1.7467136443544747e-17),
    Dd(0.3519764231571782, -1.2953893030191963e-17),
];
/// `1/n!` for n from 0 to 13.
const INVERSE_FACTORIALS: [Dd; 14] = [
    Dd(1.0, 0.0),
    Dd(1.0, 0.0),
    Dd(0.5, 0.0),
    Dd(0.16666666666666666, 9.25185853854297e-18),
    Dd(0.041666666666666664, 2.3129646346357427e-18),
    Dd(0.008333333333333333, 1.1564823173178714e-19),
    Dd(0.001388888888888889, -5.300543954373577e-20),
    Dd(0.0001984126984126984, 1.7209558293420705e-22),
    Dd(2.48015873015873e-05, 2.1511947866775882e-23),
    Dd(2.7557319223985893e-06, -1.858393274046472e-22),
    Dd(2.755731922398589e-07, 2.3767714622250297e-23),
    Dd(2.505210838544172e-08, -1.448814070935912e-24),
    Dd(2.08767569878681e-09, -1.20734505911326e-25),
    Dd(1.6059043836821613e-10, 1.2585294588752098e-26),
];
/// `1/(2n+1)` for n from 0 to 7.
const INVERSE_ODDS: [Dd; 8] = [
    Dd(1.0, 0.0),
    Dd(0.3333333333333333, 1.850371707708594e-17),
    Dd(0.2, -1.1102230246251566e-17),
    Dd(0.14285714285714285, 7.93016446160826e-18),
    Dd(0.1111111111111111, 6.1679056923619804e-18),
    Dd(0.09090909090909091, -2.523234146875356e-18),
    Dd(0.07692307692307693, -4.270088556250602e-18),
    Dd(0.06666666666666667, 9.251858538542971e-19),
];
const LN2: Dd = Dd(0.6931471805599453, 2.3190468138462996e-17);
const LN10: Dd = Dd(2.302585092994046, -2.1707562233822494e-16);

/// `2^k` times a Float whose result stays a normal Float (exact); `k` up to 1024.
fn times_power_of_2(x: f64, k: i64) -> f64 {
    if k > 1023 {
        return times_power_of_2(x * 2.0, k - 1);
    }
    x * f64::from_bits(((k + 1023) as u64) << 52)
}

/// Where `e^x` is a normal Float: from about 2.2e-308 to 1.8e308.
const EXP_RANGE: core::ops::Range<f64> = -708.39..709.78;

/// `e^x` for a double-double `x` in `EXP_RANGE`: `x = k·ln 2 + j/32 + r` with `|r| ≤ 1/64`,
/// `e^r` by 14 terms of its series; rounded, then moved by `2^k`, exact while the result is a
/// normal Float.
fn exp_dd(x: Dd) -> f64 {
    let k = libm::round(x.0 / LN2.0);
    let r = x.add(LN2.mul(Dd(k, 0.0)).neg());
    let j = libm::round(r.0 * 32.0);
    let r = r.add(Dd(-j / 32.0, 0.0));
    let mut t = INVERSE_FACTORIALS[13];
    for c in INVERSE_FACTORIALS[..13].iter().rev() {
        t = t.mul(r).add(*c);
    }
    times_power_of_2(EXP_TABLE[(j + 12.0) as usize].mul(t).round(), k as i64)
}

/// `ln x` for a normal `x > 0`: `x = m·2^e` with `m` in [√½, √2), `m` near `c = 1 + j/64`,
/// `ln m = ln c + 2·atanh((m-c)/(m+c))`, the last by 8 terms of its series.
fn ln_dd(x: f64) -> Dd {
    let bits = x.to_bits();
    let mut e = ((bits >> 52) & 0x7FF) as i64 - 1023;
    let mut m = f64::from_bits(bits & 0x000F_FFFF_FFFF_FFFF | 0x3FF0_0000_0000_0000);
    if m > core::f64::consts::SQRT_2 {
        m /= 2.0;
        e += 1;
    }
    let j = libm::round((m - 1.0) * 64.0);
    let c = 1.0 + j / 64.0;
    let s = Dd(m - c, 0.0).div(two_sum(m, c));
    let s2 = s.mul(s);
    let mut t = INVERSE_ODDS[7];
    for c in INVERSE_ODDS[..7].iter().rev() {
        t = t.mul(s2).add(*c);
    }
    let atanh = Dd(2.0, 0.0).mul(s).mul(t);
    LN2.mul(Dd(e as f64, 0.0)).add(LN_TABLE[(j + 19.0) as usize]).add(atanh)
}

fn is_normal(x: f64) -> bool {
    x.is_finite() && x.abs() >= f64::MIN_POSITIVE
}

fn exp(x: f64) -> f64 {
    if !EXP_RANGE.contains(&x) {
        return libm::exp(x);
    }
    exp_dd(Dd(x, 0.0))
}

fn ln(x: f64) -> f64 {
    if !(is_normal(x) && x > 0.0) {
        return libm::log(x);
    }
    ln_dd(x).round()
}

fn log10(x: f64) -> f64 {
    if !(is_normal(x) && x > 0.0) {
        return libm::log10(x);
    }
    ln_dd(x).div(LN10).round()
}

/// `x^y = e^(y·ln x)`; a negative `x` with a whole `y` gives its sign.
fn pow(x: f64, y: f64) -> f64 {
    let whole = y == libm::trunc(y);
    if !(is_normal(x) && y.is_finite() && x != 1.0 && y != 0.0) || (x < 0.0 && !whole) {
        return libm::pow(x, y);
    }
    let t = ln_dd(x.abs()).mul(Dd(y, 0.0));
    if !EXP_RANGE.contains(&t.0) {
        return libm::pow(x, y);
    }
    let r = exp_dd(t);
    // An odd whole `y`: `(-2)^3 = -8`; `|y| ≥ 2^53` is even.
    if x < 0.0 && y.abs() < 9007199254740992.0 && libm::fmod(y, 2.0) != 0.0 { -r } else { r }
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

/// `secure.random(from..to)` (PLAN Part 4 "The library after Python, step 3"): like `random`, but
/// from the system's unguessable chance (`fill` puts 8 such bytes in its array).
pub fn secure_random(fill: &mut dyn FnMut(&mut [u8; 8]), from: i64, to: i64) -> i64 {
    let n = (to as u64).wrapping_sub(from as u64);
    let limit = u64::MAX - u64::MAX % n;
    let x = loop {
        let mut b = [0; 8];
        fill(&mut b);
        let x = u64::from_le_bytes(b);
        if x < limit {
            break x;
        }
    };
    (from as u64).wrapping_add(x % n) as i64
}

/// How bytes are written as a text: a number passed to `bytes_text` and `text_bytes`.
pub const BYTE_FORMS: [&str; 5] = ["text", "hex", "base64", "base64url", "sha256"];

/// `data.to_hex` and the others: `none` only for `to_text` of bytes that are not UTF-8.
pub fn bytes_text(form: i64, bytes: &[u8]) -> Option<String> {
    match form {
        0 => String::from_utf8(bytes.to_vec()).ok(),
        1 => Some(hex(bytes)),
        2 | 3 => Some(crate::crypto::base64(bytes, form == 3)),
        _ => Some(hex(&crate::crypto::sha256(bytes))),
    }
}

/// `s.to_bytes` and `List<Int>.from_hex`, `from_base64`: `None` if the text is not such a code.
pub fn text_bytes(form: i64, text: &str) -> Option<Vec<u8>> {
    match form {
        0 => Some(text.as_bytes().to_vec()),
        1 => text
            .as_bytes()
            .chunks(2)
            .map(|c| Some(((*c.first()? as char).to_digit(16)? * 16 + (*c.get(1)? as char).to_digit(16)?) as u8))
            .collect(),
        _ => crate::crypto::from_base64(text, form == 3),
    }
}

/// `secure.token(n)`: `n` bytes of chance, written in hexadecimal (2 digits each).
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `secure.uuid` from 16 bytes of chance: version 4 (RFC 9562), `3f2b8c1e-9d4a-4f6b-8e21-5c7d0a9b1e34`.
pub fn uuid(mut b: [u8; 16]) -> String {
    b[6] = b[6] & 0x0f | 0x40;
    b[8] = b[8] & 0x3f | 0x80;
    let h = hex(&b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// `l.mean` of a List that is not empty (src/kir.rs checks it): its slots, Floats as their bits
/// if `float`. Floats are added with Neumaier's correction, so that `[1e16, 1.0, -1e16]` gives
/// `1.0 / 3`, not 0.
pub fn mean(values: &[i64], float: bool) -> f64 {
    let n = values.len() as f64;
    if !float {
        return wide(values.iter().map(|&v| v as i128).sum()) / n;
    }
    let (mut sum, mut lost) = (0.0f64, 0.0f64);
    for &v in values {
        let x = f64::from_bits(v as u64);
        let t = sum + x;
        lost += if sum.abs() >= x.abs() { (sum - t) + x } else { (x - t) + sum };
        sum = t;
    }
    (sum + lost) / n
}

/// An `i128` as a Float, by its two halves: `as f64` would call `__floattidf`, which the Windows
/// runtime does not have.
fn wide(x: i128) -> f64 {
    (x >> 64) as i64 as f64 * 18446744073709551616.0 + x as u64 as f64
}

/// `x.fixed(digits)`: always that many decimals.
pub fn fixed(x: f64, digits: i64) -> String {
    format!("{:.*}", digits.clamp(0, 30) as usize, x)
}

/// `x.round(digits)`.
pub fn round_to(x: f64, digits: i64) -> f64 {
    fixed(x, digits).parse().unwrap_or(x)
}

/// `l.median` of a List that is not empty: the middle value once sorted, or halfway between the
/// two middle ones.
pub fn median(values: &[i64], float: bool) -> f64 {
    let mut v = values.to_vec();
    let m = v.len() / 2;
    if !float {
        v.sort_unstable();
        return if v.len() % 2 == 1 { v[m] as f64 } else { wide(v[m - 1] as i128 + v[m] as i128) / 2.0 };
    }
    let mut x: Vec<f64> = v.iter().map(|&b| f64::from_bits(b as u64)).collect();
    x.sort_unstable_by(f64::total_cmp);
    if x.len() % 2 == 1 { x[m] } else { x[m - 1] + (x[m] - x[m - 1]) / 2.0 }
}

/// `KOVA_SEED`'s value, if it is a number (it must start with a digit).
pub fn seed(value: Option<&str>) -> Option<u64> {
    value.filter(|s| s.starts_with(|c: char| c.is_ascii_digit())).and_then(|s| s.parse().ok())
}

/// `s.to_date` (`time` false: `2026-10-23`) and `s.to_datetime` (`2026-10-23 14:05:30`): exactly
/// what Kova shows, giving the days (or seconds) since 1970-01-01; `None` if the day does not exist.
pub fn parse_date(s: &str, time: bool) -> Option<i64> {
    let b = s.as_bytes();
    let shape: &[u8] = if time { b"dddd-dd-dd dd:dd:dd" } else { b"dddd-dd-dd" };
    if b.len() != shape.len() || !b.iter().zip(shape).all(|(c, k)| if *k == b'd' { c.is_ascii_digit() } else { c == k }) {
        return None;
    }
    let n = |at: usize, len: usize| b[at..at + len].iter().fold(0, |n, c| n * 10 + (c - b'0') as i64);
    let (y, m, d) = (n(0, 4), n(5, 2), n(8, 2));
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let length = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if y < 1 || !(1..=12).contains(&m) || d < 1 || d > length[m as usize - 1] {
        return None;
    }
    let days = days_from_civil(y, m, d);
    if !time {
        return Some(days);
    }
    let (h, min, sec) = (n(11, 2), n(14, 2), n(17, 2));
    (h <= 23 && min <= 59 && sec <= 59).then(|| days * 86400 + h * 3600 + min * 60 + sec)
}

/// The days since 1970-01-01 of a day (Howard Hinnant's `days_from_civil`, as src/kir.rs emits it).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

/// A `Date` (days since 1970-01-01) or a `DateTime` (seconds) as `print` shows it:
/// `2026-10-25`, `2026-10-25 14:30:00` (src/kir.rs `date_pieces`).
pub fn date_text(v: i64, time: bool) -> String {
    let days = if time { v.div_euclid(86400) } else { v };
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    let day = format!("{y:04}-{m:02}-{d:02}");
    if !time {
        return day;
    }
    let s = v.rem_euclid(86400);
    format!("{day} {:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// The year of a number of days since 1970-01-01.
#[cfg_attr(windows, allow(dead_code))] // Windows asks the system for its local time
fn year_of(days: i64) -> i64 {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    yoe + era * 400 + if (5 * doy + 2) / 153 >= 10 { 1 } else { 0 }
}

/// The seconds to add to `utc` (seconds since 1970) to get the local time, on Linux and macOS:
/// `tz` is `$TZ`, `read` reads a file. A time zone file (`/etc/localtime`) ends with the rule of
/// its present and future years, `CET-1CEST,M3.5.0,M10.5.0/3`, which is all `Date.today` needs;
/// `$TZ` may also be such a rule itself. UTC when nothing is found.
#[cfg_attr(windows, allow(dead_code))] // Windows asks the system for its local time
pub fn local_offset(utc: i64, tz: Option<&str>, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> i64 {
    // The last line of a file of version 2 or later.
    let rule_of = |f: Vec<u8>| -> Option<String> {
        let ok = f.starts_with(b"TZif") && f.len() > 5 && f[4] >= b'2' && f.ends_with(b"\n");
        let line = f[..f.len() - 1].rsplit(|&b| b == b'\n').next()?;
        ok.then(|| String::from_utf8_lossy(line).into())
    };
    let rule = match tz.map(|t| t.strip_prefix(':').unwrap_or(t)).filter(|t| !t.is_empty()) {
        None => read("/etc/localtime").and_then(rule_of),
        Some(t) if t.starts_with('/') => read(t).and_then(rule_of),
        Some(t) => read(&format!("/usr/share/zoneinfo/{t}")).and_then(rule_of).or_else(|| Some(t.into())),
    };
    rule.and_then(|r| offset_by_rule(r.as_bytes(), utc)).unwrap_or(0)
}

/// A POSIX time zone rule read from its start: `CET-1CEST,M3.5.0,M10.5.0/3` = "CET" one hour
/// east of UTC (the sign is the other way round), "CEST" one more hour from March's last Sunday
/// at 2:00 to October's last Sunday at 3:00.
#[cfg_attr(windows, allow(dead_code))] // Windows asks the system for its local time
struct Rule<'a>(&'a [u8]);

#[cfg_attr(windows, allow(dead_code))] // Windows asks the system for its local time
impl Rule<'_> {
    fn peek(&self) -> Option<u8> {
        self.0.first().copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        let found = self.peek() == Some(c);
        if found {
            self.0 = &self.0[1..];
        }
        found
    }

    /// `CET`, or `<+0330>` for a name with signs and digits.
    fn name(&mut self) -> Option<()> {
        let n = if self.eat(b'<') { self.0.iter().position(|&c| c == b'>')? + 1 } else { self.0.iter().take_while(|c| c.is_ascii_alphabetic()).count() };
        (n >= 3).then(|| self.0 = &self.0[n..])
    }

    fn number(&mut self) -> Option<i64> {
        let n = self.0.iter().take_while(|c| c.is_ascii_digit()).count();
        let v = core::str::from_utf8(&self.0[..n]).ok()?.parse().ok()?;
        self.0 = &self.0[n..];
        Some(v)
    }

    /// `-1`, `+5:30`, `2:00:00`: seconds.
    fn time(&mut self) -> Option<i64> {
        let sign = if self.eat(b'-') { -1 } else { self.eat(b'+'); 1 };
        let mut t = self.number()? * 3600;
        for k in [60, 1] {
            if !self.eat(b':') {
                break;
            }
            t += self.number()? * k;
        }
        Some(sign * t)
    }

    /// `,M3.5.0/3` (month, week 1 to 5 = the last, day 0 = Sunday), `,J60` (1 to 365, never
    /// February 29), `,59` (0 to 365): that day of `year` at its time, in local seconds.
    fn change(&mut self, year: i64) -> Option<i64> {
        if !self.eat(b',') {
            return None;
        }
        let jan1 = days_from_civil(year, 1, 1);
        let leap = days_from_civil(year, 3, 1) - days_from_civil(year, 2, 1) == 29;
        let day = if self.eat(b'M') {
            let m = self.number()?;
            self.eat(b'.').then_some(())?;
            let w = self.number()?;
            self.eat(b'.').then_some(())?;
            let d = self.number()?;
            let first = days_from_civil(year, m, 1);
            let length = days_from_civil(year + m / 12, m % 12 + 1, 1) - first;
            // 1970-01-01 was a Thursday (4).
            let mut day = first + (d - (first + 4).rem_euclid(7)).rem_euclid(7) + 7 * (w - 1);
            while day >= first + length {
                day -= 7;
            }
            day
        } else if self.eat(b'J') {
            let n = self.number()?;
            jan1 + n - 1 + if leap && n >= 60 { 1 } else { 0 }
        } else {
            jan1 + self.number()?
        };
        let time = if self.eat(b'/') { self.time()? } else { 2 * 3600 };
        Some(day * 86400 + time)
    }
}

/// The offset `rule` gives at `utc` (see `Rule`); `None` if it cannot be read.
#[cfg_attr(windows, allow(dead_code))] // Windows asks the system for its local time
fn offset_by_rule(rule: &[u8], utc: i64) -> Option<i64> {
    let mut r = Rule(rule);
    r.name()?;
    let standard = -r.time()?;
    if r.peek().is_none() {
        return Some(standard);
    }
    r.name()?;
    let summer = if matches!(r.peek(), Some(b'+' | b'-' | b'0'..=b'9')) { -r.time()? } else { standard + 3600 };
    // Without dates: the United States' (second Sunday of March, first of November).
    let dates = if r.peek().is_none() { ",M3.2.0,M11.1.0".as_bytes() } else { r.0 };
    let year = year_of((utc + standard).div_euclid(86400));
    let mut d = Rule(dates);
    // The start is written in standard time, the end in summer time.
    let start = d.change(year)? - standard;
    let end = d.change(year)? - summer;
    let inside = if start < end { start <= utc && utc < end } else { !(end <= utc && utc < start) };
    Some(if inside { summer } else { standard })
}

/// `s.wrapped(width)` (PLAN Part 4 "The library after Python, step 2"): each line of `s` cut
/// between words so that it holds at most `width` characters, words joined by one space. The
/// line ends already there stay; a word longer than `width` stays whole on its own line.
pub fn wrapped(s: &str, width: i64) -> String {
    let mut out = String::new();
    for (k, line) in s.split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
        }
        let mut length = 0;
        for word in line.split_ascii_whitespace() {
            let n = word.chars().count() as i64;
            if length > 0 && length + 1 + n > width {
                out.push('\n');
                length = 0;
            } else if length > 0 {
                out.push(' ');
                length += 1;
            }
            out.push_str(word);
            length += n;
        }
    }
    out
}

/// `"x".red`, `"x".bold`… (PLAN Part 4 "The library after Python, step 5"): each style's ANSI code.
pub const PAINTS: &[(&str, i64)] =
    &[("red", 31), ("green", 32), ("yellow", 33), ("blue", 34), ("magenta", 35), ("cyan", 36), ("gray", 90), ("bold", 1), ("underline", 4)];

/// `s` in the style `code`, when `on`. Each reset inside `s` takes the style back, so that
/// `"a {b.green} c".red` is red again after `b`.
pub fn painted(s: &str, code: i64, on: bool) -> String {
    if !on {
        return s.into();
    }
    let style = format!("\x1b[{code}m");
    format!("{style}{}\x1b[0m", s.replace("\x1b[0m", &format!("\x1b[0m{style}")))
}

/// Whether texts are painted: only for a terminal (not a file or a pipe), unless `NO_COLOR` is set
/// (no-color.org) or `TERM=dumb`; `FORCE_COLOR` (not `0`) paints anyway.
pub fn colors(terminal: bool, var: impl Fn(&str) -> Option<String>) -> bool {
    if var("FORCE_COLOR").is_some_and(|v| v != "0") {
        return true;
    }
    terminal && var("NO_COLOR").is_none_or(|v| v.is_empty()) && var("TERM").as_deref() != Some("dumb")
}

/// The cases of `log.Level`, numbered from 0, as `KOVA_LOG` names them.
pub const LEVELS: [&str; 4] = ["debug", "info", "warning", "error"];

/// The level `KOVA_LOG` asks for: `Ok(None)` when it is not set, `Err` when it names no level.
pub fn level_named(v: Option<&str>) -> Result<Option<i64>, ()> {
    match v.map(|v| v.trim().to_ascii_lowercase()) {
        None => Ok(None),
        Some(v) if v.is_empty() => Ok(None),
        Some(v) => LEVELS.iter().position(|l| *l == v).map(|l| Some(l as i64)).ok_or(()),
    }
}

/// The line `log.info(text)` writes on stderr, like the server's lines: local time (seconds since
/// 1970, wall clock), level, text.
pub fn log_line(now: i64, level: i64, text: &str) -> String {
    let t = now.rem_euclid(86400);
    format!("{:02}:{:02}:{:02}  {:<7}  {text}\n", t / 3600, t / 60 % 60, t % 60, LEVELS[level as usize])
}

/// The files `extern sqlite from "sqlite3"` tries, in order, on `system` (`linux`, `macos`,
/// `windows`; PLAN Part 4 "`extern`, step 1"). On Linux the plain `.so` comes only with a -dev
/// package; on macOS the system's libraries are in the dyld cache, not on the disk.
#[allow(dead_code)] // the runtime's only: `kova run` runs such a program as native code
pub fn library_files(name: &str, system: &str) -> Vec<String> {
    let written = name.contains(['/', '\\']) || [".so", ".dll", ".dylib"].iter().any(|e| name.ends_with(e) || name.contains(".so."));
    if written {
        return alloc::vec![name.into()];
    }
    match (system, name) {
        ("linux", "c" | "m") => alloc::vec![format!("lib{name}.so.6")],
        ("macos", "c" | "m") => alloc::vec!["/usr/lib/libSystem.B.dylib".into()],
        ("windows", "c" | "m") => alloc::vec!["ucrtbase.dll".into()],
        ("linux", _) => core::iter::once(format!("lib{name}.so")).chain((0..10).map(|v| format!("lib{name}.so.{v}"))).collect(),
        ("macos", _) => alloc::vec![
            format!("lib{name}.dylib"),
            format!("/usr/lib/lib{name}.dylib"),
            format!("/opt/homebrew/lib/lib{name}.dylib"),
            format!("/usr/local/lib/lib{name}.dylib"),
            format!("/System/Library/Frameworks/{name}.framework/{name}"),
        ],
        _ => alloc::vec![format!("{name}.dll"), format!("lib{name}.dll")],
    }
}

/// A view of `import ui` (src/ui.rs), read back from its text: `T5:hello` a text, `B2:Go` a
/// button, `I3:Ada` a text field holding `Ada`, `CL2:` a column placing its parts on the left, then its 2 parts, `RT…` a row (`Align`'s first letter).
/// Buttons are numbered in the order they come, fields too, apart. The lengths count
/// characters, as Kova's `length` does.
#[allow(dead_code)] // a button's number: the runtime's
pub enum UiNode {
    Text(String),
    Button(String, usize),
    Input(String, usize),
    Column(Vec<UiNode>, UiAlign),
    Row(Vec<UiNode>, UiAlign),
    /// An image file and its width (0: the file's own).
    Image(String, i32),
    /// A text, button or field with a style of its own (a button keeps the system's text color).
    Styled(UiStyle, alloc::boxed::Box<UiNode>),
    /// Empty pixels around a view.
    Spaced(i32, alloc::boxed::Box<UiNode>),
    /// A drawing area.
    Canvas(UiCanvas),
    /// A view and a clock: every so many milliseconds, the button number's event.
    Every(i32, usize, alloc::boxed::Box<UiNode>),
}

/// `ui.canvas`: its size, its drawing, and the numbers (counted with the fields) of what hears the
/// mouse and the keyboard.
#[derive(Clone, Default)]
pub struct UiCanvas {
    /// 0: all the room there is (`ui.fill`).
    pub width: i32,
    pub height: i32,
    /// Its place among the view's canvases: `ui.room` asks by it.
    #[allow(dead_code)]
    pub number: usize,
    pub ops: Vec<UiOp>,
    pub mouse: Option<usize>,
    pub key: Option<usize>,
}

/// One thing drawn. An outline of 0 fills the shape. Colors are `0xRRGGBB`.
#[derive(Clone, PartialEq, Debug)]
pub enum UiOp {
    Clear(u32),
    Rect { x: i32, y: i32, w: i32, h: i32, color: u32, outline: i32 },
    Circle { x: i32, y: i32, r: i32, color: u32, outline: i32 },
    Line { x: i32, y: i32, x2: i32, y2: i32, color: u32, thickness: i32 },
    Text { x: i32, y: i32, size: i32, color: u32, text: String },
}

/// What `ui.draw` gathered, one line per thing drawn (src/ui.rs).
fn ui_ops(text: &str) -> Vec<UiOp> {
    let mut chars = text.chars();
    let mut ops = Vec::new();
    let signed = |chars: &mut core::str::Chars| {
        let mut n = 0i32;
        let mut minus = false;
        for c in chars.by_ref() {
            match c {
                '-' => minus = true,
                '0'..='9' => n = n.saturating_mul(10).saturating_add(c as i32 - '0' as i32),
                _ => break, // the space, or the `:` of a text's length
            }
        }
        if minus { -n } else { n }
    };
    while let Some(kind) = chars.next() {
        chars.next();
        let mut n = |count: usize| {
            let mut out = [0; 5];
            out[..count].iter_mut().for_each(|o| *o = signed(&mut chars));
            out
        };
        ops.push(match kind {
            'C' => UiOp::Clear(n(1)[0] as u32),
            'R' => {
                let [x, y, w, h, color] = n(5);
                UiOp::Rect { x, y, w, h, color: color as u32, outline: n(1)[0] }
            }
            'O' => {
                let [x, y, r, color, outline] = n(5);
                UiOp::Circle { x, y, r, color: color as u32, outline }
            }
            'L' => {
                let [x, y, x2, y2, color] = n(5);
                UiOp::Line { x, y, x2, y2, color: color as u32, thickness: n(1)[0] }
            }
            _ => {
                let [x, y, size, color, length] = n(5);
                let text = chars.by_ref().take(length as usize).collect();
                chars.next();
                UiOp::Text { x, y, size, color: color as u32, text }
            }
        });
    }
    ops
}

/// What `ui.typed` gives after a gesture on a canvas (src/ui.rs `gesture`). `kind` is `press`,
/// `release` or `move`; `button` 1 left, 2 right, 0 none.
#[allow(dead_code)]
pub fn ui_mouse(kind: &str, x: i32, y: i32, button: i32) -> String {
    format!("{kind} {x} {y} {button}")
}

/// The same, for a key.
#[allow(dead_code)]
pub fn ui_key(name: &str) -> String {
    format!("key {name}")
}

/// What canvases and clocks heard, until the program asks: (`ui.show`'s answer, `ui.typed`'s text).
/// A program slower than the mouse or a clock keeps only the last move and one tick; a press is
/// never lost.
#[allow(dead_code)]
pub struct UiQueue(Vec<(i64, String)>);

#[allow(dead_code)]
impl UiQueue {
    pub const fn new() -> UiQueue {
        UiQueue(Vec::new())
    }
    pub fn hear(&mut self, answer: i64, text: String) {
        if text.is_empty() && self.0.iter().any(|w| w.0 == answer) {
            return;
        }
        match self.0.last_mut() {
            Some(last) if last.0 == answer && last.1.starts_with("move ") && text.starts_with("move ") => last.1 = text,
            _ => self.0.push((answer, text)),
        }
    }
    pub fn next(&mut self) -> Option<(i64, String)> {
        (!self.0.is_empty()).then(|| self.0.remove(0))
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What `ui.act(n)` asks of the window, in words: a test sees it in the output. `0`: no system
/// frame, said before the window opens.
#[allow(dead_code)]
pub fn ui_act_name(act: i64) -> &'static str {
    match act {
        0 => "no frame",
        1 => "close",
        2 => "minimize",
        3 => "drag",
        4 => "maximize",
        5 => "full screen",
        _ => "place",
    }
}

/// `ui.window` then `ui.screen` (x, y, width, height) before a window has said, and in test mode.
#[allow(dead_code)]
pub const UI_AREAS_GUESS: [i32; 8] = [0, 0, 320, 200, 0, 0, 1920, 1080];

/// What `ui.show` gives back when a canvas took another size (src/ui.rs `run`): no button has it.
#[allow(dead_code)]
pub const UI_RESIZED: i64 = 1 << 40;

/// Everything that ticks in a view: (milliseconds, button number).
#[allow(dead_code)]
pub fn ui_clocks(node: &UiNode) -> Vec<(i32, usize)> {
    match node {
        UiNode::Column(parts, _) | UiNode::Row(parts, _) => parts.iter().flat_map(ui_clocks).collect(),
        UiNode::Spaced(_, inner) | UiNode::Styled(_, inner) => ui_clocks(inner),
        UiNode::Every(ms, n, inner) => [alloc::vec![(*ms, *n)], ui_clocks(inner)].concat(),
        _ => Vec::new(),
    }
}

/// How a text, button or field looks. `size` 0: the system's.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct UiStyle {
    pub color: Option<u32>,
    pub size: i32,
    pub bold: bool,
}

impl UiStyle {
    /// This style, what it leaves unsaid taken from `around`.
    fn over(self, around: UiStyle) -> UiStyle {
        UiStyle {
            color: self.color.or(around.color),
            size: if self.size > 0 { self.size } else { around.size },
            bold: self.bold || around.bold,
        }
    }
}

/// Where a column places its parts across its width, a row across its height.
#[derive(Clone, Copy, PartialEq)]
pub enum UiAlign {
    Start,
    Center,
    End,
}

#[allow(dead_code)] // the runtime's
impl UiAlign {
    /// How far a part goes from the start, with `free` pixels left around it.
    pub fn offset(self, free: i32) -> i32 {
        match self {
            UiAlign::Start => 0,
            UiAlign::Center => free / 2,
            UiAlign::End => free,
        }
    }
}

/// A column that places its parts takes all the width it is given: there is no other way to
/// have something in the middle of the window.
#[allow(dead_code)]
pub fn ui_fills(node: &UiNode) -> bool {
    match node {
        UiNode::Column(parts, a) => *a != UiAlign::Start || parts.iter().any(ui_fills),
        UiNode::Row(parts, _) => parts.iter().any(ui_fills),
        UiNode::Spaced(_, inner) | UiNode::Every(_, _, inner) => ui_fills(inner),
        UiNode::Canvas(c) => c.width == 0,
        _ => false,
    }
}

/// The same, down: a `ui.fill` height somewhere inside.
#[allow(dead_code)]
pub fn ui_fills_down(node: &UiNode) -> bool {
    match node {
        UiNode::Column(parts, _) | UiNode::Row(parts, _) => parts.iter().any(ui_fills_down),
        UiNode::Spaced(_, inner) | UiNode::Every(_, _, inner) => ui_fills_down(inner),
        UiNode::Canvas(c) => c.height == 0,
        _ => false,
    }
}

#[allow(dead_code)] // the interpreter only uses `ui_describe`
pub fn ui_tree(text: &str) -> UiNode {
    let mut chars = text.chars();
    ui_node(&mut chars, &mut [0, 0, 0], UiStyle::default())
}

/// What `ui.show` gives back (src/ui.rs): `-1` the window closed, `n` button `n` clicked,
/// `-2 - k` field `k` changed, its text then given by `ui.typed`.
#[allow(dead_code)]
pub fn ui_changed(field: usize) -> i64 {
    -2 - field as i64
}

/// A view's shape, `C2(T,R2(B,B))`: when it has not changed, a window keeps its widgets and only
/// changes their texts and styles, so a field being typed into keeps its cursor (even one that
/// turns red while it is wrong).
#[allow(dead_code)]
pub fn ui_shape(node: &UiNode) -> String {
    match node {
        UiNode::Text(_) => "T".into(),
        UiNode::Button(..) => "B".into(),
        UiNode::Input(..) => "I".into(),
        UiNode::Column(parts, a) | UiNode::Row(parts, a) => {
            let inside: Vec<String> = parts.iter().map(ui_shape).collect();
            let kind = if matches!(node, UiNode::Row(..)) { 'R' } else { 'C' };
            format!("{kind}{}{}({})", *a as u8, parts.len(), inside.join(","))
        }
        UiNode::Image(path, width) => format!("P{width}:{path}"),
        UiNode::Styled(_, inner) => ui_shape(inner),
        UiNode::Spaced(m, inner) => format!("M{m}({})", ui_shape(inner)),
        // Not its drawing, which changes at every frame.
        UiNode::Canvas(c) => format!("D{}x{}:{:?}{:?}", c.width, c.height, c.mouse, c.key),
        // Nor its clocks: each system compares those itself, widgets stay when one starts.
        UiNode::Every(_, _, inner) => ui_shape(inner),
    }
}

/// The texts, buttons and fields of a view, in order, each with its style.
#[allow(dead_code)]
pub fn ui_leaves(node: &UiNode) -> Vec<(&UiNode, UiStyle)> {
    match node {
        UiNode::Column(parts, _) | UiNode::Row(parts, _) => parts.iter().flat_map(ui_leaves).collect(),
        UiNode::Spaced(_, inner) | UiNode::Every(_, _, inner) => ui_leaves(inner),
        UiNode::Styled(style, leaf) => alloc::vec![(&**leaf, *style)],
        leaf => alloc::vec![(leaf, UiStyle::default())],
    }
}

/// A line of `KOVA_UI_SCRIPT`'s input: `3` clicks button 3, `0=Ada` types `Ada` into field 0;
/// anything else closes the window.
#[allow(dead_code)]
pub fn ui_answer(line: &str) -> (i64, Option<String>) {
    let line = line.trim_end_matches(['\n', '\r']);
    match line.split_once('=') {
        Some((k, text)) => match k.trim().parse() {
            Ok(k) => (ui_changed(k), Some(text.into())),
            Err(_) => (-1, None),
        },
        None => (line.trim().parse().unwrap_or(-1), None),
    }
}

/// The number before the next `:`.
fn ui_number(chars: &mut core::str::Chars) -> usize {
    let mut n = 0usize;
    for c in chars.by_ref() {
        match c.to_digit(10) {
            Some(d) => n = n * 10 + d as usize,
            None => break, // the `:`
        }
    }
    n
}

fn ui_node(chars: &mut core::str::Chars, counts: &mut [usize; 3], around: UiStyle) -> UiNode {
    let (mut own, mut margin) = (UiStyle::default(), None);
    // What listens, the last one written first.
    let mut listening = Vec::new();
    let mut kind = chars.next().unwrap_or('C');
    while kind == 'S' {
        let key = chars.next();
        let n = ui_number(chars);
        match key {
            Some('c') => own.color = own.color.or(Some(n as u32)),
            Some('s') if own.size == 0 => own.size = n as i32,
            Some('b') => own.bold = true,
            Some('m') => margin = margin.or(Some(n as i32)),
            Some(key @ ('M' | 'K' | 'E')) => listening.push((key, n as i32)),
            _ => {}
        }
        kind = chars.next().unwrap_or('C');
    }
    let style = own.over(around);
    let mut node = ui_part(kind, chars, counts, style);
    // Numbered after the view's own, as src/ui.rs adds them to its lists.
    let mut clocks = Vec::new();
    for (key, ms) in listening.into_iter().rev() {
        let count = &mut counts[(key != 'E') as usize];
        *count += 1;
        match (key, &mut node) {
            ('M', UiNode::Canvas(c)) => c.mouse = Some(*count - 1),
            ('K', UiNode::Canvas(c)) => c.key = Some(*count - 1),
            _ => clocks.push((ms, *count - 1)),
        }
    }
    let node = match kind {
        // A size on the image itself: a column's text size is not an image's width.
        'P' => match node {
            UiNode::Image(path, _) => UiNode::Image(path, own.size),
            other => other,
        },
        'B' if style.size > 0 || style.bold => UiNode::Styled(UiStyle { color: None, ..style }, node.into()),
        'T' | 'I' if style != UiStyle::default() => UiNode::Styled(style, node.into()),
        _ => node,
    };
    let node = match margin {
        Some(m) => UiNode::Spaced(m, node.into()),
        None => node,
    };
    clocks.into_iter().fold(node, |inner, (ms, n)| UiNode::Every(ms, n, inner.into()))
}

fn ui_part(kind: char, chars: &mut core::str::Chars, counts: &mut [usize; 3], style: UiStyle) -> UiNode {
    let align = if matches!(kind, 'T' | 'B' | 'I' | 'P' | 'D') {
        UiAlign::Start
    } else {
        match chars.next() {
            Some('C') => UiAlign::Center,
            Some('R' | 'B') => UiAlign::End,
            _ => UiAlign::Start,
        }
    };
    let n = ui_number(chars);
    match kind {
        'D' => {
            let body: String = chars.by_ref().take(n).collect();
            let (size, ops) = body.split_once('\n').unwrap_or((&body, ""));
            let (width, height) = size.split_once(' ').unwrap_or((size, ""));
            counts[2] += 1;
            UiNode::Canvas(UiCanvas { width: width.parse().unwrap_or(1), height: height.parse().unwrap_or(1), number: counts[2] - 1, ops: ui_ops(ops), ..UiCanvas::default() })
        }
        'T' | 'B' | 'I' | 'P' => {
            let label: String = chars.by_ref().take(n).collect();
            match kind {
                'T' => return UiNode::Text(label),
                'P' => return UiNode::Image(label, 0),
                _ => {}
            }
            let count = &mut counts[(kind == 'I') as usize];
            *count += 1;
            if kind == 'I' { UiNode::Input(label, *count - 1) } else { UiNode::Button(label, *count - 1) }
        }
        _ => {
            let parts = (0..n).map(|_| ui_node(chars, counts, style)).collect();
            if kind == 'R' { UiNode::Row(parts, align) } else { UiNode::Column(parts, align) }
        }
    }
}

/// `column(text "0 clics", button "Go")`: what `KOVA_UI_SCRIPT` prints instead of a window.
pub fn ui_describe(text: &str) -> String {
    fn write(node: &UiNode, out: &mut String) {
        let group = |name: &str, parts: &[UiNode], out: &mut String| {
            out.push_str(name);
            out.push('(');
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write(p, out);
            }
            out.push(')');
        };
        match node {
            UiNode::Text(t) => out.push_str(&format!("text {t:?}")),
            UiNode::Button(t, _) => out.push_str(&format!("button {t:?}")),
            UiNode::Input(t, _) => out.push_str(&format!("input {t:?}")),
            UiNode::Column(parts, UiAlign::Start) => group("column", parts, out),
            UiNode::Column(parts, UiAlign::Center) => group("column center", parts, out),
            UiNode::Column(parts, UiAlign::End) => group("column right", parts, out),
            UiNode::Row(parts, UiAlign::Start) => group("row", parts, out),
            UiNode::Row(parts, UiAlign::Center) => group("row center", parts, out),
            UiNode::Row(parts, UiAlign::End) => group("row bottom", parts, out),
            UiNode::Image(path, 0) => out.push_str(&format!("image {path:?}")),
            UiNode::Image(path, width) => out.push_str(&format!("image {path:?} size({width})")),
            UiNode::Styled(style, leaf) => {
                write(leaf, out);
                if let Some(c) = style.color {
                    out.push_str(&format!(" color(#{c:06x})"));
                }
                if style.size > 0 {
                    out.push_str(&format!(" size({})", style.size));
                }
                if style.bold {
                    out.push_str(" bold");
                }
            }
            UiNode::Spaced(m, inner) => {
                write(inner, out);
                out.push_str(&format!(" margin({m})"));
            }
            UiNode::Every(ms, _, inner) => {
                write(inner, out);
                out.push_str(&format!(" every({ms})"));
            }
            UiNode::Canvas(c) => {
                let side = |n: i32| if n == 0 { "fill".into() } else { format!("{n}") };
                out.push_str(&format!("canvas {}x{}", side(c.width), side(c.height)));
                for op in &c.ops {
                    out.push_str(&match op {
                        UiOp::Clear(c) => format!(" clear(#{c:06x})"),
                        UiOp::Rect { x, y, w, h, color, outline } => format!(" rect({x} {y} {w} {h} #{color:06x} {outline})"),
                        UiOp::Circle { x, y, r, color, outline } => format!(" circle({x} {y} {r} #{color:06x} {outline})"),
                        UiOp::Line { x, y, x2, y2, color, thickness } => format!(" line({x} {y} {x2} {y2} #{color:06x} {thickness})"),
                        UiOp::Text { x, y, size, color, text } => format!(" text({x} {y} {size} #{color:06x} {text:?})"),
                    });
                }
            }
        }
    }
    let mut out = String::new();
    write(&ui_tree(text), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraries() {
        assert_eq!(library_files("sqlite3", "linux")[..3], ["libsqlite3.so", "libsqlite3.so.0", "libsqlite3.so.1"]);
        assert_eq!(library_files("m", "linux"), ["libm.so.6"]);
        assert_eq!(library_files("libgtk-4.so.1", "linux"), ["libgtk-4.so.1"]);
        assert_eq!(library_files("sqlite3", "windows"), ["sqlite3.dll", "libsqlite3.dll"]);
        assert_eq!(library_files("C:\\x\\a.dll", "windows"), ["C:\\x\\a.dll"]);
        assert_eq!(library_files("AppKit", "macos")[4], "/System/Library/Frameworks/AppKit.framework/AppKit");
    }

    #[test]
    fn ui_gestures_wait_in_order() {
        let mut q = UiQueue::new();
        q.hear(5, String::new());
        q.hear(-2, ui_mouse("move", 1, 1, 0));
        q.hear(-2, ui_mouse("move", 2, 2, 0));
        q.hear(-2, ui_mouse("press", 2, 2, 1));
        q.hear(5, String::new());
        q.hear(-2, ui_mouse("move", 3, 3, 0));
        assert_eq!(q.next(), Some((5, String::new())));
        assert_eq!(q.next(), Some((-2, "move 2 2 0".into())));
        assert_eq!(q.next(), Some((-2, "press 2 2 1".into())));
        assert_eq!(q.next(), Some((-2, "move 3 3 0".into())));
        assert_eq!(q.next(), None);
    }

    #[test]
    fn ui_canvas_read_back() {
        let tree = ui_tree("SE16:SK0:SM0:Sm4:D39:3 2\nC 5\nT 1 -2 9 0 3:a b\nL 0 0 1 1 7 2\n");
        let UiNode::Every(16, 0, spaced) = tree else { panic!() };
        let UiNode::Spaced(4, canvas) = *spaced else { panic!() };
        let UiNode::Canvas(c) = *canvas else { panic!() };
        assert_eq!((c.width, c.height, c.mouse, c.key), (3, 2, Some(0), Some(1)));
        assert_eq!(c.ops[1], UiOp::Text { x: 1, y: -2, size: 9, color: 0, text: "a b".into() });
        assert_eq!(c.ops[2], UiOp::Line { x: 0, y: 0, x2: 1, y2: 1, color: 7, thickness: 2 });
    }

    #[test]
    fn colors_and_log() {
        assert_eq!(painted("a", 31, true), "\x1b[31ma\x1b[0m");
        assert_eq!(painted(&format!("a {} c", painted("b", 32, true)), 31, true), "\x1b[31ma \x1b[32mb\x1b[0m\x1b[31m c\x1b[0m");
        assert_eq!(painted("a", 31, false), "a");
        let env = |pairs: &'static [(&str, &str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert!(colors(true, env(&[])) && !colors(false, env(&[])));
        assert!(!colors(true, env(&[("NO_COLOR", "1")])) && colors(true, env(&[("NO_COLOR", "")])) && !colors(true, env(&[("TERM", "dumb")])));
        assert!(colors(false, env(&[("FORCE_COLOR", "1")])) && !colors(false, env(&[("FORCE_COLOR", "0")])));
        assert_eq!(level_named(Some(" Warning ")), Ok(Some(2)));
        assert_eq!((level_named(None), level_named(Some(""))), (Ok(None), Ok(None)));
        assert_eq!(level_named(Some("verbose")), Err(()));
        assert_eq!(log_line(86400 + 3600 + 62, 1, "hi"), "01:01:02  info     hi\n");
    }

    #[test]
    fn chance_and_numbers() {
        assert_eq!(uuid([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert_eq!(hex(&[0, 171]), "00ab");
        let mut k = 0u8;
        let mut fill = |b: &mut [u8; 8]| {
            k += 1;
            *b = [k; 8];
        };
        assert!((0..100).all(|_| (1..7).contains(&secure_random(&mut fill, 1, 7))));
        let f = |x: &[f64]| x.iter().map(|v| v.to_bits() as i64).collect::<Vec<_>>();
        assert_eq!(mean(&f(&[1e16, 1.0, -1e16]), true), 1.0 / 3.0);
        assert_eq!(mean(&[1, 2], false), 1.5);
        assert_eq!(mean(&[i64::MAX, i64::MAX], false), i64::MAX as f64);
        assert_eq!(median(&[3, 1, 2], false), 2.0);
        assert_eq!(median(&[4, 1, 3, 2], false), 2.5);
        assert_eq!(median(&f(&[0.5, -1.0]), true), -0.25);
    }

    #[test]
    fn time_zone_rules() {
        let at = |rule: &str, y: i64, m: i64, d: i64, h: i64, min: i64| offset_by_rule(rule.as_bytes(), days_from_civil(y, m, d) * 86400 + h * 3600 + min * 60);
        let paris = "CET-1CEST,M3.5.0,M10.5.0/3";
        // 2026: summer time from March 29 at 1:00 UTC to October 25 at 1:00 UTC.
        assert_eq!(at(paris, 2026, 3, 29, 0, 59), Some(3600));
        assert_eq!(at(paris, 2026, 3, 29, 1, 0), Some(7200));
        assert_eq!(at(paris, 2026, 10, 25, 0, 59), Some(7200));
        assert_eq!(at(paris, 2026, 10, 25, 1, 0), Some(3600));
        assert_eq!(at(paris, 2026, 1, 1, 12, 0), Some(3600));
        // Southern: summer time from October 4 2026 at 2:00 to April 5 2026 at 3:00 (local).
        let sydney = "AEST-10AEDT,M10.1.0,M4.1.0/3";
        assert_eq!(at(sydney, 2026, 4, 4, 15, 59), Some(39600));
        assert_eq!(at(sydney, 2026, 4, 4, 16, 0), Some(36000));
        assert_eq!(at(sydney, 2026, 10, 3, 15, 59), Some(36000));
        assert_eq!(at(sydney, 2026, 10, 3, 16, 0), Some(39600));
        assert_eq!(at(sydney, 2026, 12, 31, 13, 0), Some(39600));
        // New York without dates: March 8 2026 at 7:00 UTC, November 1 at 6:00 UTC.
        assert_eq!(at("EST5EDT", 2026, 3, 8, 6, 59), Some(-18000));
        assert_eq!(at("EST5EDT", 2026, 3, 8, 7, 0), Some(-14400));
        assert_eq!(at("EST5EDT", 2026, 11, 1, 5, 59), Some(-14400));
        assert_eq!(at("EST5EDT", 2026, 11, 1, 6, 0), Some(-18000));
        assert_eq!(at("<+0330>-3:30", 2026, 6, 1, 0, 0), Some(12600));
        assert_eq!(at("UTC0", 2026, 6, 1, 0, 0), Some(0));
        assert_eq!(at("<-03>3", 2026, 6, 1, 0, 0), Some(-10800));
        // Day numbers: `J60` is March 1st even in a leap year.
        assert_eq!(at("XST0XDT,J60/0,J300/0", 2024, 2, 29, 23, 59), Some(0));
        assert_eq!(at("XST0XDT,J60/0,J300/0", 2024, 3, 1, 0, 0), Some(3600));
        assert_eq!(at("Europe/Paris", 2026, 6, 1, 0, 0), None);
        // A file: its last line.
        let file = b"TZif2...\n<+0545>-5:45\n".to_vec();
        assert_eq!(local_offset(0, Some("Asia/Kathmandu"), &|p| (p == "/usr/share/zoneinfo/Asia/Kathmandu").then(|| file.clone())), 20700);
        assert_eq!(local_offset(0, Some("Nowhere/Else"), &|_| None), 0);
        assert_eq!(local_offset(0, Some("<+01>-1"), &|_| None), 3600);
        for days in [-719162, -1, 0, 59, 365, 20384, 2932896] {
            assert_eq!(year_of(days), (1..=9999).rev().find(|&y| days_from_civil(y, 1, 1) <= days).unwrap());
        }
    }

    #[test]
    fn ui_views() {
        assert_eq!(ui_describe("CL2:T7:0 clicsRT2:B2:GoB3:a:b"), "column(text \"0 clics\", row(button \"Go\", button \"a:b\"))");
        assert_eq!(ui_describe("T5:été !"), "text \"été !\"");
        // The last written comes first; a group hands its style down; a part's own style wins.
        assert_eq!(ui_describe("Ss20:Ss10:Sb1:T1:a"), "text \"a\" size(20) bold");
        assert_eq!(ui_describe("Sc255:Ss9:CL2:Ss5:T1:aB1:b"), "column(text \"a\" color(#0000ff) size(5), button \"b\" size(9))");
        assert_eq!(ui_describe("Ss9:RT1:Ss64:P5:a.png"), "row(image \"a.png\" size(64))");
        assert_eq!(ui_describe("Ss9:RT1:P5:a.png"), "row(image \"a.png\")");
        assert_eq!(ui_describe("Sm4:Sb1:T1:a"), "text \"a\" bold margin(4)");
    }
}

/// `files.matching(pattern)`: the paths that `pattern` describes, sorted, with `/` between the
/// names. In each name `*` stands for any characters and `?` for one; a name `**` for any number
/// of folders (none included). A wildcard does not reach names starting with `.` unless the name
/// written starts with `.` too, as in a shell. `list` gives a folder's names, `kind` says of a
/// path whether it is there (`None`), a folder, and a link: `**` does not go through links, which
/// could make it turn round forever. The folder before the first wildcard must be there (failure
/// 1, `NotFound`); folders that cannot be read further down are left out.
pub fn matching(pattern: &str, list: &dyn Fn(&str) -> Option<Vec<String>>, kind: &dyn Fn(&str) -> Option<(bool, bool)>) -> Result<Vec<String>, i64> {
    let absolute = pattern.starts_with('/');
    let names: Vec<&str> = pattern.split(['/', '\\']).filter(|n| !n.is_empty()).collect();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let wild = |n: &str| n.contains(['*', '?']);
    let fixed = names.iter().take_while(|n| !wild(n)).count();
    let mut base = String::from(if absolute { "/" } else { "" });
    let start = fixed.min(names.len() - 1);
    for n in &names[..start] {
        base = joined(&base, n);
    }
    if fixed < names.len() && !base.is_empty() && kind(&base).is_none_or(|(folder, _)| !folder) {
        return Err(1);
    }
    let rest = &names[start..];
    let mut found = Vec::new();
    walk(&base, rest, list, kind, &mut found);
    found.sort();
    found.dedup();
    Ok(found)
}

fn joined(folder: &str, name: &str) -> String {
    match folder {
        "" => name.into(),
        f if f.ends_with('/') => format!("{f}{name}"),
        f => format!("{f}/{name}"),
    }
}

fn walk(folder: &str, names: &[&str], list: &dyn Fn(&str) -> Option<Vec<String>>, kind: &dyn Fn(&str) -> Option<(bool, bool)>, found: &mut Vec<String>) {
    let Some((&name, rest)) = names.split_first() else {
        found.push(folder.into());
        return;
    };
    let inside = || list(if folder.is_empty() { "." } else { folder }).unwrap_or_default();
    // A path that is there: the end, or a folder to go on in.
    let reach = |path: String, found: &mut Vec<String>| match kind(&path) {
        Some(_) if rest.is_empty() => found.push(path),
        Some((true, _)) => walk(&path, rest, list, kind, found),
        _ => {}
    };
    if name == "**" {
        walk(folder, rest, list, kind, found);
        for n in inside().iter().filter(|n| !n.starts_with('.')) {
            let path = joined(folder, n);
            if kind(&path) == Some((true, false)) {
                walk(&path, names, list, kind, found);
            }
        }
    } else if !name.contains(['*', '?']) {
        reach(joined(folder, name), found);
    } else {
        for n in inside().iter().filter(|n| fits(name, n) && (!n.starts_with('.') || name.starts_with('.'))) {
            reach(joined(folder, n), found);
        }
    }
}

/// `name` fits `pattern`, whose `*` stands for any characters and `?` for one.
fn fits(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

/// `system.run`'s program: as written if it names a folder (`./tool`, `C:\tools\tool`), else the
/// first one of that name in the folders of `path` (`$PATH`). On Windows a name without an
/// extension gets `.exe`, and a `.bat` or `.cmd` is refused (failure 2, `Denied`): Windows would
/// run it through `cmd.exe`, which reads its arguments its own way. Failure 1 if it is nowhere.
pub fn find_program(name: &str, path: Option<&str>, windows: bool, is_file: &dyn Fn(&str) -> bool) -> Result<String, i64> {
    let slash: &[char] = if windows { &['/', '\\'] } else { &['/'] };
    let last = name.rsplit(slash).next().unwrap_or(name);
    let name = if windows && !last.contains('.') { format!("{name}.exe") } else { name.into() };
    let lower = name.to_ascii_lowercase();
    if windows && (lower.ends_with(".bat") || lower.ends_with(".cmd")) {
        return Err(2);
    }
    if name.contains(slash) {
        return if is_file(&name) { Ok(name) } else { Err(1) };
    }
    let folders = path.unwrap_or("").split(if windows { ';' } else { ':' }).filter(|f| !f.is_empty());
    folders.map(|f| format!("{}{}{name}", f.trim_end_matches(slash), if windows { '\\' } else { '/' })).find(|p| is_file(p)).ok_or(1)
}

/// The command line Windows gives a program (`CreateProcessW`), cut back into words by the
/// program the usual way: as Rust's own `std::process::Command` writes it, so that `kova run`
/// (which uses it) and a built program (runtime/windows.rs) give the same words.
#[allow(dead_code)] // by runtime/windows.rs only: `kova run` lets Rust write it
pub fn command_line(program: &str, args: &[&str]) -> String {
    let mut line = format!("\"{program}\"");
    for a in args {
        line.push(' ');
        let quote = a.is_empty() || a.contains([' ', '\t']);
        if quote {
            line.push('"');
        }
        let mut backslashes = 0;
        for c in a.chars() {
            if c == '\\' {
                backslashes += 1;
            } else {
                if c == '"' {
                    line.extend(core::iter::repeat_n('\\', backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(c);
        }
        if quote {
            line.extend(core::iter::repeat_n('\\', backslashes));
            line.push('"');
        }
    }
    line
}

/// A program's exit status as `system.run` gives it: its code, or 128 + the signal that
/// stopped it (as shells say it). `status` is what `wait4` / `waitpid` wrote.
#[cfg_attr(windows, allow(dead_code))] // Windows gives the code itself
pub fn exit_code(status: i32) -> i64 {
    match status & 0x7f {
        0 => ((status >> 8) & 0xff) as i64,
        signal => 128 + signal as i64,
    }
}

/// `s.padded(width)`: spaces after `s` (before it if `left`) up to `width` characters.
pub fn padded(s: &str, width: i64, left: bool) -> String {
    let spaces = " ".repeat((width.max(0) as usize).saturating_sub(s.chars().count()));
    if left { spaces + s } else { String::from(s) + &spaces }
}
