// `BigInt` under `kova run`: a value is its decimal text, like "-42", without leading zeros
// ("0" is never negative). The runtime of `kova build` uses this file too (runtime/), so only
// `core` and `alloc` here.
// ponytail: one decimal digit at a time, O(n²) for `*` and `/`; limbs of 9 digits when big
// numbers get thousands of digits long.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};
use core::cmp::Ordering;

/// The sign and the digits, the units first: "-120" → (true, [0, 2, 1]).
fn split(s: &str) -> (bool, Vec<u8>) {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    (neg, digits.bytes().rev().map(|b| b - b'0').collect())
}

fn join(neg: bool, mut d: Vec<u8>) -> String {
    while d.len() > 1 && d.last() == Some(&0) {
        d.pop();
    }
    let zero = d.iter().all(|&x| x == 0);
    let digits: String = d.iter().rev().map(|&x| (x + b'0') as char).collect();
    if neg && !zero { format!("-{digits}") } else if zero { "0".into() } else { digits }
}

fn compare(a: &[u8], b: &[u8]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn trimmed(mut d: Vec<u8>) -> Vec<u8> {
    while d.len() > 1 && d.last() == Some(&0) {
        d.pop();
    }
    d
}

fn plus(a: &[u8], b: &[u8]) -> Vec<u8> {
    let (mut out, mut carry) = (vec![], 0);
    for i in 0..a.len().max(b.len()) {
        let s = a.get(i).unwrap_or(&0) + b.get(i).unwrap_or(&0) + carry;
        out.push(s % 10);
        carry = s / 10;
    }
    if carry > 0 {
        out.push(carry);
    }
    out
}

/// `a - b`, with `a >= b`.
fn minus(a: &[u8], b: &[u8]) -> Vec<u8> {
    let (mut out, mut borrow) = (vec![], 0i8);
    for i in 0..a.len() {
        let mut d = a[i] as i8 - *b.get(i).unwrap_or(&0) as i8 - borrow;
        borrow = (d < 0) as i8;
        if d < 0 {
            d += 10;
        }
        out.push(d as u8);
    }
    trimmed(out)
}

fn times(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            out[i + j] += x as u32 * y as u32;
        }
    }
    let mut carry = 0;
    for d in out.iter_mut() {
        *d += carry;
        carry = *d / 10;
        *d %= 10;
    }
    trimmed(out.into_iter().map(|d| d as u8).collect())
}

/// Long division, as on paper: (quotient, remainder). `b` is not 0.
fn divide(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut q, mut r) = (vec![0; a.len()], vec![0]);
    for i in (0..a.len()).rev() {
        r.insert(0, a[i]);
        r = trimmed(r);
        while compare(&r, b) != Ordering::Less {
            r = minus(&r, b);
            q[i] += 1;
        }
    }
    (trimmed(q), r)
}

/// `a + b`.
pub fn add(a: &str, b: &str) -> String {
    let ((na, da), (nb, db)) = (split(a), split(b));
    if na == nb {
        return join(na, plus(&da, &db));
    }
    match compare(&da, &db) {
        Ordering::Less => join(nb, minus(&db, &da)),
        _ => join(na, minus(&da, &db)),
    }
}

/// `a - b`.
pub fn sub(a: &str, b: &str) -> String {
    let negated = match b.strip_prefix('-') {
        Some(rest) => rest.to_string(),
        None => format!("-{b}"),
    };
    add(a, &negated)
}

/// `a * b`.
pub fn mul(a: &str, b: &str) -> String {
    let ((na, da), (nb, db)) = (split(a), split(b));
    join(na != nb, times(&da, &db))
}

/// `a / b`, truncated like `Int`'s (`-7 / 2` = `-3`); `b` is not 0 (checked before).
pub fn div(a: &str, b: &str) -> String {
    let ((na, da), (nb, db)) = (split(a), split(b));
    join(na != nb, divide(&da, &db).0)
}

/// `a % b`, of the sign of `a` like `Int`'s (`-7 % 2` = `-1`); `b` is not 0.
pub fn rem(a: &str, b: &str) -> String {
    let ((na, da), (_, db)) = (split(a), split(b));
    join(na, divide(&da, &db).1)
}

/// `a < b`.
pub fn less(a: &str, b: &str) -> bool {
    let ((na, da), (nb, db)) = (split(a), split(b));
    match (na, nb) {
        (true, false) => true,
        (false, true) => false,
        (false, false) => compare(&da, &db) == Ordering::Less,
        (true, true) => compare(&db, &da) == Ordering::Less,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same answers as `i128` on small numbers, signs included.
    #[test]
    fn like_i128() {
        let values = [0i128, 1, -1, 7, -7, 2, -2, 10, -10, 99, 100, -1000, 123456789, -987654321];
        for &a in &values {
            for &b in &values {
                let (x, y) = (a.to_string(), b.to_string());
                assert_eq!(add(&x, &y), (a + b).to_string(), "{a} + {b}");
                assert_eq!(sub(&x, &y), (a - b).to_string(), "{a} - {b}");
                assert_eq!(mul(&x, &y), (a * b).to_string(), "{a} * {b}");
                assert_eq!(less(&x, &y), a < b, "{a} < {b}");
                if b != 0 {
                    assert_eq!(div(&x, &y), (a / b).to_string(), "{a} / {b}");
                    assert_eq!(rem(&x, &y), (a % b).to_string(), "{a} % {b}");
                }
            }
        }
        // 2^64 * 2^64 = 2^128 - 1 + 1
        let big = mul("18446744073709551616", "18446744073709551616");
        assert_eq!(big, "340282366920938463463374607431768211456");
        assert_eq!(div(&big, "18446744073709551616"), "18446744073709551616");
    }
}
