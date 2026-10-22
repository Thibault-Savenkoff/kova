// Kova's own linker (PLAN Part 4 "Kova without a C compiler", steps 3 to 5): glues the program's
// object (Cranelift's) and the runtime (an archive of objects made by the Rust compiler, build.rs)
// into an executable, without the system's linker. It knows only what these two produce and
// refuses anything else loudly: it is not a general linker.
//
// 1. Every defined symbol of every object is noted (a strong one wins over a weak one).
// 2. From the entry point, the sections reached through relocations are kept: the functions a
//    program never calls are not in its executable.
// 3. The kept sections get their addresses: code, constants, then data, then zeros.
// 4. Each relocation is written: the address (or the distance to it) the code needs.
//
// `elf`: Linux x86-64, a static executable (no dynamic linker). `pe`: Windows x86-64, whose
// functions come from `kernel32.dll` through its import table.

use std::collections::{HashMap, HashSet};

use object::read::archive::ArchiveFile;
use object::{Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationKind, RelocationTarget, SectionIndex, SectionKind, SymbolIndex, SymbolSection};

/// A section of an object: (object number, section index).
type Place = (usize, SectionIndex);

/// What a relocation points to.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Target {
    /// In a kept section, at this offset.
    At(Place, u64),
    /// A weak symbol defined nowhere: address 0.
    Nothing,
    /// A function of the system (Windows: its name in `kernel32.dll`); true for the place where
    /// the system writes its address (`__imp_` + its name).
    Import(String, bool),
}

/// Where a kept section goes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Part {
    Code,
    Constants,
    Data,
    Zeros,
}

struct Inputs<'a> {
    objects: Vec<object::File<'a>>,
    /// Name → (object, symbol, weak).
    defined: HashMap<&'a str, (usize, SymbolIndex, bool)>,
    /// Undefined symbols are the system's (Windows), not errors.
    imports: bool,
}

fn error(e: object::Error) -> String {
    e.to_string()
}

impl<'a> Inputs<'a> {
    fn read(main: &'a [u8], runtime: &'a [u8], imports: bool) -> Result<Self, String> {
        let mut objects = vec![object::File::parse(main).map_err(error)?];
        for member in ArchiveFile::parse(runtime).map_err(error)?.members() {
            objects.push(object::File::parse(member.map_err(error)?.data(runtime).map_err(error)?).map_err(error)?);
        }
        let mut defined: HashMap<&str, (usize, SymbolIndex, bool)> = HashMap::new();
        for (k, o) in objects.iter().enumerate() {
            for s in o.symbols() {
                if s.section() == SymbolSection::Common {
                    return Err(format!("common symbol `{}`", s.name().unwrap_or("?")));
                }
                if s.is_local() || !matches!(s.section(), SymbolSection::Section(_)) {
                    continue;
                }
                let name = s.name().map_err(error)?;
                // A function in a section of its own may come in several objects (COFF's COMDAT):
                // any one of them will do.
                let weak = s.is_weak() || comdat(o, &s);
                match defined.get(name) {
                    Some(&(_, _, false)) if !weak => return Err(format!("`{name}` defined twice")),
                    Some(&(_, _, was_weak)) if !was_weak || weak => {}
                    _ => {
                        defined.insert(name, (k, s.index(), weak));
                    }
                }
            }
        }
        Ok(Inputs { objects, defined, imports })
    }

    fn target(&self, k: usize, t: RelocationTarget) -> Result<Target, String> {
        let s = match t {
            RelocationTarget::Section(i) => return Ok(Target::At((k, i), 0)),
            RelocationTarget::Symbol(i) => self.objects[k].symbol_by_index(i).map_err(error)?,
            _ => return Err("absolute relocation".into()),
        };
        let (k, s) = match s.section() {
            SymbolSection::Section(_) if s.is_local() => (k, s),
            _ => {
                let name = s.name().map_err(error)?;
                match self.defined.get(name) {
                    Some(&(k2, i, _)) => (k2, self.objects[k2].symbol_by_index(i).map_err(error)?),
                    None if self.imports => {
                        return Ok(match name.strip_prefix("__imp_") {
                            Some(f) => Target::Import(f.to_string(), true),
                            None => Target::Import(name.to_string(), false),
                        });
                    }
                    None if s.is_weak() => return Ok(Target::Nothing),
                    None => return Err(format!("`{name}` is defined nowhere")),
                }
            }
        };
        match s.section() {
            SymbolSection::Section(i) => Ok(Target::At((k, i), self.offset(k, &s)?)),
            _ => Err(format!("`{}` is in no section", s.name().unwrap_or("?"))),
        }
    }

