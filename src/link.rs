// Kova's own linker (PLAN Part 4 "Kova without a C compiler", step 3): glues the program's
// object (Cranelift's) and the runtime (an archive of objects made by the Rust compiler, build.rs)
// into an executable, without the system's linker. It knows only what these two produce and
// refuses anything else loudly: it is not a general linker.
//
// 1. Every defined symbol of every object is noted (a strong one wins over a weak one).
// 2. From the entry point, the sections reached through relocations are kept: the functions a
//    program never calls are not in its executable.
// 3. The kept sections get their addresses: code and constants, then data, then zeros.
// 4. Each relocation is written: the address (or the distance to it) the code needs.
//
// Linux x86-64: a static ELF executable (no dynamic linker), loaded at 0x400000.

use std::collections::HashMap;

use object::read::archive::ArchiveFile;
use object::{Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationTarget, SectionIndex, SectionKind, SymbolSection};

/// A section of an object: (object number, section index).
type Place = (usize, SectionIndex);

/// Where a kept section goes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Code,
    Constants,
    Data,
    Zeros,
}

const BASE: u64 = 0x40_0000;
const PAGE: u64 = 0x1000;

/// The executable of `main` (the program's object) and `runtime` (an archive), for Linux x86-64.
pub fn elf(main: &[u8], runtime: &[u8]) -> Result<Vec<u8>, String> {
    let error = |e: object::Error| e.to_string();
    let mut objects = vec![object::File::parse(main).map_err(error)?];
    let archive = ArchiveFile::parse(runtime).map_err(error)?;
    for member in archive.members() {
        let member = member.map_err(error)?;
        objects.push(object::File::parse(member.data(runtime).map_err(error)?).map_err(error)?);
    }

    // 1. The defined symbols.
    let mut defined: HashMap<&str, (usize, object::SymbolIndex, bool)> = HashMap::new();
    for (k, o) in objects.iter().enumerate() {
        for s in o.symbols() {
            if s.is_local() || !matches!(s.section(), SymbolSection::Section(_)) {
                if s.section() == SymbolSection::Common {
                    return Err(format!("common symbol `{}`", s.name().unwrap_or("?")));
                }
                continue;
            }
            let name = s.name().map_err(error)?;
            match defined.get(name) {
                Some(&(_, _, false)) if !s.is_weak() => return Err(format!("`{name}` defined twice")),
                Some(&(_, _, weak)) if !weak || s.is_weak() => {}
                _ => {
                    defined.insert(name, (k, s.index(), s.is_weak()));
                }
            }
        }
    }
    let entry = *defined.get("_start").ok_or("no `_start`")?;

    // Where a relocation's target is: its section and its offset in it, or nothing (a weak
    // symbol defined nowhere: address 0).
    let target = |k: usize, t: RelocationTarget| -> Result<Option<(Place, u64)>, String> {
        let o = &objects[k];
        let s = match t {
            RelocationTarget::Section(i) => return Ok(Some(((k, i), 0))),
            RelocationTarget::Symbol(i) => o.symbol_by_index(i).map_err(error)?,
            _ => return Err("absolute relocation".into()),
        };
        let (k, s) = match s.section() {
            SymbolSection::Section(_) if s.is_local() => (k, s),
            _ => {
                let name = s.name().map_err(error)?;
                match defined.get(name) {
                    Some(&(k2, i, _)) => (k2, objects[k2].symbol_by_index(i).map_err(error)?),
                    None if s.is_weak() => return Ok(None),
                    None => return Err(format!("`{name}` is defined nowhere")),
                }
            }
        };
        match s.section() {
            SymbolSection::Section(i) => Ok(Some(((k, i), s.address()))),
            _ => Err(format!("`{}` is in no section", s.name().unwrap_or("?"))),
        }
    };

    // 2. The sections reached from the entry point, and the symbols reached through the GOT
    // (a table of addresses Cranelift's code reads them from).
    let start = objects[entry.0].symbol_by_index(entry.1).map_err(error)?;
    let first = (entry.0, start.section_index().ok_or("`_start` is in no section")?);
    let mut kept = vec![first];
    let mut seen: std::collections::HashSet<Place> = kept.iter().copied().collect();
    let mut got: Vec<(Place, u64)> = vec![];
    let mut got_slot: HashMap<(Place, u64), usize> = HashMap::new();
    let mut i = 0;
    while i < kept.len() {
        let (k, s) = kept[i];
        i += 1;
        for (_, r) in objects[k].section_by_index(s).map_err(error)?.relocations() {
            let Some(t) = target(k, r.target())? else { continue };
            if is_got(r.flags()) && !got_slot.contains_key(&t) {
                got_slot.insert(t, got.len());
                got.push(t);
            }
            if seen.insert(t.0) {
                kept.push(t.0);
            }
        }
    }

    // 3. The addresses: the headers, code and constants (with the GOT) in a first segment
    // (read and run); data then zeros in a second one (read and write). The second starts on a
    // new page, at the same place in the page as in the file (the system maps whole pages).
    let mut part_of = HashMap::new();
    for &(k, s) in &kept {
        let section = objects[k].section_by_index(s).map_err(error)?;
        let part = match section.kind() {
            SectionKind::Text => Part::Code,
            SectionKind::ReadOnlyData | SectionKind::ReadOnlyString | SectionKind::ReadOnlyDataWithRel => Part::Constants,
            SectionKind::Data => Part::Data,
            SectionKind::UninitializedData => Part::Zeros,
            kind => return Err(format!("section `{}` of an unknown kind ({kind:?})", section.name().unwrap_or("?"))),
        };
        part_of.insert((k, s), part);
    }
    kept.sort_by_key(|p| part_of[p]);
    let headers = 64 + 56 * PROGRAM_HEADERS;
    let mut address: HashMap<Place, u64> = HashMap::new();
    let mut at = BASE + headers;
    let mut place = |p: Place, at: &mut u64| -> Result<(), String> {
        let section = objects[p.0].section_by_index(p.1).map_err(error)?;
        *at = align(*at, section.align().max(1));
        address.insert(p, *at);
        *at += section.size();
        Ok(())
    };
    let (first_part, rest): (Vec<Place>, Vec<Place>) = kept.iter().partition(|p| part_of[p] < Part::Data);
    for &p in &first_part {
        place(p, &mut at)?;
    }
    let got_at = align(at, 8);
    let first_end = got_at + 8 * got.len() as u64;
    // An address of the second segment is `shift` further than its place in the file.
    let shift = align(first_end, PAGE) + first_end % PAGE - first_end;
    let data_at = first_end + shift;
    at = data_at;
    let mut zeros = None;
    for &p in &rest {
        if part_of[&p] == Part::Zeros && zeros.is_none() {
            zeros = Some(at);
        }
        place(p, &mut at)?;
    }
    let zeros = zeros.unwrap_or(at);
    let end = at;

    // 4. The bytes, with the relocations written.
    let offset = |a: u64| if a < data_at { a - BASE } else { a - shift - BASE };
    let mut out = vec![0u8; offset(zeros) as usize];
    let symbol_address = |t: Option<(Place, u64)>| t.map_or(0, |(p, off)| address[&p] + off);
    for (slot, &t) in got.iter().enumerate() {
        let o = offset(got_at) as usize + 8 * slot;
        out[o..o + 8].copy_from_slice(&symbol_address(Some(t)).to_le_bytes());
    }
    for &p in &kept {
        if part_of[&p] == Part::Zeros {
            continue;
        }
        let section = objects[p.0].section_by_index(p.1).map_err(error)?;
        let data = section.data().map_err(error)?;
        let o = offset(address[&p]) as usize;
        out[o..o + data.len()].copy_from_slice(data);
        for (r_offset, r) in section.relocations() {
            let t = target(p.0, r.target())?;
            let (s, a) = (symbol_address(t) as i64, r.addend());
            let place = (address[&p] + r_offset) as i64;
            let RelocationFlags::Elf { r_type } = r.flags() else { return Err("not an ELF relocation".into()) };
            let at = o + r_offset as usize;
            let value: i64 = match r_type {
                object::elf::R_X86_64_64 => {
                    out[at..at + 8].copy_from_slice(&(s + a).to_le_bytes());
                    continue;
                }
                object::elf::R_X86_64_PC64 => {
                    out[at..at + 8].copy_from_slice(&(s + a - place).to_le_bytes());
                    continue;
                }
                // Static: a call through the PLT goes straight to the function.
                object::elf::R_X86_64_PC32 | object::elf::R_X86_64_PLT32 => s + a - place,
                object::elf::R_X86_64_GOTPCREL | object::elf::R_X86_64_GOTPCRELX | object::elf::R_X86_64_REX_GOTPCRELX => {
                    let slot = got_slot[&t.ok_or("GOT entry of a missing symbol")?];
                    (got_at + 8 * slot as u64) as i64 + a - place
                }
                object::elf::R_X86_64_32 if (0..=u32::MAX as i64).contains(&(s + a)) => s + a,
                object::elf::R_X86_64_32S => s + a,
                _ => return Err(format!("relocation of type {r_type} in `{}`", section.name().unwrap_or("?"))),
            };
            let v = i32::try_from(value).or_else(|_| u32::try_from(value).map(|v| v as i32)).map_err(|_| "relocation too far")?;
            out[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    // The headers: the ELF header, then the segments.
    let start = address[&first] + start.address();
    let mut h = Vec::with_capacity(headers as usize);
    h.extend_from_slice(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0"); // 64 bits, little-endian, version 1
    h.extend_from_slice(&2u16.to_le_bytes()); // an executable
    h.extend_from_slice(&62u16.to_le_bytes()); // x86-64
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&start.to_le_bytes());
    h.extend_from_slice(&64u64.to_le_bytes()); // the segments' headers follow
    h.extend_from_slice(&0u64.to_le_bytes()); // no sections' headers
    h.extend_from_slice(&0u32.to_le_bytes());
    for v in [64u16, 56, PROGRAM_HEADERS as u16, 64, 0, 0] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    let segment = |h: &mut Vec<u8>, kind: u32, flags: u32, offset: u64, address: u64, file: u64, memory: u64| {
        for v in [kind, flags] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        for v in [offset, address, address, file, memory, PAGE] {
            h.extend_from_slice(&v.to_le_bytes());
        }
    };
    segment(&mut h, 1, 5, 0, BASE, first_end - BASE, first_end - BASE); // PT_LOAD, read + run
    segment(&mut h, 1, 6, offset(data_at), data_at, zeros - data_at, end - data_at); // PT_LOAD, read + write
    segment(&mut h, 0x6474e551, 6, 0, 0, 0, 0); // PT_GNU_STACK: the stack does not run
    out[..h.len()].copy_from_slice(&h);
    Ok(out)
}

const PROGRAM_HEADERS: u64 = 3;

fn align(a: u64, to: u64) -> u64 {
    a.div_ceil(to) * to
}

fn is_got(flags: RelocationFlags) -> bool {
    matches!(flags, RelocationFlags::Elf { r_type: object::elf::R_X86_64_GOTPCREL | object::elf::R_X86_64_GOTPCRELX | object::elf::R_X86_64_REX_GOTPCRELX })
}
