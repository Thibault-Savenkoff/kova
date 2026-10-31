// DEFLATE (RFC 1951), for `data.deflate` and `data.inflate`, and CRC-32. Shared by the interpreter
// and the runtime of built programs.

use alloc::vec::Vec;

/// Reads bits from the lowest of each byte up, as DEFLATE writes them.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    held: u32,
    count: u32,
}

impl Bits<'_> {
    fn take(&mut self, n: u32) -> Option<u32> {
        while self.count < n {
            self.held |= (*self.data.get(self.at)? as u32) << self.count;
            self.at += 1;
            self.count += 8;
        }
        let v = self.held & ((1u64 << n) - 1) as u32;
        self.held >>= n;
        self.count -= n;
        Some(v)
    }
}

/// A Huffman code: how many codes of each length, and the symbols in the order of their codes.
struct Code {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Code {
    fn new(lengths: &[u8]) -> Code {
        let mut counts = [0u16; 16];
        lengths.iter().for_each(|&l| counts[l as usize] += 1);
        counts[0] = 0;
        let mut offsets = [0u16; 16];
        for l in 1..15 {
            offsets[l + 1] = offsets[l] + counts[l];
        }
        let mut symbols = alloc::vec![0; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = s as u16;
                offsets[l as usize] += 1;
            }
        }
        Code { counts, symbols }
    }

    fn read(&self, bits: &mut Bits) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for l in 1..16 {
            code |= bits.take(1)? as i32;
            let count = self.counts[l] as i32;
            if code - count < first {
                return self.symbols.get((index + code - first) as usize).copied();
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        None
    }
}

const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DISTANCE_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DISTANCE_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn fixed_lengths() -> [u8; 288] {
    let mut l = [8u8; 288];
    l[144..256].fill(9);
    l[256..280].fill(7);
    l
}

/// What `data` unpacks to; `None` if it is not DEFLATE data. `limit`: the most it may give, so that
/// a tiny file cannot fill the memory.
pub fn inflate(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut bits = Bits { data, at: 0, held: 0, count: 0 };
    let mut out = Vec::new();
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => {
                bits.held = 0;
                bits.count = 0;
                let head = data.get(bits.at..bits.at + 4)?;
                let size = u16::from_le_bytes([head[0], head[1]]);
                if size != !u16::from_le_bytes([head[2], head[3]]) {
                    return None;
                }
                bits.at += 4;
                out.extend_from_slice(data.get(bits.at..bits.at + size as usize)?);
                bits.at += size as usize;
            }
            1 => blocks(&mut bits, &Code::new(&fixed_lengths()), &Code::new(&[5; 30]), &mut out, limit)?,
            2 => {
                let (lit, dist) = dynamic(&mut bits)?;
                blocks(&mut bits, &lit, &dist, &mut out, limit)?;
            }
            _ => return None,
        }
        if out.len() > limit {
            return None;
        }
        if last {
            return Some(out);
        }
    }
}

fn dynamic(bits: &mut Bits) -> Option<(Code, Code)> {
    let lit = bits.take(5)? as usize + 257;
    let dist = bits.take(5)? as usize + 1;
    let n = bits.take(4)? as usize + 4;
    let mut first = [0u8; 19];
    for &o in &ORDER_OF_LENGTHS[..n] {
        first[o] = bits.take(3)? as u8;
    }
    let code = Code::new(&first);
    let mut lengths = Vec::with_capacity(lit + dist);
    while lengths.len() < lit + dist {
        match code.read(bits)? {
            s @ 0..=15 => lengths.push(s as u8),
            16 => {
                let repeat = bits.take(2)? + 3;
                let last = *lengths.last()?;
                (0..repeat).for_each(|_| lengths.push(last));
            }
            17 => {
                let repeat = bits.take(3)? + 3;
                (0..repeat).for_each(|_| lengths.push(0));
            }
            _ => {
                let repeat = bits.take(7)? + 11;
                (0..repeat).for_each(|_| lengths.push(0));
            }
        }
    }
    if lengths.len() != lit + dist {
        return None;
    }
    Some((Code::new(&lengths[..lit]), Code::new(&lengths[lit..])))
}