    /// 2. The sections reached from `entry` (and the place of `entry`), the targets reached
    /// through the GOT (`got`: an ELF relocation reading an address from it), and the system's
    /// functions.
    fn reach(&self, entry: &str, got: fn(RelocationFlags) -> bool) -> Result<Reached, String> {
        let &(k, i, _) = self.defined.get(entry).ok_or(format!("no `{entry}`"))?;
        let s = self.objects[k].symbol_by_index(i).map_err(error)?;
        let first = (k, s.section_index().ok_or(format!("`{entry}` is in no section"))?);
        let mut r = Reached { kept: vec![first], entry: (first, self.offset(k, &s)?), got: vec![], imports: vec![] };
        let mut seen: HashSet<Place> = HashSet::from([first]);
        let mut seen_targets = HashSet::new();
        let mut i = 0;
        while i < r.kept.len() {
            let (k, s) = r.kept[i];
            i += 1;
            for (_, rel) in self.objects[k].section_by_index(s).map_err(error)?.relocations() {
                let t = self.target(k, rel.target())?;
                if got(rel.flags()) && seen_targets.insert(t.clone()) {
                    r.got.push(t.clone());
                }
                match t {
                    Target::At(p, _) if seen.insert(p) => r.kept.push(p),
                    Target::Import(name, _) if !r.imports.contains(&name) => r.imports.push(name),
                    _ => {}
                }
            }
        }
        Ok(r)
    }

    /// Where symbol `s` of object `k` is in its section (a Mach-O object gives its sections
    /// addresses, ELF and COFF ones don't).
    fn offset(&self, k: usize, s: &object::Symbol) -> Result<u64, String> {
        let i = s.section_index().ok_or("a symbol in no section")?;
        Ok(s.address() - self.section((k, i))?.address())
    }

    fn section(&self, p: Place) -> Result<object::Section<'a, '_>, String> {
        self.objects[p.0].section_by_index(p.1).map_err(error)
    }

    fn part(&self, p: Place) -> Result<Part, String> {
        let section = self.section(p)?;
        Ok(match section.kind() {
            SectionKind::Text => Part::Code,
            SectionKind::ReadOnlyData | SectionKind::ReadOnlyString | SectionKind::ReadOnlyDataWithRel => Part::Constants,
            SectionKind::Data => Part::Data,
            SectionKind::UninitializedData => Part::Zeros,
            kind => return Err(format!("section `{}` of an unknown kind ({kind:?})", section.name().unwrap_or("?"))),
        })
    }

    /// The kept sections sorted by part, with their parts.
    fn sorted(&self, r: &mut Reached) -> Result<HashMap<Place, Part>, String> {
        let parts: HashMap<Place, Part> = r.kept.iter().map(|&p| Ok((p, self.part(p)?))).collect::<Result<_, String>>()?;
        r.kept.sort_by_key(|p| parts[p]);
        Ok(parts)
    }

    /// 3. Gives each section of `kept` (sorted by part) its address, from `at` on; `next` is
    /// called when the parts' kinds change (before the first of them), and may move `at`.
    fn place(&self, kept: &[Place], mut at: u64, mut next: impl FnMut(Part, &mut u64), address: &mut HashMap<Place, u64>) -> Result<u64, String> {
        let mut part = None;
        for &p in kept {
            let this = self.part(p)?;
            if part != Some(this) {
                next(this, &mut at);
                part = Some(this);
            }
            let section = self.section(p)?;
            at = align(at, section.align().max(1));
            address.insert(p, at);
            at += section.size();
        }
        Ok(at)
    }

