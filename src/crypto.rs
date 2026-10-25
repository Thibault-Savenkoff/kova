// Fingerprints, signatures, base64, passwords and encryption (PLAN Part 4 "The library after
// Python, step 4"), shared by `kova run` (src/interp.rs), the runtime of `kova build`
// (runtime/kova.rs) and the Mac signatures of src/link.rs. Each one written from its standard and
// checked against the standard's own examples (tests below). No table looked up by a secret and
// no branch on one: the time taken tells nothing. Only `core` and `alloc`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// ---- SHA-256 (FIPS 180-4) and HMAC (RFC 2104) ----

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_block(h: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for i in 0..64 {
        w[i] = if i < 16 {
            u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]])
        } else {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1)
        };
    }
    let mut v = *h;
    for i in 0..64 {
        let [a, b, c, d, e, f, g, hh] = v;
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let t1 = hh.wrapping_add(s1).wrapping_add((e & f) ^ (!e & g)).wrapping_add(K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
        v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
    }
    for i in 0..8 {
        h[i] = h[i].wrapping_add(v[i]);
    }
}

/// SHA-256 of the pieces one after the other (HMAC hashes a key then a message without
/// copying them together).
pub fn sha256_of(pieces: &[&[u8]]) -> [u8; 32] {
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let (mut buffer, mut kept, mut length) = ([0u8; 64], 0, 0u64);
    for piece in pieces {
        length += piece.len() as u64;
        let mut rest = *piece;
        while !rest.is_empty() {
            if kept == 0 && rest.len() >= 64 {
                sha256_block(&mut h, &rest[..64]);
                rest = &rest[64..];
                continue;
            }
            let n = (64 - kept).min(rest.len());
            buffer[kept..kept + n].copy_from_slice(&rest[..n]);
            (kept, rest) = (kept + n, &rest[n..]);
            if kept == 64 {
                sha256_block(&mut h, &buffer);
                kept = 0;
            }
        }
    }
    buffer[kept] = 0x80;
    buffer[kept + 1..].fill(0);
    if kept >= 56 {
        sha256_block(&mut h, &buffer);
        buffer.fill(0);
    }
    buffer[56..].copy_from_slice(&(length * 8).to_be_bytes());
    sha256_block(&mut h, &buffer);
    let mut out = [0; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    sha256_of(&[data])
}

/// HMAC-SHA256 of `message` with `key`.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (inner, outer) = (k.map(|b| b ^ 0x36), k.map(|b| b ^ 0x5c));
    let first = sha256_of(&[&inner, message]);
    sha256_of(&[&outer, &first])
}

// ---- Base64 (RFC 4648) ----

const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 of `bytes`: with `+`, `/` and `=` at the end, or for addresses (`url`) with `-`, `_` and
/// no `=`.
pub fn base64(bytes: &[u8], url: bool) -> String {
    let letter = |n: u32| {
        let c = LETTERS[(n & 63) as usize];
        (match (url, c) {
            (true, b'+') => b'-',
            (true, b'/') => b'_',
            _ => c,
        }) as char
    };
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for k in 0..=chunk.len() {
            out.push(letter(n >> (18 - 6 * k)));
        }
        if !url {
            (chunk.len()..3).for_each(|_| out.push('='));
        }
    }
    out
}

/// The bytes written in base64 (either alphabet's letters, the other one's refused), or `None`.
/// Spaces and line ends are skipped (texts cut into lines); the `=` at the end may be missing.
pub fn from_base64(text: &str, url: bool) -> Option<Vec<u8>> {
    let mut out = vec![];
    let (mut n, mut bits, mut ended) = (0u32, 0, false);
    for c in text.bytes() {
        let value = match c {
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            b'=' => {
                ended = true;
                continue;
            }
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' if !url => 62,
            b'/' if !url => 63,
            b'-' if url => 62,
            b'_' if url => 63,
            _ => return None,
        };
        if ended {
            return None;
        }
        n = n << 6 | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((n >> bits) as u8);
            n &= (1 << bits) - 1;
        }
    }
    // 6 bits left over can't come from bytes; leftover bits must be 0.
    (bits != 6 && n == 0).then_some(out)
}

