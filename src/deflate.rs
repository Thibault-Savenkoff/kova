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
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let lit = bits.take(5)? as usize + 257;
    let dist = bits.take(5)? as usize + 1;
    let n = bits.take(4)? as usize + 4;
    let mut first = [0u8; 19];
    for &o in &ORDER[..n] {
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
    fn symbol(&mut self, s: u32) {
        match s {
            0..=143 => self.code(0x30 + s, 8),
            144..=255 => self.code(0x190 + s - 144, 9),
            256..=279 => self.code(s - 256, 7),
            _ => self.code(0xc0 + s - 280, 8),
        }
    }
}

/// `data` packed: repeats found through a hash of 3 bytes, in one block.
// ponytail: the fixed Huffman code and the first repeat of the last few; 10 to 20% larger than gzip -9.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    const WINDOW: usize = 32768;
    const TRIES: usize = 16;
    let mut w = Writer { out: Vec::with_capacity(data.len() / 2 + 16), held: 0, count: 0 };
    w.put(1, 1);
    w.put(1, 2);
    let hash = |i: usize| ((data[i] as usize) << 8 ^ (data[i + 1] as usize) << 4 ^ data[i + 2] as usize ^ (data[i + 2] as usize) << 11) & 0xffff;
    let mut head = alloc::vec![usize::MAX; 1 << 16];
    let mut previous = alloc::vec![usize::MAX; WINDOW];
    let mut i = 0;
    while i < data.len() {
        let (mut best, mut back) = (0, 0);
        if i + 2 < data.len() {
            let mut at = head[hash(i)];
            for _ in 0..TRIES {
                if at == usize::MAX || i - at > WINDOW - 1 {
                    break;
                }
                let same = data[at..].iter().zip(&data[i..]).take(258).take_while(|(a, b)| a == b).count();
                if same > best {
                    (best, back) = (same, i - at);
                }
                at = previous[at % WINDOW];
            }
        }
        let length = if best >= 3 { best } else { 1 };
        if best >= 3 {
            let k = LENGTH_BASE.iter().rposition(|&b| b as usize <= length).unwrap();
            w.symbol(257 + k as u32);
            w.put((length - LENGTH_BASE[k] as usize) as u32, LENGTH_EXTRA[k] as u32);
            let d = DISTANCE_BASE.iter().rposition(|&b| b as usize <= back).unwrap();
            w.code(d as u32, 5);
            w.put((back - DISTANCE_BASE[d] as usize) as u32, DISTANCE_EXTRA[d] as u32);
        } else {
            w.symbol(data[i] as u32);
        }
        for k in i..i + length {
            if k + 2 < data.len() {
                let h = hash(k);
                previous[k % WINDOW] = head[h];
                head[h] = k;
            }
        }
        i += length;
    }
    w.symbol(256);
    w.put(0, 7);
    w.out
}

pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |mut c, &b| {
        c ^= b as u32;
        (0..8).for_each(|_| c = if c & 1 != 0 { c >> 1 ^ 0xedb8_8320 } else { c >> 1 });
        c
    })
}