    /// 4. Copies the kept sections into `out` (`offset`: an address's place in it) and writes
    /// their relocations; `address_of` gives a target's address, `apply` writes one relocation
    /// (at its place, the target's address plus the addend).
    fn write(
        &self,
        kept: &[Place],
        address: &HashMap<Place, u64>,
        offset: impl Fn(u64) -> usize,
        out: &mut [u8],
        address_of: impl Fn(&Target) -> Result<u64, String>,
        apply: impl Fn(&mut [u8], &object::Relocation, &Target, u64, i64) -> Result<(), String>,
    ) -> Result<(), String> {
        for &p in kept {
            if self.part(p)? == Part::Zeros {
                continue;
            }
            let section = self.section(p)?;
            let data = section.data().map_err(error)?;
            let o = offset(address[&p]);
            out[o..o + data.len()].copy_from_slice(data);
            for (r_offset, r) in section.relocations() {
                let t = self.target(p.0, r.target())?;
                let at = o + r_offset as usize;
                // COFF keeps a part of the value in the code itself.
                let mut a = r.addend();
                if r.has_implicit_addend() {
                    a += match r.size() {
                        32 => i32::from_le_bytes(out[at..at + 4].try_into().unwrap()) as i64,
                        64 => i64::from_le_bytes(out[at..at + 8].try_into().unwrap()),
                        n => return Err(format!("relocation of {n} bits")),
                    };
                }
                let value = address_of(&t)? as i64 + a;
                apply(&mut out[at..], &r, &t, address[&p] + r_offset, value).map_err(|e| format!("{e} in `{}`", section.name().unwrap_or("?")))?;
            }
        }
        Ok(())
    }
}

/// Whether symbol `s` is in a COMDAT section of a COFF object.
fn comdat(o: &object::File, s: &object::Symbol) -> bool {
    let Some(i) = s.section_index() else { return false };
    o.format() == object::BinaryFormat::Coff
        && o.section_by_index(i).is_ok_and(|sec| matches!(sec.flags(), object::SectionFlags::Coff { characteristics } if characteristics.0 & object::pe::IMAGE_SCN_LNK_COMDAT.0 != 0))
}

struct Reached {
    kept: Vec<Place>,
    entry: (Place, u64),
    got: Vec<Target>,
    imports: Vec<String>,
}

fn align(a: u64, to: u64) -> u64 {
    a.div_ceil(to) * to
}

fn put(out: &mut [u8], value: i64, bits: u8) -> Result<(), String> {
    match bits {
        64 => out[..8].copy_from_slice(&value.to_le_bytes()),
        32 => {
            let v = i32::try_from(value).or_else(|_| u32::try_from(value).map(|v| v as i32)).map_err(|_| "relocation too far")?;
            out[..4].copy_from_slice(&v.to_le_bytes());
        }
        n => return Err(format!("relocation of {n} bits")),
    }
    Ok(())
}

const PAGE: u64 = 0x1000;

// ---- Linux: ELF ----

const ELF_BASE: u64 = 0x40_0000;
const PROGRAM_HEADERS: u64 = 3;

fn is_got(flags: RelocationFlags) -> bool {
    use object::elf::*;
    matches!(flags, RelocationFlags::Elf { r_type: R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX })
}