fn blocks(bits: &mut Bits, lit: &Code, dist: &Code, out: &mut Vec<u8>, limit: usize) -> Option<()> {
    loop {
        match lit.read(bits)? {
            s @ 0..=255 => out.push(s as u8),
            256 => return Some(()),
            s => {
                let k = s as usize - 257;
                let length = *LENGTH_BASE.get(k)? as usize + bits.take(LENGTH_EXTRA[k] as u32)? as usize;
                let d = dist.read(bits)? as usize;
                let back = *DISTANCE_BASE.get(d)? as usize + bits.take(DISTANCE_EXTRA[d] as u32)? as usize;
                if back > out.len() || out.len() + length > limit {
                    return None;
                }
                let from = out.len() - back;
                (0..length).for_each(|i| out.push(out[from + i]));
            }
        }
    }
}

/// Writes bits from the lowest up; Huffman codes go from their highest bit.
struct Writer {
    out: Vec<u8>,
    held: u64,
    count: u32,
}

impl Writer {
    fn put(&mut self, v: u32, n: u32) {
        self.held |= (v as u64) << self.count;
        self.count += n;
        while self.count >= 8 {
            self.out.push(self.held as u8);
            self.held >>= 8;
            self.count -= 8;
        }
    }
    fn code(&mut self, v: u32, n: u32) {
        self.put(v.reverse_bits() >> (32 - n), n);
    }
    /// What is held, up to a byte's end.
    fn align(&mut self) {
        if self.count > 0 {
            self.put(0, 8 - self.count);
        }
    }
}

/// How long each symbol's code is, none past `limit`. Huffman: the two rarest groups join, and each
/// of their symbols gets one bit longer. Too long: the counts are flattened and it starts again.
fn lengths(counts: &[u32], limit: u8) -> Vec<u8> {
    let mut counts = counts.to_vec();
    loop {
        let mut made = alloc::vec![0u8; counts.len()];
        let mut groups: Vec<(u64, Vec<usize>)> = counts.iter().enumerate().filter(|(_, n)| **n > 0).map(|(s, n)| (*n as u64, alloc::vec![s])).collect();
        if let [(_, only)] = groups.as_slice() {
            made[only[0]] = 1;
            return made;
        }
        while groups.len() > 1 {
            groups.sort_by_key(|g| core::cmp::Reverse(g.0));
            let (a, b) = (groups.pop().unwrap(), groups.pop().unwrap());
            let mut joined = a.1;
            joined.extend(b.1);
            joined.iter().for_each(|&s| made[s] += 1);
            groups.push((a.0 + b.0, joined));
        }
        if made.iter().all(|&l| l <= limit) {
            return made;
        }
        counts.iter_mut().filter(|n| **n > 0).for_each(|n| *n = *n / 2 + 1);
    }
}

/// The codes of those lengths, in DEFLATE's order (RFC 1951, 3.2.2).
fn codes(lengths: &[u8]) -> Vec<u32> {
    let mut count = [0u32; 16];
    lengths.iter().for_each(|&l| count[l as usize] += 1);
    count[0] = 0;
    let mut next = [0u32; 16];
    for l in 1..16 {
        next[l] = (next[l - 1] + count[l - 1]) << 1;
    }
    lengths.iter().map(|&l| {
        let code = next[l as usize];
        next[l as usize] += 1;
        code
    }).collect()
}

/// The lengths of both codes, written as DEFLATE wants: what repeats is said once. A symbol, and
/// the number after it with its size in bits.
fn runs(lengths: &[u8]) -> Vec<(u8, u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let l = lengths[i];
        let n = lengths[i..].iter().take_while(|&&x| x == l).count();
        let taken = match (l, n) {
            (0, 11..) => {
                out.push((18, n.min(138) as u32 - 11, 7));
                n.min(138)
            }
            (0, 3..) => {
                out.push((17, n as u32 - 3, 3));
                n
            }
            (_, 4..) => {
                out.push((l, 0, 0));
                out.push((16, n.min(7) as u32 - 4, 2));
                n.min(7)
            }
            _ => {
                out.push((l, 0, 0));
                1
            }
        };
        i += taken;
    }
    out
}