// ---- BLAKE2b (RFC 7693), for Argon2 ----

const IV: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
];

const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

fn blake2b_compress(h: &mut [u64; 8], block: &[u8; 128], length: u128, last: bool) {
    let mut m = [0u64; 16];
    for (i, w) in m.iter_mut().enumerate() {
        *w = u64::from_le_bytes(block[8 * i..8 * i + 8].try_into().unwrap());
    }
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(h);
    v[8..].copy_from_slice(&IV);
    v[12] ^= length as u64;
    v[13] ^= (length >> 64) as u64;
    if last {
        v[14] = !v[14];
    }
    let g = |v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64| {
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
        v[d] = (v[d] ^ v[a]).rotate_right(32);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(24);
        v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
        v[d] = (v[d] ^ v[a]).rotate_right(16);
        v[c] = v[c].wrapping_add(v[d]);
        v[b] = (v[b] ^ v[c]).rotate_right(63);
    };
    for i in 0..12 {
        let s = &SIGMA[i % 10];
        g(&mut v, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        g(&mut v, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        g(&mut v, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        g(&mut v, 3, 7, 11, 15, m[s[6]], m[s[7]]);
        g(&mut v, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        g(&mut v, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        g(&mut v, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        g(&mut v, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }
    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

/// BLAKE2b without a key, `size` bytes (1 to 64), of the pieces one after the other.
pub fn blake2b(size: usize, pieces: &[&[u8]]) -> Vec<u8> {
    let mut h = IV;
    h[0] ^= 0x0101_0000 ^ size as u64;
    let (mut block, mut kept, mut length) = ([0u8; 128], 0, 0u128);
    for piece in pieces {
        for &b in *piece {
            // A full block is compressed only once more bytes come: the last one is marked.
            if kept == 128 {
                length += 128;
                blake2b_compress(&mut h, &block, length, false);
                kept = 0;
            }
            block[kept] = b;
            kept += 1;
        }
    }
    block[kept..].fill(0);
    blake2b_compress(&mut h, &block, length + kept as u128, true);
    h.iter().flat_map(|w| w.to_le_bytes()).take(size).collect()
}

// ---- Argon2id (RFC 9106) ----

/// H' (RFC 9106 3.3): a hash of any size, made of BLAKE2b pieces.
fn long_hash(size: usize, pieces: &[&[u8]]) -> Vec<u8> {
    let n = (size as u32).to_le_bytes();
    let mut all: Vec<&[u8]> = vec![&n];
    all.extend_from_slice(pieces);
    if size <= 64 {
        return blake2b(size, &all);
    }
    let mut out = vec![];
    let mut v = blake2b(64, &all);
    while size - out.len() > 64 {
        out.extend_from_slice(&v[..32]);
        v = blake2b(if size - out.len() > 64 { 64 } else { size - out.len() }, &[&v]);
    }
    out.extend_from_slice(&v);
    out
}

type Block = [u64; 128];

/// GB of Argon2: BLAKE2b's mixing with a multiplication of the low halves.
fn gb(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize) {
    let mul = |x: u64, y: u64| 2u64.wrapping_mul(x & 0xffff_ffff).wrapping_mul(y & 0xffff_ffff);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(mul(v[a], v[b]));
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]).wrapping_add(mul(v[c], v[d]));
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(mul(v[a], v[b]));
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]).wrapping_add(mul(v[c], v[d]));
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

fn permute(v: &mut [u64; 16]) {
    gb(v, 0, 4, 8, 12);
    gb(v, 1, 5, 9, 13);
    gb(v, 2, 6, 10, 14);
    gb(v, 3, 7, 11, 15);
    gb(v, 0, 5, 10, 15);
    gb(v, 1, 6, 11, 12);
    gb(v, 2, 7, 8, 13);
    gb(v, 3, 4, 9, 14);
}

/// G (RFC 9106 3.5): `x` and `y` mixed into a new block.
fn compress(x: &Block, y: &Block) -> Block {
    let mut r = [0u64; 128];
    for i in 0..128 {
        r[i] = x[i] ^ y[i];
    }
    let mut q = r;
    // Rows: 16 words each; then columns: words 2i, 2i + 1 of each row.
    for row in 0..8 {
        let mut v: [u64; 16] = q[16 * row..16 * row + 16].try_into().unwrap();
        permute(&mut v);
        q[16 * row..16 * row + 16].copy_from_slice(&v);
    }
    for col in 0..8 {
        let mut v = [0u64; 16];
        for j in 0..8 {
            v[2 * j] = q[2 * col + 16 * j];
            v[2 * j + 1] = q[2 * col + 16 * j + 1];
        }
        permute(&mut v);
        for j in 0..8 {
            q[2 * col + 16 * j] = v[2 * j];
            q[2 * col + 16 * j + 1] = v[2 * j + 1];
        }
    }
    for i in 0..128 {
        q[i] ^= r[i];
    }
    q
}

fn to_block(bytes: &[u8]) -> Block {
    let mut b = [0u64; 128];
    for (i, w) in b.iter_mut().enumerate() {
        *w = u64::from_le_bytes(bytes[8 * i..8 * i + 8].try_into().unwrap());
    }
    b
}

/// Argon2id (RFC 9106) of `password` with `salt`, `memory` KiB (at least 8 per lane), `passes`,
/// `lanes` (computed one after the other), giving `size` bytes. `secret` and `data` are the
/// standard's optional K and X.
pub fn argon2id(password: &[u8], salt: &[u8], secret: &[u8], data: &[u8], memory: u32, passes: u32, lanes: u32, size: usize) -> Vec<u8> {
    let le = |n: usize| (n as u32).to_le_bytes();
    let h0 = blake2b(
        64,
        &[&lanes.to_le_bytes(), &le(size), &memory.to_le_bytes(), &passes.to_le_bytes(), &0x13u32.to_le_bytes(), &2u32.to_le_bytes(), &le(password.len()), password, &le(salt.len()), salt, &le(secret.len()), secret, &le(data.len()), data],
    );
    let (p, segment) = (lanes as usize, (memory / (4 * lanes)) as usize);
    let q = 4 * segment;
    let mut b: Vec<Block> = vec![[0; 128]; p * q];
    for l in 0..p {
        for j in 0..2 {
            b[l * q + j] = to_block(&long_hash(1024, &[&h0, &(j as u32).to_le_bytes(), &(l as u32).to_le_bytes()]));
        }
    }
    let zero = [0u64; 128];
    for pass in 0..passes as usize {
        for slice in 0..4 {
            for l in 0..p {
                // Argon2i's way (addresses from a counter) for the first half of the first pass,
                // Argon2d's (from the previous block) after.
                let independent = pass == 0 && slice < 2;
                let mut input = [0u64; 128];
                input[..6].copy_from_slice(&[pass as u64, l as u64, slice as u64, (p * q) as u64, passes as u64, 2]);
                let mut addresses = [0u64; 128];
                let next_addresses = |input: &mut Block, addresses: &mut Block| {
                    input[6] += 1;
                    *addresses = compress(&zero, &compress(&zero, input));
                };
                let start = if pass == 0 && slice == 0 { 2 } else { 0 };
                if independent && start == 2 {
                    next_addresses(&mut input, &mut addresses);
                }
                for i in start..segment {
                    let j = slice * segment + i;
                    let prev = if j == 0 { q - 1 } else { j - 1 };
                    let random = if independent {
                        if i % 128 == 0 {
                            next_addresses(&mut input, &mut addresses);
                        }
                        addresses[i % 128]
                    } else {
                        b[l * q + prev][0]
                    };
                    let (j1, j2) = (random & 0xffff_ffff, random >> 32);
                    let lane = if pass == 0 && slice == 0 { l } else { (j2 % p as u64) as usize };
                    let same = lane == l;
                    let area = match (pass, same) {
                        (0, true) => slice * segment + i - 1,
                        (0, false) => slice * segment - (i == 0) as usize,
                        (_, true) => q - segment + i - 1,
                        (_, false) => q - segment - (i == 0) as usize,
                    } as u64;
                    let x = j1 * j1 >> 32;
                    let y = area * x >> 32;
                    let start = if pass == 0 || slice == 3 { 0 } else { (slice + 1) * segment };
                    let reference = (start + (area - 1 - y) as usize) % q;
                    let new = compress(&b[l * q + prev], &b[lane * q + reference]);
                    let at = &mut b[l * q + j];
                    if pass == 0 {
                        *at = new;
                    } else {
                        for k in 0..128 {
                            at[k] ^= new[k];
                        }
                    }
                }
            }
        }
    }
    let mut last = b[q - 1];
    for l in 1..p {
        for k in 0..128 {
            last[k] ^= b[l * q + q - 1][k];
        }
    }
    let bytes: Vec<u8> = last.iter().flat_map(|w| w.to_le_bytes()).collect();
    long_hash(size, &[&bytes])
}

/// The settings of `secure.password_hash` (OWASP's first choice): 19 MiB, 2 passes, 1 lane.
const MEMORY: u32 = 19456;
const PASSES: u32 = 2;

/// `secure.password_hash`: the PHC text every Argon2 library reads,
/// `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>` (base64 without `=`), from 16 bytes of `salt`.
pub fn password_hash(password: &str, salt: &[u8; 16]) -> String {
    let hash = argon2id(password.as_bytes(), salt, &[], &[], MEMORY, PASSES, 1, 32);
    let plain = |b: &[u8]| base64(b, false).trim_end_matches('=').into();
    let parts: [String; 3] = [alloc::format!("m={MEMORY},t={PASSES},p=1"), plain(salt), plain(&hash)];
    alloc::format!("$argon2id$v=19${}${}${}", parts[0], parts[1], parts[2])
}

/// `secure.password_matches(password) hash(text)`: false for a text that is no Argon2id PHC
/// text, or whose settings ask too much (more than 1 GiB, 16 passes or 16 lanes: a text from
/// outside must not stall the program).
pub fn password_matches(password: &str, text: &str) -> bool {
    let mut parts = text.split('$');
    let (Some(""), Some("argon2id"), Some("v=19"), Some(settings), Some(salt), Some(hash), None) =
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let mut values = [0u32; 3];
    for (k, (name, part)) in ["m=", "t=", "p="].iter().zip(settings.split(',')).enumerate() {
        match part.strip_prefix(name).and_then(|n| n.parse().ok()) {
            Some(n) => values[k] = n,
            None => return false,
        }
    }
    let [m, t, p] = values;
    if settings.split(',').count() != 3 || !(1..=16).contains(&t) || !(1..=16).contains(&p) || m < 8 * p || m > 1 << 20 {
        return false;
    }
    let (Some(salt), Some(hash)) = (from_base64(salt, false), from_base64(hash, false)) else { return false };
    if salt.len() < 8 || !(4..=64).contains(&hash.len()) {
        return false;
    }
    same(&argon2id(password.as_bytes(), &salt, &[], &[], m, t, p, hash.len()), &hash)
}

/// The same bytes, in a time that does not tell where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

// ---- XChaCha20-Poly1305 (RFC 8439, draft-irtf-cfrg-xchacha) ----

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn words<const N: usize>(b: &[u8]) -> [u32; N] {
    core::array::from_fn(|i| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]))
}

/// The ChaCha state after its 20 rounds, from the key and the 4 last words.
fn rounds(key: &[u8; 32], tail: [u32; 4]) -> ([u32; 16], [u32; 16]) {
    let mut s = [0u32; 16];
    s[..4].copy_from_slice(&[0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]);
    s[4..12].copy_from_slice(&words::<8>(key));
    s[12..].copy_from_slice(&tail);
    let start = s;
    for _ in 0..10 {
        quarter(&mut s, 0, 4, 8, 12);
        quarter(&mut s, 1, 5, 9, 13);
        quarter(&mut s, 2, 6, 10, 14);
        quarter(&mut s, 3, 7, 11, 15);
        quarter(&mut s, 0, 5, 10, 15);
        quarter(&mut s, 1, 6, 11, 12);
        quarter(&mut s, 2, 7, 8, 13);
        quarter(&mut s, 3, 4, 9, 14);
    }
    (start, s)
}

/// ChaCha20's block `counter` with a 12-byte `nonce`.
fn chacha_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let n = words::<3>(nonce);
    let (start, s) = rounds(key, [counter, n[0], n[1], n[2]]);
    let mut out = [0; 64];
    for i in 0..16 {
        out[4 * i..4 * i + 4].copy_from_slice(&s[i].wrapping_add(start[i]).to_le_bytes());
    }
    out
}

/// HChaCha20: a new key from a key and 16 bytes of a nonce.
fn hchacha(key: &[u8; 32], nonce: &[u8]) -> [u8; 32] {
    let (_, s) = rounds(key, words::<4>(nonce));
    let mut out = [0; 32];
    for (i, w) in s[..4].iter().chain(&s[12..]).enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

/// `data` xored with ChaCha20's stream from block `counter` on.
fn chacha_xor(key: &[u8; 32], nonce: &[u8; 12], counter: u32, data: &mut [u8]) {
    for (k, chunk) in data.chunks_mut(64).enumerate() {
        let stream = chacha_block(key, counter + k as u32, nonce);
        chunk.iter_mut().zip(stream).for_each(|(b, s)| *b ^= s);
    }
}

/// Poly1305 (RFC 8439 2.5), with 26-bit pieces so that every product fits in 64 bits.
fn poly1305(key: &[u8; 32], message: &[u8]) -> [u8; 16] {
    let t: [u32; 4] = words(key);
    let r = [t[0] & 0x3ffffff, (t[0] >> 26 | t[1] << 6) & 0x3ffff03, (t[1] >> 20 | t[2] << 12) & 0x3ffc0ff, (t[2] >> 14 | t[3] << 18) & 0x3f03fff, (t[3] >> 8) & 0x00fffff].map(u64::from);
    let s = [r[1] * 5, r[2] * 5, r[3] * 5, r[4] * 5];
    let mut h = [0u64; 5];
    const MASK: u64 = 0x3ffffff;
    for chunk in message.chunks(16) {
        let mut m = [0u8; 17];
        m[..chunk.len()].copy_from_slice(chunk);
        m[chunk.len()] = 1;
        let w = |at: usize| u32::from_le_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]]) as u64;
        h[0] += w(0) & MASK;
        h[1] += (w(3) >> 2) & MASK;
        h[2] += (w(6) >> 4) & MASK;
        h[3] += (w(9) >> 6) & MASK;
        h[4] += (w(12) >> 8) | (m[16] as u64) << 24;
        let d = [
            h[0] * r[0] + h[1] * s[3] + h[2] * s[2] + h[3] * s[1] + h[4] * s[0],
            h[0] * r[1] + h[1] * r[0] + h[2] * s[3] + h[3] * s[2] + h[4] * s[1],
            h[0] * r[2] + h[1] * r[1] + h[2] * r[0] + h[3] * s[3] + h[4] * s[2],
            h[0] * r[3] + h[1] * r[2] + h[2] * r[1] + h[3] * r[0] + h[4] * s[3],
            h[0] * r[4] + h[1] * r[3] + h[2] * r[2] + h[3] * r[1] + h[4] * r[0],
        ];
        let mut c = 0;
        for i in 0..5 {
            let x = d[i] + c;
            h[i] = x & MASK;
            c = x >> 26;
        }
        h[0] += c * 5;
        h[1] += h[0] >> 26;
        h[0] &= MASK;
    }
    // Fully carried, then h - p kept if it is not below 0.
    let mut c = 0;
    for i in 1..5 {
        h[i] += c;
        c = h[i] >> 26;
        h[i] &= MASK;
    }
    h[0] += c * 5;
    c = h[0] >> 26;
    h[0] &= MASK;
    h[1] += c;
    let mut g = [0u64; 5];
    c = 5;
    for i in 0..5 {
        g[i] = h[i] + c;
        c = g[i] >> 26;
        g[i] &= MASK;
    }
    // g = h + 5 - 2^130: kept when it reached 2^130 (its carry `c` is 1).
    let keep = 0u64.wrapping_sub(c);
    for i in 0..5 {
        h[i] = (h[i] & !keep) | (g[i] & keep);
    }
    let lo = h[0] | h[1] << 26 | h[2] << 52;
    let hi = h[2] >> 12 | h[3] << 14 | h[4] << 40;
    let pad: [u32; 4] = words(&key[16..]);
    let (lo, carry) = lo.overflowing_add(pad[0] as u64 | (pad[1] as u64) << 32);
    let hi = hi.wrapping_add(pad[2] as u64 | (pad[3] as u64) << 32).wrapping_add(carry as u64);
    let mut out = [0; 16];
    out[..8].copy_from_slice(&lo.to_le_bytes());
    out[8..].copy_from_slice(&hi.to_le_bytes());
    out
}