/// The executable of `main` (the program's object) and `runtime` (an archive), for Linux x86-64.
/// Headers, code, constants and the GOT (the addresses Cranelift's code reads) in a first segment
/// (read and run); data then zeros in a second one (read and write), which starts on a new page at
/// the same place in the page as in the file (the system maps whole pages).
pub fn elf(main: &[u8], runtime: &[u8]) -> Result<Vec<u8>, String> {
    let inputs = Inputs::read(main, runtime, false)?;
    let mut r = inputs.reach("_start", is_got)?;
    let parts = inputs.sorted(&mut r)?;
    let split = r.kept.iter().position(|p| parts[p] >= Part::Data).unwrap_or(r.kept.len());
    let (first, second) = r.kept.split_at(split);

    let mut address = HashMap::new();
    let at = inputs.place(first, ELF_BASE + 64 + 56 * PROGRAM_HEADERS, |_, _| {}, &mut address)?;
    let got_at = align(at, 8);
    let first_end = got_at + 8 * r.got.len() as u64;
    // An address of the second segment is `shift` further than its place in the file.
    let shift = align(first_end, PAGE) + first_end % PAGE - first_end;
    let data_at = first_end + shift;
    let mut zeros = None;
    let end = inputs.place(
        second,
        data_at,
        |part, at| {
            if part == Part::Zeros {
                zeros = Some(*at)
            }
        },
        &mut address,
    )?;
    let zeros = zeros.unwrap_or(end);

    let offset = |a: u64| (if a < data_at { a - ELF_BASE } else { a - shift - ELF_BASE }) as usize;
    let mut out = vec![0u8; offset(zeros)];
    let address_of = |t: &Target| match t {
        Target::At(p, off) => Ok(address[p] + off),
        Target::Nothing => Ok(0),
        Target::Import(name, _) => Err(format!("`{name}` is defined nowhere")),
    };
    for (slot, t) in r.got.iter().enumerate() {
        let o = offset(got_at) + 8 * slot;
        out[o..o + 8].copy_from_slice(&address_of(t)?.to_le_bytes());
    }
    let apply = |out: &mut [u8], rel: &object::Relocation, t: &Target, place: u64, value: i64| {
        use object::elf::*;
        let RelocationFlags::Elf { r_type } = rel.flags() else { return Err("not an ELF relocation".to_string()) };
        let place = place as i64;
        match r_type {
            R_X86_64_64 => put(out, value, 64),
            R_X86_64_PC64 => put(out, value - place, 64),
            // Static: a call through the PLT goes straight to the function.
            R_X86_64_PC32 | R_X86_64_PLT32 => put(out, value - place, 32),
            R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                let slot = r.got.iter().position(|g| g == t).unwrap() as i64;
                put(out, got_at as i64 + 8 * slot + rel.addend() - place, 32)
            }
            R_X86_64_32 if (0..=u32::MAX as i64).contains(&value) => put(out, value, 32),
            R_X86_64_32S => put(out, value, 32),
            _ => Err(format!("relocation of type {r_type}")),
        }
    };
    inputs.write(&r.kept, &address, offset, &mut out, address_of, apply)?;

    // The headers: the ELF header, then the segments (no sections' headers, like `strip`).
    let mut h = Vec::new();
    h.extend_from_slice(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0"); // 64 bits, little-endian, version 1
    h.extend_from_slice(&2u16.to_le_bytes()); // an executable
    h.extend_from_slice(&62u16.to_le_bytes()); // x86-64
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&(address[&r.entry.0] + r.entry.1).to_le_bytes());
    h.extend_from_slice(&64u64.to_le_bytes()); // the segments' headers follow
    h.extend_from_slice(&0u64.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    for v in [64u16, 56, PROGRAM_HEADERS as u16, 64, 0, 0] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    let mut segment = |kind: u32, flags: u32, offset: u64, address: u64, file: u64, memory: u64| {
        for v in [kind, flags] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        for v in [offset, address, address, file, memory, PAGE] {
            h.extend_from_slice(&v.to_le_bytes());
        }
    };
    segment(1, 5, 0, ELF_BASE, first_end - ELF_BASE, first_end - ELF_BASE); // PT_LOAD, read + run
    segment(1, 6, offset(data_at) as u64, data_at, zeros - data_at, end - data_at); // PT_LOAD, read + write
    segment(0x6474e551, 6, 0, 0, 0, 0); // PT_GNU_STACK: the stack does not run
    out[..h.len()].copy_from_slice(&h);
    Ok(out)
}

// ---- Windows: PE ----

const PE_BASE: u64 = 0x1_4000_0000;
const FILE_ALIGN: u64 = 0x200;

