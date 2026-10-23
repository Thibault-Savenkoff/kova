// Kova's own linker (PLAN Part 4 "Kova without a C compiler", steps 3 to 5): glues the program's
// object (Cranelift's) and the runtime (an archive of objects made by the Rust compiler, build.rs)
// into an executable, without the system's linker. It knows only what these two produce and
// refuses anything else loudly: it is not a general linker.
//
// 1. Every defined symbol of every object is noted (a strong one wins over a weak one).
// 2. From the entry point, the sections reached through relocations are kept: the functions a
//    program never calls are not in its executable. A Mach-O section holds many functions: it is
//    cut at each name, as Apple's linker does.
// 3. The kept sections get their addresses: code, constants, then data, then zeros.
// 4. Each relocation is written: the address (or the distance to it) the code needs.
//
// `elf`: Linux x86-64 or arm64, static (no dynamic linker) unless it calls the C library. `pe`:
// Windows x86-64 or arm64, whose functions come from `kernel32.dll` (and `ws2_32.dll` for the
// network) through its import table. `macho`: macOS arm64 or
// x86-64, whose functions come from `libSystem` through `dyld`, signed as the Mac requires.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use object::read::archive::ArchiveFile;
use object::{Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationKind, RelocationTarget, SectionIndex, SectionKind, SymbolIndex, SymbolSection};

/// A section of an object, or a piece of one: (object number, section index, where the piece
/// starts in it).
type Place = (usize, SectionIndex, u64);