const ORDER_OF_LENGTHS: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// A literal, or a repeat of `length` bytes found `back` bytes behind.
#[derive(Clone, Copy)]
struct Token {
    value: u16,
    back: u16,
}

fn length_code(length: usize) -> usize {
    LENGTH_BASE.iter().rposition(|&b| b as usize <= length).unwrap()
}

fn distance_code(back: usize) -> usize {
    DISTANCE_BASE.iter().rposition(|&b| b as usize <= back).unwrap()
}

/// One block: stored, or with the fixed code, or with one made for it, whichever is the smallest.
fn block(w: &mut Writer, tokens: &[Token], plain: &[u8], last: bool) {
    let (mut lit, mut dist, mut extra) = ([0u32; 288], [0u32; 30], 0usize);
    lit[256] = 1;
    for t in tokens {
        if t.back == 0 {
            lit[t.value as usize] += 1;
        } else {
            let (k, d) = (length_code(t.value as usize), distance_code(t.back as usize));
            lit[257 + k] += 1;
            dist[d] += 1;
            extra += LENGTH_EXTRA[k] as usize + DISTANCE_EXTRA[d] as usize;
        }
    }
    let (lit_lengths, mut dist_lengths) = (lengths(&lit, 15), lengths(&dist, 15));
    if dist.iter().all(|&n| n == 0) {
        dist_lengths[0] = 1;
    }
    let lit_used = (257..=286).rev().find(|&s| lit_lengths[s - 1] != 0).unwrap_or(257);
    let dist_used = (1..=30).rev().find(|&s| dist_lengths[s - 1] != 0).unwrap_or(1);
    let written = runs(&[&lit_lengths[..lit_used], &dist_lengths[..dist_used]].concat());
    let mut small = [0u32; 19];
    written.iter().for_each(|r| small[r.0 as usize] += 1);
    let small_lengths = lengths(&small, 7);
    let small_used = (4..=19).rev().find(|&n| small_lengths[ORDER_OF_LENGTHS[n - 1]] != 0).unwrap_or(4);
    let bits = |lit_l: &[u8], dist_l: &[u8]| -> usize {
        lit.iter().zip(lit_l).map(|(&n, &l)| n as usize * l as usize).sum::<usize>() + dist.iter().zip(dist_l).map(|(&n, &l)| n as usize * l as usize).sum::<usize>() + extra
    };
    let fixed = (fixed_lengths(), [5u8; 30]);
    let fixed_bits = bits(&fixed.0, &fixed.1);
    let header = 14 + 3 * small_used + written.iter().map(|r| small_lengths[r.0 as usize] as usize + r.2 as usize).sum::<usize>();
    let made_bits = bits(&lit_lengths, &dist_lengths) + header;
    w.put(last as u32, 1);
    if plain.len() <= 65535 && (plain.len() + 4) * 8 < fixed_bits.min(made_bits) {
        w.put(0, 2);
        w.align();
        w.out.extend_from_slice(&(plain.len() as u16).to_le_bytes());
        w.out.extend_from_slice(&(!(plain.len() as u16)).to_le_bytes());
        w.out.extend_from_slice(plain);
        return;
    }
    let (lit_lengths, dist_lengths): (&[u8], &[u8]) = if fixed_bits <= made_bits {
        w.put(1, 2);
        (&fixed.0, &fixed.1)
    } else {
        w.put(2, 2);
        w.put(lit_used as u32 - 257, 5);
        w.put(dist_used as u32 - 1, 5);
        w.put(small_used as u32 - 4, 4);
        ORDER_OF_LENGTHS[..small_used].iter().for_each(|&o| w.put(small_lengths[o] as u32, 3));
        let small_codes = codes(&small_lengths);
        for &(s, value, size) in &written {
            w.code(small_codes[s as usize], small_lengths[s as usize] as u32);
            w.put(value, size);
        }
        (&lit_lengths, &dist_lengths)
    };
    let (lit_codes, dist_codes) = (codes(lit_lengths), codes(dist_lengths));
    for t in tokens {
        if t.back == 0 {
            w.code(lit_codes[t.value as usize], lit_lengths[t.value as usize] as u32);
        } else {
            let (length, back) = (t.value as usize, t.back as usize);
            let (k, d) = (length_code(length), distance_code(back));
            w.code(lit_codes[257 + k], lit_lengths[257 + k] as u32);
            w.put((length - LENGTH_BASE[k] as usize) as u32, LENGTH_EXTRA[k] as u32);
            w.code(dist_codes[d], dist_lengths[d] as u32);
            w.put((back - DISTANCE_BASE[d] as usize) as u32, DISTANCE_EXTRA[d] as u32);
        }
    }
    w.code(lit_codes[256], lit_lengths[256] as u32);
}