/// The Poly1305 tag of the AEAD (RFC 8439 2.8) over `aad` and `cipher`.
fn tag(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], cipher: &[u8]) -> [u8; 16] {
    let one: [u8; 32] = chacha_block(key, 0, nonce)[..32].try_into().unwrap();
    let pad = |n: usize| vec![0u8; (16 - n % 16) % 16];
    let mut m = aad.to_vec();
    m.extend(pad(aad.len()));
    m.extend_from_slice(cipher);
    m.extend(pad(cipher.len()));
    m.extend((aad.len() as u64).to_le_bytes());
    m.extend((cipher.len() as u64).to_le_bytes());
    poly1305(&one, &m)
}

/// XChaCha20-Poly1305: the key and 12-byte nonce of the inner ChaCha20-Poly1305.
fn inner(key: &[u8; 32], nonce: &[u8; 24]) -> ([u8; 32], [u8; 12]) {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&nonce[16..]);
    (hchacha(key, &nonce[..16]), n)
}

/// `plain` encrypted, then its 16-byte tag.
pub fn seal(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let (k, n) = inner(key, nonce);
    let mut out = plain.to_vec();
    chacha_xor(&k, &n, 1, &mut out);
    let t = tag(&k, &n, aad, &out);
    out.extend_from_slice(&t);
    out
}