/// What a relocation points to.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Target {
    /// In a kept section, at this offset.
    At(Place, u64),
    /// A weak symbol defined nowhere: address 0.
    Nothing,
    /// A function of the system (Windows: its name in its DLL); true for the place where
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
    /// Where the pieces of a cut section start (0 first).
    pieces: HashMap<(usize, SectionIndex), Vec<u64>>,
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
        let pieces = objects.iter().enumerate().flat_map(|(k, o)| cut(k, o)).collect();
        Ok(Inputs { objects, defined, imports, pieces })
    }

    /// The piece of section `i` of object `k` holding `offset`, and where `offset` is in it.
    fn piece(&self, k: usize, i: SectionIndex, offset: u64) -> (Place, u64) {
        let start = self.pieces.get(&(k, i)).map_or(0, |starts| starts[starts.partition_point(|&s| s <= offset) - 1]);
        ((k, i, start), offset - start)
    }

    /// Where piece `p` starts and ends in its section.
    fn range(&self, p: Place) -> Result<(u64, u64), String> {
        let starts = self.pieces.get(&(p.0, p.1));
        let next = starts.and_then(|s| s.iter().find(|&&s| s > p.2));
        Ok((p.2, match next {
            Some(&end) => end,
            None => self.section(p)?.size(),
        }))
    }

    /// The relocations of piece `p`, with their offsets in it and in the section.
    fn relocations(&self, p: Place) -> Result<Vec<(u64, u64, object::Relocation)>, String> {
        let (start, end) = self.range(p)?;
        Ok(self.section(p)?.relocations().filter(|(o, _)| (start..end).contains(o)).map(|(o, r)| (o - start, o, r)).collect())
    }

    fn target(&self, k: usize, t: RelocationTarget) -> Result<Target, String> {
        let s = match t {
            // Mach-O: the place holds the target's address in its object, from its section's.
            // Such a section is never cut (`cut`).
            RelocationTarget::Section(i) => return Ok(Target::At((k, i, 0), 0u64.wrapping_sub(self.objects[k].section_by_index(i).map_err(error)?.address()))),
            RelocationTarget::Symbol(i) => self.objects[k].symbol_by_index(i).map_err(error)?,
            _ => return Err("absolute relocation".into()),
        };
        let (k, s) = match s.section() {
            SymbolSection::Section(_) if s.is_local() => (k, s),
            _ => {
                let name = s.name().map_err(error)?;
                match self.defined.get(name) {
                    Some(&(k2, i, _)) => (k2, self.objects[k2].symbol_by_index(i).map_err(error)?),
                    None if s.is_weak() => return Ok(Target::Nothing),
                    None if self.imports => {
                        return Ok(match name.strip_prefix("__imp_") {
                            Some(f) => Target::Import(f.to_string(), true),
                            None => Target::Import(name.to_string(), false),
                        });
                    }
                    None => return Err(format!("`{name}` is defined nowhere")),
                }
            }
        };
        let (p, offset) = self.at(k, &s)?;
        Ok(Target::At(p, offset))
    }

    /// 2. The sections reached from `entry` (and the place of `entry`), the targets reached
    /// through the GOT (`got`: an ELF relocation reading an address from it), and the system's
    /// functions.
    fn reach(&self, entry: &str, got: fn(RelocationFlags) -> bool) -> Result<Reached, String> {
        let &(k, i, _) = self.defined.get(entry).ok_or(format!("no `{entry}`"))?;
        let s = self.objects[k].symbol_by_index(i).map_err(error)?;
        let (first, offset) = self.at(k, &s)?;
        let mut r = Reached { kept: vec![first], entry: (first, offset), got: vec![], imports: vec![] };
        let mut seen: HashSet<Place> = HashSet::from([first]);
        let mut seen_targets = HashSet::new();
        let mut i = 0;
        while i < r.kept.len() {
            let p = r.kept[i];
            i += 1;
            for (_, _, rel) in self.relocations(p)? {
                let t = self.target(p.0, rel.target())?;
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

    /// The piece holding symbol `s` of object `k`, and where it is in it (a Mach-O object gives
    /// its sections addresses, ELF and COFF ones don't).
    fn at(&self, k: usize, s: &object::Symbol) -> Result<(Place, u64), String> {
        let i = s.section_index().ok_or(format!("`{}` is in no section", s.name().unwrap_or("?")))?;
        Ok(self.piece(k, i, s.address() - self.objects[k].section_by_index(i).map_err(error)?.address()))
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
            // A piece is aligned as its section, or less if it starts at a smaller power of 2.
            let (start, end) = self.range(p)?;
            let alignment = self.section(p)?.align().max(1);
            at = align(at, if start == 0 { alignment } else { alignment.min(1 << start.trailing_zeros()) });
            address.insert(p, at);
            at += end - start;
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
            let (start, end) = self.range(p)?;
            let data = &section.data().map_err(error)?[start as usize..end as usize];
            let o = offset(address[&p]);
            out[o..o + data.len()].copy_from_slice(data);
            for (r_offset, in_section, r) in self.relocations(p)? {
                let t = self.target(p.0, r.target())?;
                let at = o + r_offset as usize;
                // COFF and x86-64 Mach-O keep a part of the value in the code itself; arm64 Mach-O
                // only in a pointer (its instructions get theirs from a relocation of their own);
                // arm64 COFF in an instruction's own number (read by `pe`).
                let mut a = r.addend();
                let arm = self.objects[p.0].architecture() == object::Architecture::Aarch64;
                let in_instruction = match r.flags() {
                    RelocationFlags::MachO { r_type, .. } => r_type != object::macho::ARM64_RELOC_UNSIGNED,
                    RelocationFlags::Coff { .. } => !matches!(r.size(), 32 | 64),
                    _ => false,
                };
                if r.has_implicit_addend() && !(arm && in_instruction) {
                    a += match r.size() {
                        32 => i32::from_le_bytes(out[at..at + 4].try_into().unwrap()) as i64,
                        64 => i64::from_le_bytes(out[at..at + 8].try_into().unwrap()),
                        n => return Err(format!("relocation of {n} bits")),
                    };
                    // x86-64 Mach-O: a distance to a section was measured from the place in the object.
                    if let (RelocationFlags::MachO { r_pcrel: true, .. }, RelocationTarget::Section(_)) = (r.flags(), r.target()) {
                        a += (section.address() + in_section) as i64 - r.addend();
                    }
                }
                let mut value = (address_of(&t)? as i64).wrapping_add(a);
                // Mach-O: a distance between two names (a jump table).
                if let Some(s) = r.subtractor() {
                    if matches!(r.target(), RelocationTarget::Section(_)) {
                        return Err("a distance from a section".into());
                    }
                    value -= address_of(&self.target(p.0, RelocationTarget::Symbol(s))?)? as i64;
                }
                apply(&mut out[at..], &r, &t, address[&p] + r_offset, value).map_err(|e| format!("{e} in `{}`", section.name().unwrap_or("?")))?;
            }
        }
        Ok(())
    }
}

/// Where the sections of Mach-O object `o` (number `k`) are cut: at each name, when its compiler
/// promises that no code goes from one name to the next ("subsections via symbols"). A section
/// that a relocation reaches without a name stays whole (the debug information and unwind tables
/// do, but they are never kept).
fn cut(k: usize, o: &object::File) -> Vec<((usize, SectionIndex), Vec<u64>)> {
    if !matches!(o.flags(), object::FileFlags::MachO { flags } if flags.0 & object::macho::MH_SUBSECTIONS_VIA_SYMBOLS.0 != 0) {
        return vec![];
    }
    let kept = |s: &object::Section| s.kind() != SectionKind::Debug && !matches!(s.name(), Ok("__compact_unwind" | "__eh_frame"));
    let whole: HashSet<SectionIndex> = o.sections().filter(kept).flat_map(|s| s.relocations().filter_map(|(_, r)| match r.target() {
        RelocationTarget::Section(i) => Some(i),
        _ => None,
    }).collect::<Vec<_>>()).collect();
    let mut starts: HashMap<SectionIndex, Vec<u64>> = HashMap::new();
    for sym in o.symbols() {
        if let Some(i) = sym.section_index().filter(|i| !whole.contains(i)) {
            let Ok(section) = o.section_by_index(i) else { continue };
            starts.entry(i).or_insert_with(|| vec![0]).push(sym.address() - section.address());
        }
    }
    starts.into_iter().map(|(i, mut s)| {
        s.sort();
        s.dedup();
        ((k, i), s)
    }).collect()
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

fn is_got(flags: RelocationFlags) -> bool {
    use object::elf::*;
    matches!(
        flags,
        RelocationFlags::Elf { r_type: R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX | R_AARCH64_ADR_GOT_PAGE | R_AARCH64_LD64_GOT_LO12_NC }
    )
}

/// The executable of `main` (the program's object) and `runtime` (an archive), for Linux x86-64
/// (`arm`: arm64). Headers, code, constants and the GOT (the addresses Cranelift's code reads) in a
/// first segment (read and run); data then zeros in a second one (read and write), which starts on
/// a new page at the same place in the page as in the file (the system maps whole pages).
pub fn elf(main: &[u8], runtime: &[u8], arm: bool) -> Result<Vec<u8>, String> {
    // An arm64 Linux may have pages of 4, 16 or 64 KB: aligned for the largest.
    let page_size = if arm { 0x10000 } else { PAGE };
    let inputs = Inputs::read(main, runtime, true)?;
    let mut r = inputs.reach("_start", is_got)?;
    // A program calling the C library's functions (the web's) is linked with it, by the system's
    // dynamic linker (`ld.so`) at launch, and starts through it: `kova_libc_start`.
    let dynamic = !r.imports.is_empty();
    if dynamic {
        r = inputs.reach("kova_libc_start", is_got)?;
        for f in r.imports.clone() {
            let t = Target::Import(f, false);
            if !r.got.contains(&t) {
                r.got.push(t); // its address, written by `ld.so`, read by its jump
            }
        }
    }
    let parts = inputs.sorted(&mut r)?;
    let split = r.kept.iter().position(|p| parts[p] >= Part::Data).unwrap_or(r.kept.len());
    let (first, second) = r.kept.split_at(split);
    let headers: u64 = if dynamic { 6 } else { 3 };
    let interp: &[u8] = if arm { b"/lib/ld-linux-aarch64.so.1\0" } else { b"/lib64/ld-linux-x86-64.so.2\0" };
    let interp_at = ELF_BASE + 64 + 56 * headers;

    // The first segment (read and run): the headers, the dynamic linker's name, code and
    // constants, the jumps to the C library's functions, then what `ld.so` reads: their names
    // (`dynsym`, `dynstr`), an empty hash table and where to write their addresses (`rela`).
    let mut address = HashMap::new();
    let at = inputs.place(first, if dynamic { interp_at + interp.len() as u64 } else { interp_at }, |_, _| {}, &mut address)?;
    let n = r.imports.len() as u64;
    let stub_size = if arm { 12 } else { 6 };
    let stubs = align(at, 4);
    let stub = |f: &str| stubs + stub_size * r.imports.iter().position(|g| g == f).unwrap() as u64;
    // `dlopen` (src/openssl.rs) is in `libc.so.6` since glibc 2.34, in `libdl.so.2` before.
    let mut strings = b"\0libc.so.6\0libdl.so.2\0".to_vec();
    let names: Vec<u64> = r.imports.iter().map(|f| {
        let at = strings.len() as u64;
        strings.extend(f.as_bytes());
        strings.push(0);
        at
    }).collect();
    let dynsym = align(stubs + stub_size * n, 8);
    let dynstr = dynsym + 24 * (n + 1);
    let hash = align(dynstr + strings.len() as u64, 8);
    let rela = align(hash + 4 * (3 + n + 1), 8);
    let first_end = align(if dynamic { rela + 24 * n } else { stubs }, 8);

    // The second one (read and write): the GOT, the `dynamic` table, data then zeros. Its
    // addresses are `shift` further than its place in the file.
    let shift = align(first_end, page_size) + first_end % page_size - first_end;
    let got_at = first_end + shift;
    let dynamic_at = got_at + 8 * r.got.len() as u64;
    let table: Vec<(u64, u64)> = if dynamic {
        // NEEDED (twice), HASH, STRTAB, SYMTAB, RELA, RELASZ, RELAENT, STRSZ, SYMENT; every address at once
        // (BIND_NOW, FLAGS = BIND_NOW, FLAGS_1 = NOW); the end.
        vec![(1, 1), (1, 11), (4, hash), (5, dynstr), (6, dynsym), (7, rela), (8, 24 * n), (9, 24), (10, strings.len() as u64), (11, 24), (24, 0), (30, 8), (0x6fff_fffb, 1), (0, 0)]
    } else {
        vec![]
    };
    let data_at = dynamic_at + 16 * table.len() as u64;
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

    let offset = |a: u64| (if a < got_at { a - ELF_BASE } else { a - shift - ELF_BASE }) as usize;
    let mut out = vec![0u8; offset(zeros)];
    let address_of = |t: &Target| match t {
        Target::At(p, off) => Ok(address[p] + off),
        Target::Nothing => Ok(0),
        Target::Import(name, _) => Ok(stub(name)),
    };
    for (slot, t) in r.got.iter().enumerate() {
        if !matches!(t, Target::Import(..)) {
            let o = offset(got_at) + 8 * slot;
            out[o..o + 8].copy_from_slice(&address_of(t)?.to_le_bytes());
        }
    }
    let slot_of = |t: &Target| got_at + 8 * r.got.iter().position(|g| g == t).unwrap() as u64;
    let apply = |out: &mut [u8], rel: &object::Relocation, t: &Target, place: u64, value: i64| {
        use object::elf::*;
        let RelocationFlags::Elf { r_type } = rel.flags() else { return Err("not an ELF relocation".to_string()) };
        let slot = || slot_of(t) as i64;
        let place = place as i64;
        match r_type {
            _ if arm => match r_type {
                R_AARCH64_ABS64 => put(out, value, 64),
                R_AARCH64_ABS32 => put(out, value, 32),
                R_AARCH64_PREL64 => put(out, value - place, 64),
                R_AARCH64_PREL32 => put(out, value - place, 32),
                R_AARCH64_CALL26 | R_AARCH64_JUMP26 => branch(out, value - place),
                R_AARCH64_ADR_PREL_PG_HI21 | R_AARCH64_ADR_PREL_PG_HI21_NC => page(out, value, place as u64),
                R_AARCH64_ADD_ABS_LO12_NC
                | R_AARCH64_LDST8_ABS_LO12_NC
                | R_AARCH64_LDST16_ABS_LO12_NC
                | R_AARCH64_LDST32_ABS_LO12_NC
                | R_AARCH64_LDST64_ABS_LO12_NC
                | R_AARCH64_LDST128_ABS_LO12_NC => in_page(out, value),
                R_AARCH64_ADR_GOT_PAGE => page(out, slot(), place as u64),
                R_AARCH64_LD64_GOT_LO12_NC => in_page(out, slot()),
                _ => Err(format!("relocation of type {r_type}")),
            },
            R_X86_64_64 => put(out, value, 64),
            R_X86_64_PC64 => put(out, value - place, 64),
            // A call through the PLT goes straight to the function (or to its jump).
            R_X86_64_PC32 | R_X86_64_PLT32 => put(out, value - place, 32),
            R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => put(out, slot() + rel.addend() - place, 32),
            R_X86_64_32 if (0..=u32::MAX as i64).contains(&value) => put(out, value, 32),
            R_X86_64_32S => put(out, value, 32),
            _ => Err(format!("relocation of type {r_type}")),
        }
    };
    inputs.write(&r.kept, &address, offset, &mut out, address_of, apply)?;

    if dynamic {
        let o = offset(interp_at);
        out[o..o + interp.len()].copy_from_slice(interp);
        for (k, f) in r.imports.iter().enumerate() {
            let (s, g) = (stub(f), slot_of(&Target::Import(f.clone(), false)));
            let o = offset(s);
            if arm {
                // adrp x16, slot@PAGE; ldr x16, [x16, slot@PAGEOFF]; br x16
                for (i, insn) in [0x9000_0010u32, 0xF940_0210, 0xD61F_0200].into_iter().enumerate() {
                    out[o + 4 * i..o + 4 * i + 4].copy_from_slice(&insn.to_le_bytes());
                }
                page(&mut out[o..], g as i64, s)?;
                in_page(&mut out[o + 4..], g as i64)?;
            } else {
                out[o..o + 2].copy_from_slice(&[0xFF, 0x25]); // jmp [rip + slot]
                put(&mut out[o + 2..], g as i64 - (s as i64 + 6), 32)?;
            }
            // Its name: global, a function, defined elsewhere.
            let o = offset(dynsym) + 24 * (k + 1);
            out[o..o + 4].copy_from_slice(&(names[k] as u32).to_le_bytes());
            out[o + 4] = 0x12;
            // Its address, written by `ld.so` into its slot (GLOB_DAT).
            let o = offset(rela) + 24 * k;
            out[o..o + 8].copy_from_slice(&g.to_le_bytes());
            out[o + 8..o + 16].copy_from_slice(&((k as u64 + 1) << 32 | if arm { 1025 } else { 6 }).to_le_bytes());
        }
        let o = offset(dynstr);
        out[o..o + strings.len()].copy_from_slice(&strings);
        // One bucket, empty: the program gives no names to the libraries.
        let o = offset(hash);
        out[o..o + 4].copy_from_slice(&1u32.to_le_bytes());
        out[o + 4..o + 8].copy_from_slice(&(n as u32 + 1).to_le_bytes());
        for (k, (tag, value)) in table.iter().enumerate() {
            let o = offset(dynamic_at) + 16 * k;
            out[o..o + 8].copy_from_slice(&tag.to_le_bytes());
            out[o + 8..o + 16].copy_from_slice(&value.to_le_bytes());
        }
    }

    // The headers: the ELF header, then the segments (no sections' headers, like `strip`).
    let mut h = Vec::new();
    h.extend_from_slice(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0"); // 64 bits, little-endian, version 1
    h.extend_from_slice(&2u16.to_le_bytes()); // an executable
    h.extend_from_slice(&(if arm { 183u16 } else { 62 }).to_le_bytes()); // arm64 or x86-64
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&(address[&r.entry.0] + r.entry.1).to_le_bytes());
    h.extend_from_slice(&64u64.to_le_bytes()); // the segments' headers follow
    h.extend_from_slice(&0u64.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    for v in [64u16, 56, headers as u16, 64, 0, 0] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    let mut segment = |kind: u32, flags: u32, offset: u64, address: u64, file: u64, memory: u64| {
        for v in [kind, flags] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        for v in [offset, address, address, file, memory, page_size] {
            h.extend_from_slice(&v.to_le_bytes());
        }
    };
    if dynamic {
        segment(6, 4, 64, ELF_BASE + 64, 56 * headers, 56 * headers); // PT_PHDR: these headers
        segment(3, 4, interp_at - ELF_BASE, interp_at, interp.len() as u64, interp.len() as u64); // PT_INTERP
    }
    segment(1, 5, 0, ELF_BASE, first_end - ELF_BASE, first_end - ELF_BASE); // PT_LOAD, read + run
    segment(1, 6, offset(got_at) as u64, got_at, zeros - got_at, end - got_at); // PT_LOAD, read + write
    if dynamic {
        segment(2, 6, offset(dynamic_at) as u64, dynamic_at, 16 * table.len() as u64, 16 * table.len() as u64); // PT_DYNAMIC
    }
    segment(0x6474e551, 6, 0, 0, 0, 0); // PT_GNU_STACK: the stack does not run
    out[..h.len()].copy_from_slice(&h);
    Ok(out)
}

// ---- Windows: PE ----

const PE_BASE: u64 = 0x1_4000_0000;
const FILE_ALIGN: u64 = 0x200;

/// The executable of `main` and `runtime`, for Windows x86-64: three sections, `.text` (code, then
/// the jumps to the system's functions), `.rdata` (constants, then the import table, which Windows
/// fills with the addresses of the DLLs' functions) and `.data` (data then zeros).
pub fn pe(main: &[u8], runtime: &[u8], arm: bool) -> Result<Vec<u8>, String> {
    let inputs = Inputs::read(main, runtime, true)?;
    let mut r = inputs.reach("kova_start", |_| false)?;
    inputs.sorted(&mut r)?;
    // The functions grouped by DLL, `kernel32.dll`'s first.
    r.imports.sort_by_key(|f| DLLS.iter().position(|(d, _)| *d == dll(f)));
    let mut dlls: Vec<&str> = r.imports.iter().map(|f| dll(f)).collect();
    dlls.dedup();
    let headers = align(0x40 + 4 + 20 + 240 + 40 * 4, FILE_ALIGN);
    let n = r.imports.len() as u64;
    // A jump to a function of Windows, through its address in the IAT.
    let thunk = if arm { 12 } else { 6 };

    // Where each section starts and ends (addresses); the jumps after the code.
    let mut address = HashMap::new();
    let (mut starts, mut ends) = ([PE_BASE + PAGE, 0, 0], [0u64; 3]);
    let (mut zeros, mut thunks, mut tables) = (None, 0, 0);
    // Entering `part`: what comes before it is closed (even when a part has no section).
    let mut enter = |part: Part, at: &mut u64| {
        if part >= Part::Constants && ends[0] == 0 {
            thunks = *at;
            ends[0] = thunks + thunk * n;
            starts[1] = align(ends[0], PAGE);
            *at = starts[1];
        }
        if part >= Part::Data && ends[1] == 0 {
            tables = align(*at, 8);
            ends[1] = tables + tables_size(&r.imports, dlls.len() as u64);
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

    // The import table: the addresses (IAT, filled by Windows: each DLL's, then a 0), the same
    // list for Windows to look the names up (ILT), the directory (one entry per DLL, then an
    // empty one), the DLLs' names, then each function's name (after a 2-byte "hint").
    let g = dlls.len() as u64;
    let iat = tables;
    let ilt = iat + 8 * (n + g);
    let directory = ilt + 8 * (n + g);
    let dll_names = directory + 20 * (g + 1);
    let mut names = vec![];
    let mut name_at = dll_names + 16 * g;
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
    // A function's place in the IAT (or the ILT, from `ilt`): after the 0 of each DLL before its.
    let group = |name: &str| dlls.iter().position(|d| *d == dll(name)).unwrap() as u64;
    let slot = |table: u64, name: &str| table + 8 * (index(name) + group(name));
    let address_of = |t: &Target| match t {
        Target::At(p, off) => Ok(address[p] + off),
        Target::Nothing => Ok(0),
        Target::Import(name, true) => Ok(slot(iat, name)),
        Target::Import(name, false) => Ok(thunks + thunk * index(name)),
    };
    // The addresses written in the program, which Windows changes when it puts it elsewhere.
    let based = RefCell::new(vec![]);
    let apply = |out: &mut [u8], rel: &object::Relocation, t: &Target, place: u64, value: i64| {
        use object::pe::*;
        // arm64: an instruction, which holds its addend itself (see `Inputs::write`).
        let insn = || u32::from_le_bytes(out[..4].try_into().unwrap());
        match (rel.kind(), rel.size(), rel.flags()) {
            (RelocationKind::Absolute, 64, _) => {
                if *t != Target::Nothing {
                    based.borrow_mut().push(place);
                }
                put(out, value, 64)
            }
            // The addend (-4…) already counts from the end of the instruction.
            (RelocationKind::Relative, 32, _) => put(out, value - place as i64, 32),
            (RelocationKind::ImageOffset, 32, _) => put(out, value - PE_BASE as i64, 32),
            (_, _, RelocationFlags::Coff { typ }) if arm => match typ {
                IMAGE_REL_ARM64_BRANCH26 => branch(out, value + ((insn() << 6) as i32 >> 4) as i64 - place as i64),
                IMAGE_REL_ARM64_PAGEBASE_REL21 => {
                    let low = (insn() >> 29 & 3 | (insn() >> 3 & 0x1F_FFFC)) as i64;
                    page(out, value + (low << 43 >> 43), place)
                }
                IMAGE_REL_ARM64_PAGEOFFSET_12A => in_page(out, value + (insn() >> 10 & 0xFFF) as i64),
                IMAGE_REL_ARM64_PAGEOFFSET_12L => in_page(out, value + ((insn() >> 10 & 0xFFF) << scale(insn())) as i64),
                _ => Err(format!("relocation of type {typ}")),
            },
            // The program's own object, written for Linux (see `codegen::executable`).
            (_, _, RelocationFlags::Elf { r_type }) if arm => {
                use object::elf::*;
                match r_type {
                    R_AARCH64_CALL26 | R_AARCH64_JUMP26 => branch(out, value - place as i64),
                    R_AARCH64_ADR_PREL_PG_HI21 => page(out, value, place),
                    R_AARCH64_ADD_ABS_LO12_NC
                    | R_AARCH64_LDST8_ABS_LO12_NC
                    | R_AARCH64_LDST16_ABS_LO12_NC
                    | R_AARCH64_LDST32_ABS_LO12_NC
                    | R_AARCH64_LDST64_ABS_LO12_NC
                    | R_AARCH64_LDST128_ABS_LO12_NC => in_page(out, value),
                    _ => Err(format!("relocation of type {r_type}")),
                }
            }
            (kind, size, _) => Err(format!("relocation {kind:?} of {size} bits")),
        }
    };
    inputs.write(&r.kept, &address, offset, &mut out, address_of, apply)?;

    for (i, f) in r.imports.iter().enumerate() {
        let (i, t) = (i as u64, offset(thunks + thunk * i as u64));
        let at = slot(iat, f);
        if arm {
            // adrp x16, slot@PAGE; ldr x16, [x16, slot@PAGEOFF]; br x16
            for (k, insn) in [0x9000_0010u32, 0xF940_0210, 0xD61F_0200].into_iter().enumerate() {
                out[t + 4 * k..t + 4 * k + 4].copy_from_slice(&insn.to_le_bytes());
            }
            page(&mut out[t..], at as i64, thunks + thunk * i)?;
            in_page(&mut out[t + 4..], at as i64)?;
        } else {
            out[t..t + 2].copy_from_slice(&[0xFF, 0x25]); // jmp [rip + …]
            out[t + 2..t + 6].copy_from_slice(&(at as i32 - (thunks + 6 * i + 6) as i32).to_le_bytes());
        }
        for table in [iat, ilt] {
            let o = offset(slot(table, f));
            out[o..o + 8].copy_from_slice(&rva(names[i as usize]).to_le_bytes());
        }
        let o = offset(names[i as usize]) + 2;
        out[o..o + f.len()].copy_from_slice(f.as_bytes());
    }
    for (k, name) in dlls.iter().enumerate() {
        let first = r.imports.iter().find(|f| dll(f) == *name).unwrap();
        let (d, at) = (offset(directory + 20 * k as u64), dll_names + 16 * k as u64);
        for (o, v) in [(0, rva(slot(ilt, first))), (12, rva(at)), (16, rva(slot(iat, first)))] {
            out[d + o..d + o + 4].copy_from_slice(&(v as u32).to_le_bytes());
        }
        let o = offset(at);
        out[o..o + name.len()].copy_from_slice(name.as_bytes());
    }

    // A fourth section, `.reloc`: for each page of 4 KB, the places in it holding an address
    // (type 10: 64 bits). Windows on arm64 always puts a program at an address of its choosing.
    let mut based = based.into_inner();
    based.sort();
    let mut relocs = vec![];
    for places in based.chunk_by(|a, b| a >> 12 == b >> 12) {
        let size = 8 + 2 * places.len().next_multiple_of(2);
        relocs.extend((rva(places[0] & !0xFFF) as u32).to_le_bytes());
        relocs.extend((size as u32).to_le_bytes());
        for a in places {
            relocs.extend((0xA000 | (a & 0xFFF) as u16).to_le_bytes());
        }
        relocs.resize(relocs.len().next_multiple_of(4), 0);
    }
    if relocs.is_empty() {
        relocs.extend([0, 0, 0, 0, 8, 0, 0, 0]); // one page, no place
    }
    let reloc_at = align(end, PAGE);
    let reloc_raw = (out.len() as u64, align(relocs.len() as u64, FILE_ALIGN));
    out.extend(&relocs);
    out.resize((reloc_raw.0 + reloc_raw.1) as usize, 0);

    // The headers: MS-DOS's (which only says where the next one is), PE's, the "optional" one
    // (which every executable has), then the three sections'.
    let mut h = vec![0u8; 0x40];
    h[..2].copy_from_slice(b"MZ");
    h[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    h.extend_from_slice(b"PE\0\0");
    let le16 = |h: &mut Vec<u8>, v: u16| h.extend_from_slice(&v.to_le_bytes());
    let le32 = |h: &mut Vec<u8>, v: u64| h.extend_from_slice(&(v as u32).to_le_bytes());
    let le64 = |h: &mut Vec<u8>, v: u64| h.extend_from_slice(&v.to_le_bytes());
    le16(&mut h, if arm { 0xAA64 } else { 0x8664 }); // arm64 or x86-64
    le16(&mut h, 4); // sections
    for _ in 0..3 {
        le32(&mut h, 0); // date, no symbols
    }
    le16(&mut h, 240); // size of the optional header
    le16(&mut h, 0x22); // an executable using addresses above 2 GB
    le16(&mut h, 0x20B); // PE32+
    h.extend_from_slice(&[14, 0]); // "linker version"
    le32(&mut h, sizes[0]);
    le32(&mut h, sizes[1] + sizes[2] + reloc_raw.1);
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
    le32(&mut h, rva(align(reloc_at + relocs.len() as u64, PAGE))); // size of the image
    le32(&mut h, headers);
    le32(&mut h, 0); // checksum
    le16(&mut h, 3); // a console program
    le16(&mut h, 0x8160); // anywhere in the 64 bits (ASLR), data never runs (NX), terminal server aware
    for v in [0x10_0000u64, PAGE, 0x10_0000, PAGE] {
        le64(&mut h, v); // stack and heap: reserved, committed
    }
    le32(&mut h, 0);
    le32(&mut h, 16); // data directories
    for k in 0..16 {
        let (at, size) = match k {
            1 => (rva(directory), 40),     // the imports
            5 => (rva(reloc_at), relocs.len() as u64), // the places holding an address
            12 => (rva(iat), 8 * (n + 1)), // their addresses
            _ => (0, 0),
        };
        le32(&mut h, at);
        le32(&mut h, size);
    }
    let sections = [(*b".text\0\0\0", 0x6000_0020), (*b".rdata\0\0", 0x4000_0040), (*b".data\0\0\0", 0xC000_0040), (*b".reloc\0\0", 0x4200_0040)];
    for (k, (name, flags)) in sections.into_iter().enumerate() {
        let (start, size, file) = if k < 3 { (starts[k], ends[k] - starts[k], raw(k)) } else { (reloc_at, relocs.len() as u64, reloc_raw) };
        h.extend_from_slice(&name);
        le32(&mut h, size);
        le32(&mut h, rva(start));
        le32(&mut h, file.1);
        le32(&mut h, file.0);
        for _ in 0..3 {
            le32(&mut h, 0);
        }
        le32(&mut h, flags);
    }
    out[..h.len()].copy_from_slice(&h);
    Ok(out)
}

/// The size of the import table of `imports`, from `dlls` DLLs (see `pe`).
fn tables_size(imports: &[String], dlls: u64) -> u64 {
    let n = imports.len() as u64;
    let names: u64 = imports.iter().map(|f| align(2 + f.len() as u64 + 1, 2)).sum();
    16 * (n + dlls) + 20 * (dlls + 1) + 16 * dlls + names
}

/// The DLLs the runtime calls (runtime/windows.rs, src/schannel.rs) and their functions; the
/// first one has every function not named here.
const DLLS: [(&str, &[&str]); 3] = [
    ("KERNEL32.dll", &[]),
    ("WS2_32.dll", &["WSAStartup", "WSAGetLastError", "getaddrinfo", "freeaddrinfo", "socket", "connect", "send", "recv", "closesocket"]),
    ("SECUR32.dll", &["AcquireCredentialsHandleW", "InitializeSecurityContextW", "QueryContextAttributesW", "EncryptMessage", "DecryptMessage", "FreeContextBuffer", "DeleteSecurityContext", "FreeCredentialsHandle"]),
];

fn dll(f: &str) -> &'static str {
    DLLS.iter().find(|(_, functions)| functions.contains(&f)).unwrap_or(&DLLS[0]).0
}

// ---- macOS: Mach-O ----

/// Under it, 4 GB that no address may be in (`__PAGEZERO`): a pointer cut to 32 bits faults.
const MACHO_BASE: u64 = 0x1_0000_0000;
/// One file holding executables for several processors (`(cpu type, subtype, executable)`): a
/// "universal" Mac executable, in which macOS picks the one for its processor.
pub fn fat(slices: &[(u32, u32, &[u8])]) -> Vec<u8> {
    let mut out = vec![];
    for v in [0xCAFE_BABE, slices.len() as u32] {
        out.extend(v.to_be_bytes());
    }
    // Each one starts on a page of its processor (16 KB on arm64, 4 KB on x86-64).
    let page = |cpu: u32| if cpu == 0x0100_000C { MACHO_PAGE } else { PAGE };
    let mut at = 8 + 20 * slices.len() as u64;
    for &(cpu, subtype, exe) in slices {
        at = align(at, page(cpu));
        for v in [cpu, subtype, at as u32, exe.len() as u32, page(cpu).trailing_zeros()] {
            out.extend(v.to_be_bytes());
        }
        at += exe.len() as u64;
    }
    for &(cpu, _, exe) in slices {
        out.resize(align(out.len() as u64, page(cpu)) as usize, 0);
        out.extend(exe);
    }
    out
}

/// The Mac's pages on arm64 (4 times those of x86-64).
const MACHO_PAGE: u64 = 0x4000;
/// The header's commands (see `macho`).
const COMMANDS: u64 = 72 * 4 + 80 + 48 + 24 + 80 + 32 + 24 + 56 + 24 + 16;

fn macho_got(flags: RelocationFlags) -> bool {
    use object::macho::*;
    matches!(flags, RelocationFlags::MachO { r_type: ARM64_RELOC_GOT_LOAD_PAGE21 | ARM64_RELOC_GOT_LOAD_PAGEOFF12 | ARM64_RELOC_POINTER_TO_GOT, .. })
}

fn macho_got_intel(flags: RelocationFlags) -> bool {
    use object::macho::*;
    matches!(flags, RelocationFlags::MachO { r_type: X86_64_RELOC_GOT_LOAD | X86_64_RELOC_GOT, .. })
}

/// The executable of `main` and `runtime` for macOS arm64 (`arm`; else x86-64), named `name`. Four segments: none at
/// the bottom, `__TEXT` (headers, code, then the stubs that jump to `libSystem`'s functions,
/// constants), `__DATA` (the GOT, data, zeros), `__LINKEDIT` (what `dyld` writes at launch, then
/// the signature). An arm64 Mac starts a program anywhere in memory: each address the program
/// holds is listed ("rebase") for `dyld` to move it, and each function of `libSystem` it reads
/// ("bind") for `dyld` to write it.
pub fn macho(main: &[u8], runtime: &[u8], name: &str, arm: bool) -> Result<Vec<u8>, String> {
    let inputs = Inputs::read(main, runtime, true)?;
    let mut r = inputs.reach("_main", if arm { macho_got } else { macho_got_intel })?;
    // A stub jumps to a function of `libSystem`, reading its address in the GOT.
    let stub_size = if arm { 12 } else { 6 };
    // A page of the processor: what each segment is rounded to.
    let page_size = if arm { MACHO_PAGE } else { PAGE };
    let parts = inputs.sorted(&mut r)?;
    // ponytail: the runtime's code is one section, kept whole (76 KB); split it by symbol
    // (`subsections_via_symbols`) if the executables' size matters.
    for f in &r.imports {
        let t = Target::Import(f.clone(), false);
        if !r.got.contains(&t) {
            r.got.push(t); // its stub reads it
        }
    }
    let n = r.imports.len() as u64;
    let split = |part| r.kept.iter().position(|p| parts[p] >= part).unwrap_or(r.kept.len());
    let (constants, data) = (split(Part::Constants), split(Part::Data));

    let mut address = HashMap::new();
    let code_at = MACHO_BASE + 32 + COMMANDS;
    let stubs = align(inputs.place(&r.kept[..constants], code_at, |_, _| {}, &mut address)?, 4);
    let text_end = inputs.place(&r.kept[constants..data], stubs + stub_size * n, |_, _| {}, &mut address)?;
    let data_at = align(text_end, page_size);
    let got_at = data_at;
    let mut zeros = None;
    let end = inputs.place(
        &r.kept[data..],
        got_at + 8 * r.got.len() as u64,
        |part, at| {
            if part == Part::Zeros && zeros.is_none() {
                zeros = Some(*at)
            }
        },
        &mut address,
    )?;
    let zeros = zeros.unwrap_or(end);
    // Whole pages in the file; the zeros after them only in memory.
    let data_file = align(zeros - data_at, page_size);
    let data_size = align(end - data_at, page_size).max(data_file);
    let linkedit = data_at - MACHO_BASE + data_file;

    let offset = |a: u64| (a - MACHO_BASE) as usize;
    let mut out = vec![0u8; linkedit as usize];
    let slot = |t: &Target| got_at + 8 * r.got.iter().position(|g| g == t).unwrap() as u64;
    let stub = |f: &str| stubs + stub_size * r.imports.iter().position(|g| g == f).unwrap() as u64;
    let address_of = |t: &Target| match t {
        Target::At(p, off) => Ok(address[p].wrapping_add(*off)),
        Target::Nothing => Ok(0),
        Target::Import(f, _) => Ok(stub(f)),
    };
    // The addresses `dyld` writes: the program's own, moved with it (`None`), or a function's.
    let fixups: RefCell<Vec<(u64, Option<String>)>> = RefCell::new(vec![]);
    let apply = |out: &mut [u8], rel: &object::Relocation, t: &Target, place: u64, value: i64| {
        use object::macho::*;
        let RelocationFlags::MachO { r_type, .. } = rel.flags() else { return Err("not a Mach-O relocation".to_string()) };
        if rel.subtractor().is_some() {
            return put(out, value, rel.size());
        }
        let place = place as i64;
        match r_type {
            // `X86_64_RELOC_UNSIGNED` = `ARM64_RELOC_UNSIGNED` = 0.
            ARM64_RELOC_UNSIGNED if rel.size() == 64 => {
                let place = place as u64;
                if place < data_at {
                    return Err("an address outside the data".into());
                }
                match t {
                    Target::Import(f, _) if value == stub(f) as i64 => Ok(fixups.borrow_mut().push((place, Some(f.clone())))),
                    Target::Import(..) => Err("an address inside a function of the system".into()),
                    Target::Nothing => put(out, 0, 64),
                    Target::At(..) => {
                        fixups.borrow_mut().push((place, None));
                        put(out, value, 64)
                    }
                }
            }
            // x86-64: a distance from the end of the instruction (`object` put it in the addend).
            _ if !arm
                && matches!(r_type, X86_64_RELOC_SIGNED | X86_64_RELOC_SIGNED_1 | X86_64_RELOC_SIGNED_2 | X86_64_RELOC_SIGNED_4 | X86_64_RELOC_BRANCH) =>
            {
                put(out, value - place, 32)
            }
            _ if !arm && matches!(r_type, X86_64_RELOC_GOT_LOAD | X86_64_RELOC_GOT) => {
                put(out, value - address_of(t)? as i64 + slot(t) as i64 - place, 32)
            }
            _ if !arm => Err(format!("relocation of type {r_type}")),
            ARM64_RELOC_BRANCH26 => branch(out, value - place),
            ARM64_RELOC_PAGE21 => page(out, value, place as u64),
            ARM64_RELOC_PAGEOFF12 => in_page(out, value),
            ARM64_RELOC_GOT_LOAD_PAGE21 => page(out, slot(t) as i64 + rel.addend(), place as u64),
            ARM64_RELOC_GOT_LOAD_PAGEOFF12 => in_page(out, slot(t) as i64 + rel.addend()),
            ARM64_RELOC_POINTER_TO_GOT => put(out, slot(t) as i64 - place, 32),
            _ => Err(format!("relocation of type {r_type}")),
        }
    };
    inputs.write(&r.kept, &address, offset, &mut out, address_of, apply)?;
    let mut fixups = fixups.into_inner();

    for (k, t) in r.got.iter().enumerate() {
        let at = got_at + 8 * k as u64;
        match t {
            Target::Import(f, _) => fixups.push((at, Some(f.clone()))),
            Target::At(..) => {
                fixups.push((at, None));
                put(&mut out[offset(at)..], address_of(t)? as i64, 64)?;
            }
            Target::Nothing => {}
        }
    }
    for f in &r.imports {
        let (s, g) = (stub(f), slot(&Target::Import(f.clone(), false)) as i64);
        let o = offset(s);
        if !arm {
            // jmp [rip + slot]
            out[o..o + 2].copy_from_slice(&[0xFF, 0x25]);
            put(&mut out[o + 2..], g - (s as i64 + 6), 32)?;
            continue;
        }
        // adrp x16, slot@PAGE; ldr x16, [x16, slot@PAGEOFF]; br x16
        for (k, insn) in [0x9000_0010u32, 0xF940_0210, 0xD61F_0200].into_iter().enumerate() {
            out[o + 4 * k..o + 4 * k + 4].copy_from_slice(&insn.to_le_bytes());
        }
        page(&mut out[o..], g, s)?;
        in_page(&mut out[o + 4..], g)?;
    }

    // What `dyld` reads: each address to move (segment 2 = `__DATA`, its place in it), then each
    // function to write (from library 1 = `libSystem`).
    fixups.sort();
    let (mut rebase, mut bind) = (vec![0x11], vec![]); // pointers
    for (at, f) in &fixups {
        match f {
            None => {
                rebase.push(0x22);
                uleb(&mut rebase, at - data_at);
                rebase.push(0x51); // once
            }
            Some(f) => {
                bind.extend_from_slice(&[0x11, 0x40]);
                bind.extend_from_slice(f.as_bytes());
                bind.extend_from_slice(&[0, 0x51, 0x72]);
                uleb(&mut bind, at - data_at);
                bind.push(0x90);
            }
        }
    }
    for b in [&mut rebase, &mut bind] {
        b.push(0);
        b.resize(align(b.len() as u64, 8) as usize, 0);
    }
    let (rebase_at, bind_at) = (linkedit, linkedit + rebase.len() as u64);
    let strings = bind_at + bind.len() as u64; // no symbols: an empty table of names
    let signed = align(strings + 8, 16);
    let pages = signed.div_ceil(SIGNED_PAGE);
    let signature_size = 20 + 88 + name.len() as u64 + 1 + 32 * pages;
    let file_end = signed + signature_size;
    out.extend_from_slice(&rebase);
    out.extend_from_slice(&bind);
    out.resize(signed as usize, 0);

    let mut h = Vec::new();
    let le32 = |h: &mut Vec<u8>, v: u64| h.extend_from_slice(&(v as u32).to_le_bytes());
    let le64 = |h: &mut Vec<u8>, v: u64| h.extend_from_slice(&v.to_le_bytes());
    let named = |h: &mut Vec<u8>, name: &str, size: usize| {
        h.extend_from_slice(name.as_bytes());
        h.resize(h.len() + size - name.len(), 0);
    };
    // 64 bits, arm64 (or x86-64), an executable; no undefined names, linked by dyld, two-level
    // names, PIE.
    let (cpu, subtype) = if arm { (0x0100_000C, 0) } else { (0x0100_0007, 3) };
    for v in [0xFEED_FACF, cpu, subtype, 2, 12, COMMANDS, 0x20_0085, 0] {
        le32(&mut h, v);
    }
    let segment = |h: &mut Vec<u8>, name: &str, at: u64, size: u64, file: u64, file_size: u64, rights: u64, sections: u64| {
        le32(h, 0x19);
        le32(h, 72 + 80 * sections);
        named(h, name, 16);
        for v in [at, size, file, file_size] {
            le64(h, v);
        }
        for v in [rights, rights, sections, 0] {
            le32(h, v);
        }
    };
    segment(&mut h, "__PAGEZERO", 0, MACHO_BASE, 0, 0, 0, 0);
    segment(&mut h, "__TEXT", MACHO_BASE, data_at - MACHO_BASE, 0, data_at - MACHO_BASE, 5, 1); // read, run
    named(&mut h, "__text", 16);
    named(&mut h, "__TEXT", 16);
    le64(&mut h, code_at);
    le64(&mut h, stubs + stub_size * n - code_at);
    for v in [code_at - MACHO_BASE, 2, 0, 0, 0x8000_0400, 0, 0, 0] {
        le32(&mut h, v); // aligned on 4, only instructions
    }
    segment(&mut h, "__DATA", data_at, data_size, data_at - MACHO_BASE, data_file, 3, 0); // read, write
    segment(&mut h, "__LINKEDIT", data_at + data_size, align(file_end - linkedit, page_size), linkedit, file_end - linkedit, 1, 0);
    for v in [0x8000_0022, 48, rebase_at, rebase.len() as u64, bind_at, bind.len() as u64, 0, 0, 0, 0, 0, 0] {
        le32(&mut h, v); // LC_DYLD_INFO_ONLY: no weak, lazy nor exported names
    }
    for v in [2, 24, strings, 0, strings, 8] {
        le32(&mut h, v); // LC_SYMTAB
    }
    le32(&mut h, 0xB); // LC_DYSYMTAB, empty
    le32(&mut h, 80);
    h.resize(h.len() + 72, 0);
    for v in [0xE, 32, 12] {
        le32(&mut h, v); // LC_LOAD_DYLINKER
    }
    named(&mut h, "/usr/lib/dyld", 20);
    for v in [0x8000_0028, 24] {
        le32(&mut h, v); // LC_MAIN
    }
    le64(&mut h, address[&r.entry.0] + r.entry.1 - MACHO_BASE);
    le64(&mut h, 0);
    for v in [0xC, 56, 24, 2, 0x1_0000, 0x1_0000] {
        le32(&mut h, v); // LC_LOAD_DYLIB, version 1.0
    }
    named(&mut h, "/usr/lib/libSystem.B.dylib", 32);
    for v in [0x32, 24, 1, if arm { 0xB_0000 } else { 0xA_0F00 }, 0xE_0000, 0] {
        le32(&mut h, v); // LC_BUILD_VERSION: macOS 11 and later (10.15 on an Intel Mac)
    }
    for v in [0x1D, 16, signed, signature_size] {
        le32(&mut h, v); // LC_CODE_SIGNATURE
    }
    debug_assert_eq!(h.len() as u64, 32 + COMMANDS);
    out[..h.len()].copy_from_slice(&h);
    let s = signature(&out, name, data_at - MACHO_BASE);
    debug_assert_eq!(s.len() as u64, signature_size);
    out.extend_from_slice(&s);
    Ok(out)
}

/// `b` / `bl`: a distance in instructions, within 128 MB.
fn branch(out: &mut [u8], distance: i64) -> Result<(), String> {
    if distance % 4 != 0 || !(-(1 << 27)..1 << 27).contains(&distance) {
        return Err("branch too far".into());
    }
    let insn = u32::from_le_bytes(out[..4].try_into().unwrap()) & 0xFC00_0000 | (distance >> 2) as u32 & 0x03FF_FFFF;
    out[..4].copy_from_slice(&insn.to_le_bytes());
    Ok(())
}

/// `adrp`: the distance in pages of 4 KB from the instruction's to `target`'s.
fn page(out: &mut [u8], target: i64, place: u64) -> Result<(), String> {
    let pages = (target >> 12) - (place as i64 >> 12);
    if !(-(1 << 20)..1 << 20).contains(&pages) {
        return Err("address too far".into());
    }
    let insn = u32::from_le_bytes(out[..4].try_into().unwrap()) & 0x9F00_001F | ((pages & 3) as u32) << 29 | ((pages >> 2) as u32 & 0x7_FFFF) << 5;
    out[..4].copy_from_slice(&insn.to_le_bytes());
    Ok(())
}

/// After an `adrp`: `target`'s place in its page, counted in the size a load or store reads.
fn in_page(out: &mut [u8], target: i64) -> Result<(), String> {
    let insn = u32::from_le_bytes(out[..4].try_into().unwrap());
    let scale = scale(insn);
    let low = (target & 0xFFF) as u32;
    if low % (1 << scale) != 0 {
        return Err("misaligned address".into());
    }
    let insn = insn & 0xFFC0_03FF | (low >> scale) << 10;
    out[..4].copy_from_slice(&insn.to_le_bytes());
    Ok(())
}

/// How many bytes (as a power of 2) a load or store `insn` reads; 0 for an `add`.
fn scale(insn: u32) -> u32 {
    if insn & 0x3B00_0000 != 0x3900_0000 {
        return 0;
    }
    match insn >> 30 {
        0 if insn & 0x0480_0000 == 0x0480_0000 => 4, // 128 bits
        n => n,
    }
}

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            return out.push(byte);
        }
        out.push(byte | 0x80);
    }
}

/// The pages the signature checks one by one.
const SIGNED_PAGE: u64 = 0x1000;

/// The "ad-hoc" signature of `file` (everything before it), as Apple's linker makes it: a
/// "code directory" with the SHA-256 of each page, signed by no one; an arm64 Mac starts nothing
/// without it. `text_size`: where the code ends.
fn signature(file: &[u8], name: &str, text_size: u64) -> Vec<u8> {
    let pages = file.len().div_ceil(SIGNED_PAGE as usize) as u64;
    let identifier = name.len() as u64 + 1;
    let directory = 88 + identifier + 32 * pages;
    let mut s = Vec::new();
    let be32 = |s: &mut Vec<u8>, v: u64| s.extend_from_slice(&(v as u32).to_be_bytes());
    let be64 = |s: &mut Vec<u8>, v: u64| s.extend_from_slice(&v.to_be_bytes());
    // The blob holding the others (one: the code directory, at 20).
    for v in [0xFADE_0CC0, 20 + directory, 1, 0, 20] {
        be32(&mut s, v);
    }
    // Version 2.4; ad-hoc, by a linker; where the hashes and the name are; no special slots.
    for v in [0xFADE_0C02, directory, 0x2_0400, 0x2_0002, 88 + identifier, 88, 0, pages, file.len() as u64] {
        be32(&mut s, v);
    }
    s.extend_from_slice(&[32, 2, 0, 12]); // SHA-256 of 32 bytes, pages of 2^12
    for _ in 0..4 {
        be32(&mut s, 0);
    }
    for v in [0, 0, text_size, 1] {
        be64(&mut s, v); // the code: from 0 to `text_size`, of the main program
    }
    s.extend_from_slice(name.as_bytes());
    s.push(0);
    for page in file.chunks(SIGNED_PAGE as usize) {
        s.extend_from_slice(&sha256(page));
    }
    s
}

/// SHA-256 (FIPS 180-4).
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in m.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..64 {
            w[i] = if i < 16 {
                u32::from_be_bytes(block[4 * i..4 * i + 4].try_into().unwrap())
            } else {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1)
            };
        }
        let mut v = h;
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
    let mut out = [0; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn sha256() {
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(super::sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(super::sha256(&[b'a'; 56])), "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a");
        assert_eq!(hex(super::sha256(&[7; 1000])), "df1329c8b6c7cf3740bbe2f8bab34d253a8d9534a79dceea18177081fdf9f0e9");
    }
}