/// The repeats found by the chain of positions sharing a hash of 3 bytes; a repeat waits if the next
/// byte starts a longer one, as gzip does.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    const WINDOW: usize = 32768;
    const TRIES: usize = 512;
    const BLOCK: usize = 1 << 16;
    let mut w = Writer { out: Vec::with_capacity(data.len() / 2 + 16), held: 0, count: 0 };
    let hash = |i: usize| ((data[i] as usize) << 8 ^ (data[i + 1] as usize) << 4 ^ data[i + 2] as usize ^ (data[i + 2] as usize) << 11) & 0xffff;
    let mut head = alloc::vec![usize::MAX; 1 << 16];
    let mut previous = alloc::vec![usize::MAX; WINDOW];
    let longest = |i: usize, head: &[usize], previous: &[usize]| {
        let (mut best, mut back) = (0, 0);
        if i + 2 >= data.len() {
            return (0, 0);
        }
        let most = (data.len() - i).min(258);
        let mut at = head[hash(i)];
        for _ in 0..TRIES {
            if at == usize::MAX || i - at > WINDOW - 1 {
                break;
            }
            if best == 0 || data[at + best] == data[i + best] {
                let same = data[at..].iter().zip(&data[i..]).take(most).take_while(|(a, b)| a == b).count();
                if same > best {
                    (best, back) = (same, i - at);
                    if same == most {
                        break;
                    }
                }
            }
            at = previous[at % WINDOW];
        }
        // Three bytes far away cost more than they save.
        if best < 3 || (best == 3 && back > 4096) { (0, 0) } else { (best, back) }
    };
    let mut tokens: Vec<Token> = Vec::with_capacity(BLOCK);
    let (mut i, mut start) = (0, 0);
    while i < data.len() {
        let (length, back) = longest(i, &head, &previous);
        let insert = |k: usize, head: &mut [usize], previous: &mut [usize]| {
            if k + 2 < data.len() {
                let h = hash(k);
                previous[k % WINDOW] = head[h];
                head[h] = k;
            }
        };
        insert(i, &mut head, &mut previous);
        let later = if length > 0 && length < 258 { longest(i + 1, &head, &previous).0 } else { 0 };
        if length == 0 || later > length {
            tokens.push(Token { value: data[i] as u16, back: 0 });
            i += 1;
        } else {
            tokens.push(Token { value: length as u16, back: back as u16 });
            (i + 1..i + length).for_each(|k| insert(k, &mut head, &mut previous));
            i += length;
        }
        if tokens.len() == BLOCK {
            block(&mut w, &tokens, &data[start..i], i == data.len());
            tokens.clear();
            start = i;
        }
    }
    if !tokens.is_empty() || data.is_empty() {
        block(&mut w, &tokens, &data[start..], true);
    }
    w.align();
    w.out
}

pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |mut c, &b| {
        c ^= b as u32;
        (0..8).for_each(|_| c = if c & 1 != 0 { c >> 1 ^ 0xedb8_8320 } else { c >> 1 });
        c
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_and_unpacks() {
        let mut noise = Vec::new();
        let mut x = 1u32;
        for _ in 0..300_000 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            noise.push((x >> 24) as u8);
        }
        let text = "the quick brown fox ".repeat(20_000).into_bytes();
        for data in [&b""[..], b"a", b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", b"abc", &noise, &text] {
            let packed = deflate(data);
            assert_eq!(inflate(&packed, data.len() + 1).unwrap(), data);
        }
        assert!(deflate(&noise).len() < noise.len() + noise.len() / 1000 + 16);
        assert!(deflate(&text).len() < 2000);
    }
}