/// The executable of `main` and `runtime`, for Windows x86-64: three sections, `.text` (code, then
/// the jumps to the system's functions), `.rdata` (constants, then the import table, which Windows
/// fills with the addresses of `kernel32.dll`'s functions) and `.data` (data then zeros).
pub fn pe(main: &[u8], runtime: &[u8]) -> Result<Vec<u8>, String> {
    let inputs = Inputs::read(main, runtime, true)?;
    let mut r = inputs.reach("kova_start", |_| false)?;
    inputs.sorted(&mut r)?;
    let headers = align(0x40 + 4 + 20 + 240 + 40 * 3, FILE_ALIGN);
    let n = r.imports.len() as u64;

    // Where each section starts and ends (addresses); the jumps after the code.
    let mut address = HashMap::new();
    let (mut starts, mut ends) = ([PE_BASE + PAGE, 0, 0], [0u64; 3]);
    let (mut zeros, mut thunks, mut tables) = (None, 0, 0);
    // Entering `part`: what comes before it is closed (even when a part has no section).
    let mut enter = |part: Part, at: &mut u64| {
        if part >= Part::Constants && ends[0] == 0 {
            thunks = *at;
            ends[0] = thunks + 6 * n;
            starts[1] = align(ends[0], PAGE);
            *at = starts[1];
        }
        if part >= Part::Data && ends[1] == 0 {
            tables = align(*at, 8);
            ends[1] = tables + tables_size(&r.imports);
            starts[2] = align(ends[1], PAGE);
            *at = starts[2];
        }
        if part == Part::Zeros && zeros.is_none() {
            zeros = Some(*at);
        }
    };
    let mut end = inputs.place(&r.kept, PE_BASE + PAGE, &mut enter, &mut address)?;
    enter(Part::Zeros, &mut end);
    ends[2] = end;
    let zeros = zeros.unwrap_or(end);

    // The import table: the addresses (IAT, filled by Windows), the same list for Windows to look
    // the names up (ILT), the directory (one DLL, then an empty entry), the DLL's name, then each
    // function's name (after a 2-byte "hint").
    let iat = tables;
    let ilt = iat + 8 * (n + 1);
    let directory = ilt + 8 * (n + 1);
    let dll_name = directory + 40;
    let mut names = vec![];
    let mut name_at = dll_name + 16;
    for f in &r.imports {
        names.push(name_at);
        name_at = align(name_at + 2 + f.len() as u64 + 1, 2);
    }

    let rva = |a: u64| a - PE_BASE;
    // Each section's place in the file and size there (the zeros are not in the file).
    let file_ends = [ends[0], ends[1], zeros];
    let sizes: Vec<u64> = (0..3).map(|k| align(file_ends[k] - starts[k], FILE_ALIGN)).collect();
    let raw = |k: usize| (headers + sizes[..k].iter().sum::<u64>(), sizes[k]);
    let offset = |a: u64| {
        let k = if a >= starts[2] { 2 } else if a >= starts[1] { 1 } else { 0 };
        (raw(k).0 + a - starts[k]) as usize
    };
    let mut out = vec![0u8; (raw(2).0 + raw(2).1) as usize];

    let index = |name: &str| r.imports.iter().position(|f| f == name).unwrap() as u64;
    let address_of = |t: &Target| match t {
        Target::At(p, off) => Ok(address[p] + off),
        Target::Nothing => Ok(0),
        Target::Import(name, true) => Ok(iat + 8 * index(name)),
        Target::Import(name, false) => Ok(thunks + 6 * index(name)),
    };
    let apply = |out: &mut [u8], rel: &object::Relocation, _: &Target, place: u64, value: i64| match (rel.kind(), rel.size()) {
        (RelocationKind::Absolute, 64) => put(out, value, 64),
        // The addend (-4…) already counts from the end of the instruction.
        (RelocationKind::Relative, 32) => put(out, value - place as i64, 32),
        (RelocationKind::ImageOffset, 32) => put(out, value - PE_BASE as i64, 32),
        (kind, size) => Err(format!("relocation {kind:?} of {size} bits")),
    };
    inputs.write(&r.kept, &address, offset, &mut out, address_of, apply)?;

    for (i, f) in r.imports.iter().enumerate() {
        let (i, t) = (i as u64, offset(thunks + 6 * i as u64));
        out[t..t + 2].copy_from_slice(&[0xFF, 0x25]); // jmp [rip + …]
        out[t + 2..t + 6].copy_from_slice(&((iat + 8 * i) as i32 - (thunks + 6 * i + 6) as i32).to_le_bytes());
        for table in [iat, ilt] {
            let o = offset(table + 8 * i);
            out[o..o + 8].copy_from_slice(&rva(names[i as usize]).to_le_bytes());
        }
        let o = offset(names[i as usize]) + 2;
        out[o..o + f.len()].copy_from_slice(f.as_bytes());
    }
    let d = offset(directory);
    for (k, v) in [(0, rva(ilt)), (12, rva(dll_name)), (16, rva(iat))] {
        out[d + k..d + k + 4].copy_from_slice(&(v as u32).to_le_bytes());
    }
    let o = offset(dll_name);
    out[o..o + 12].copy_from_slice(b"KERNEL32.dll");

    // The headers: MS-DOS's (which only says where the next one is), PE's, the "optional" one
    // (which every executable has), then the three sections'.
    let mut h = vec![0u8; 0x40];
    h[..2].copy_from_slice(b"MZ");
    h[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    h.extend_from_slice(b"PE\0\0");
    let le16 = |h: &mut Vec<u8>, v: u16| h.extend_from_slice(&v.to_le_bytes());
    let le32 = |h: &mut Vec<u8>, v: u64| h.extend_from_slice(&(v as u32).to_le_bytes());
    let le64 = |h: &mut Vec<u8>, v: u64| h.extend_from_slice(&v.to_le_bytes());
    le16(&mut h, 0x8664); // x86-64
    le16(&mut h, 3); // sections
    for _ in 0..3 {
        le32(&mut h, 0); // date, no symbols
    }
    le16(&mut h, 240); // size of the optional header
    le16(&mut h, 0x23); // an executable, without relocations, using addresses above 2 GB
    le16(&mut h, 0x20B); // PE32+
    h.extend_from_slice(&[14, 0]); // "linker version"
    le32(&mut h, sizes[0]);
    le32(&mut h, sizes[1] + sizes[2]);
    le32(&mut h, end - zeros);
    le32(&mut h, rva(address[&r.entry.0] + r.entry.1));
    le32(&mut h, rva(starts[0]));
    le64(&mut h, PE_BASE);
    le32(&mut h, PAGE);
    le32(&mut h, FILE_ALIGN);
    for v in [6u16, 0, 0, 0, 6, 0] {
        le16(&mut h, v); // Windows Vista and later
    }
    le32(&mut h, 0);
    le32(&mut h, rva(align(end, PAGE))); // size of the image
    le32(&mut h, headers);
    le32(&mut h, 0); // checksum
    le16(&mut h, 3); // a console program
    le16(&mut h, 0x8100); // data never runs (NX), terminal server aware; a fixed address
    for v in [0x10_0000u64, PAGE, 0x10_0000, PAGE] {
        le64(&mut h, v); // stack and heap: reserved, committed
    }
    le32(&mut h, 0);
    le32(&mut h, 16); // data directories
    for k in 0..16 {
        let (at, size) = match k {
            1 => (rva(directory), 40),     // the imports
            12 => (rva(iat), 8 * (n + 1)), // their addresses
            _ => (0, 0),
        };
        le32(&mut h, at);
        le32(&mut h, size);
    }
    for (k, (name, flags)) in [(*b".text\0\0\0", 0x6000_0020), (*b".rdata\0\0", 0x4000_0040), (*b".data\0\0\0", 0xC000_0040)].into_iter().enumerate() {
        h.extend_from_slice(&name);
        le32(&mut h, ends[k] - starts[k]);
        le32(&mut h, rva(starts[k]));
        le32(&mut h, raw(k).1);
        le32(&mut h, raw(k).0);
        for _ in 0..3 {
            le32(&mut h, 0);
        }
        le32(&mut h, flags);
    }
    out[..h.len()].copy_from_slice(&h);
    Ok(out)
}

/// The size of the import table of `imports` (see `pe`).
fn tables_size(imports: &[String]) -> u64 {
    let n = imports.len() as u64;
    let names: u64 = imports.iter().map(|f| align(2 + f.len() as u64 + 1, 2)).sum();
    16 * (n + 1) + 40 + 16 + names
}