/// What `seal` made, back, or `None` if the key is not the one or a byte changed.
pub fn open(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let cut = sealed.len().checked_sub(16)?;
    let (k, n) = inner(key, nonce);
    if !same(&tag(&k, &n, aad, &sealed[..cut]), &sealed[cut..]) {
        return None;
    }
    let mut out = sealed[..cut].to_vec();
    chacha_xor(&k, &n, 1, &mut out);
    Some(out)
}

/// `secure.encrypt(text) key(k)`: base64 of the 24-byte `nonce` (drawn at random), the encrypted
/// text and its tag; `None` if `key` is not 32 bytes in base64 (`secure.key`).
pub fn encrypt(text: &str, key: &str, nonce: &[u8; 24]) -> Option<String> {
    let key: [u8; 32] = from_base64(key, false)?.try_into().ok()?;
    let mut all = nonce.to_vec();
    all.extend(seal(&key, nonce, &[], text.as_bytes()));
    Some(base64(&all, false))
}

/// `secure.decrypt(code) key(k)`: the text, or `None` (a wrong key, a changed byte, not a text).
pub fn decrypt(code: &str, key: &str) -> Option<String> {
    let key: [u8; 32] = from_base64(key, false)?.try_into().ok()?;
    let all = from_base64(code, false)?;
    let nonce: [u8; 24] = all.get(..24)?.try_into().ok()?;
    String::from_utf8(open(&key, &nonce, &[], &all[24..])?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| alloc::format!("{b:02x}")).collect()
    }

    fn bytes(h: &str) -> Vec<u8> {
        let h: String = h.split_whitespace().collect();
        (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn sha256_and_hmac() {
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(&[b'a'; 56])), "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
        assert_eq!(hex(&sha256(&[7; 1000])), "df1329c8b6c7cf3740bbe2f8bab34d253a8d9534a79dceea18177081fdf9f0e9");
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        // RFC 4231, cases 2 and 6.
        assert_eq!(hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn base64_both_ways() {
        // RFC 4648 section 10.
        for (plain, code) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(plain.as_bytes(), false), code);
            assert_eq!(from_base64(code, false).unwrap(), plain.as_bytes());
        }
        assert_eq!(base64(&[0xfb, 0xff], false), "+/8=");
        assert_eq!(base64(&[0xfb, 0xff], true), "-_8");
        assert_eq!(from_base64("-_8", true).unwrap(), [0xfb, 0xff]);
        assert_eq!(from_base64("Zm9v\nYmFy", false).unwrap(), b"foobar");
        assert_eq!(from_base64("Zm9vYg", false).unwrap(), b"foob");
        for wrong in ["Zm9v!", "-_8", "Z", "Zh==", "Zg==Zg"] {
            assert_eq!(from_base64(wrong, false), None, "{wrong}");
        }
    }

    #[test]
    fn blake2b_and_argon2id() {
        assert_eq!(hex(&blake2b(64, &[b"abc"])), "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923");
        assert_eq!(hex(&blake2b(32, &[&[1; 200], b""])), hex(&blake2b(32, &[&[1; 128], &[1; 72]])));
        // RFC 9106 5.3.
        let tag = argon2id(&[1; 32], &[2; 16], &[3; 8], &[4; 12], 32, 3, 4, 32);
        assert_eq!(hex(&tag), "0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659");
        let h = password_hash("correct horse", &[9; 16]);
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$CQkJCQkJCQkJCQkJCQkJCQ$"), "{h}");
        assert!(password_matches("correct horse", &h));
        assert!(!password_matches("correct hors", &h));
        assert!(!password_matches("correct horse", &h.replace("t=2", "t=200")));
        assert!(!password_matches("x", "not a hash"));
    }

    #[test]
    fn xchacha20_poly1305() {
        // RFC 8439 2.3.2 and 2.5.2.
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let block = chacha_block(&key, 1, &bytes("000000090000004a00000000").try_into().unwrap());
        assert_eq!(hex(&block[..16]), "10f1e7e4d13b5915500fdd1fa32071c4");
        let pk: [u8; 32] = bytes("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b").try_into().unwrap();
        assert_eq!(hex(&poly1305(&pk, b"Cryptographic Forum Research Group")), "a8061dc1305136c6c22b8baf0c0127a9");
        // draft-irtf-cfrg-xchacha 2.2.1 and A.3.1.
        assert_eq!(hex(&hchacha(&key, &bytes("000000090000004a0000000031415927"))), "82413b4227b27bfed30e42508a877d73a0f9e4d58a74a853c12ec41326d3ecdc");
        let plain = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let key: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
        let nonce: [u8; 24] = core::array::from_fn(|i| 0x40 + i as u8);
        let aad = bytes("50515253c0c1c2c3c4c5c6c7");
        let sealed = seal(&key, &nonce, &aad, plain);
        assert_eq!(hex(&sealed[..16]), "bd6d179d3e83d43b9576579493c0e939");
        assert_eq!(hex(&sealed[sealed.len() - 16..]), "c0875924c1c7987947deafd8780acf49");
        assert_eq!(open(&key, &nonce, &aad, &sealed).unwrap(), plain);
        let mut changed = sealed.clone();
        changed[3] ^= 1;
        assert_eq!(open(&key, &nonce, &aad, &changed), None);
        let k = base64(&[5; 32], false);
        let code = encrypt("secret", &k, &[6; 24]).unwrap();
        assert_eq!(decrypt(&code, &k).as_deref(), Some("secret"));
        assert_eq!(decrypt(&code, &base64(&[7; 32], false)), None);
        assert_eq!(encrypt("x", "short", &[0; 24]), None);
    }
}
