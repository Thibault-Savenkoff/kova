// Code used both by `kova run` (src/interp.rs) and by the runtime of `kova build` (runtime/),
// so that the two print and compute the same thing from one copy. Only `core` and `alloc`
// here: the runtime has no `std`.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

/// The capital (`upper`) or small letter of `c`, for the Latin, Greek and Cyrillic alphabets;
/// any other character stays. Rust's own `to_uppercase` knows all of Unicode, but its tables
/// are in `std`, which the runtime does not have.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
