// Builds the runtime glued to every Kova executable (runtime/kova.rs), with the Rust compiler that
// builds `kova`, once for each system `kova build --target` knows: `kova` carries them inside
// (src/codegen.rs, `Target::runtime`). A system whose Rust library is not installed gets an empty
// file (`kova build --target` then says what to install). `runtime-host.a`: the system `kova`
// runs on when it is none of these (an Intel Mac), linked by `cc`; empty on the others (Linux on
// arm64), where `kova build` compiles the C runtime.
//
// Each archive keeps only the objects the runtime reaches: Rust's `compiler_builtins` brings
// hundreds (division on 128 bits, conversions…) with their debug information, ~3 MB per system.

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::process::Command;

use object::read::archive::ArchiveFile;
use object::{Object, ObjectSymbol};

fn main() {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let out = PathBuf::from(var("OUT_DIR"));
    for f in ["runtime", "src/shared.rs", "src/bigint.rs"] {
        println!("cargo:rerun-if-changed={f}");
    }
    let rustc = var("RUSTC");
    let installed = |triple: &str| {
        let dir = Command::new(&rustc).args(["--print", "target-libdir", "--target", triple]).output().ok();
        dir.is_some_and(|d| d.status.success() && Path::new(String::from_utf8_lossy(&d.stdout).trim()).is_dir())
    };
    // The same names as `Target::triples` in src/codegen.rs; Windows: Microsoft's or GNU's
    // Rust library, whichever is installed (the runtime asks nothing of either).
    let targets: [(&str, &[&str]); 3] = [
        ("linux", &["x86_64-unknown-linux-gnu"]),
        ("windows", &["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu"]),
        ("mac", &["aarch64-apple-darwin"]),
    ];
    for (name, triples) in targets {
        let lib = out.join(format!("runtime-{name}.a"));
        match triples.iter().find(|t| installed(t)) {
            Some(triple) => compile(&rustc, triple, &lib),
            None => {
                println!("cargo:warning=no runtime for `kova build --target {name}`: `rustup target add {}`, then build kova again", triples[0]);
                std::fs::write(&lib, []).unwrap();
            }
        }
    }
    let host = var("TARGET");
    let lib = out.join("runtime-host.a");
    if host == "x86_64-apple-darwin" {
        compile(&rustc, &host, &lib);
    } else {
        std::fs::write(&lib, []).unwrap();
    }
}

fn compile(rustc: &str, triple: &str, lib: &Path) {
    let status = Command::new(rustc)
        .args(["--edition", "2024", "--crate-type", "staticlib", "--crate-name", "kova_runtime", "--target", triple])
        .args(["-C", "panic=abort", "-C", "opt-level=2", "-C", "lto=fat", "-C", "codegen-units=1"])
        // Linux executables are static, not "position independent".
        .args(if triple.contains("linux") { &["-C", "relocation-model=static"][..] } else { &[] })
        .arg("-o")
        .arg(lib)
        .arg("runtime/kova.rs")
        .status()
        .expect("the Rust compiler");
    assert!(status.success(), "the runtime (runtime/kova.rs) does not compile for {triple}");
    let archive = std::fs::read(lib).unwrap();
    std::fs::write(lib, reached(&archive)).unwrap();
}

/// The archive of the objects reached from the runtime's own (`kova_runtime`), through the names
/// they use and others define, as a linker takes them from an archive.
fn reached(archive: &[u8]) -> Vec<u8> {
    let members: Vec<(String, &[u8])> = ArchiveFile::parse(archive)
        .unwrap()
        .members()
        .map(|m| {
            let m = m.unwrap();
            (String::from_utf8_lossy(m.name()).into_owned(), m.data(archive).unwrap())
        })
        .collect();
    let objects: Vec<Option<object::File>> = members.iter().map(|(_, data)| object::File::parse(*data).ok()).collect();
    let mut defined: HashMap<&str, usize> = HashMap::new();
    for (k, o) in objects.iter().enumerate() {
        for s in o.iter().flat_map(|o| o.symbols()) {
            if s.is_definition() && !s.is_local() {
                defined.entry(s.name().unwrap()).or_insert(k);
            }
        }
    }
    let mut kept: Vec<usize> = (0..members.len()).filter(|&k| objects[k].is_some() && members[k].0.contains("kova_runtime")).collect();
    assert!(!kept.is_empty(), "no runtime object in the archive");
    let mut i = 0;
    while i < kept.len() {
        for s in objects[kept[i]].as_ref().unwrap().symbols().filter(|s| s.is_undefined()) {
            if let Some(&k) = defined.get(s.name().unwrap())
                && !kept.contains(&k)
            {
                kept.push(k);
            }
        }
        i += 1;
    }
    kept.sort();
    // The common archive format: a header of 60 bytes before each object, on an even place.
    let mut out = b"!<arch>\n".to_vec();
    for (n, &k) in kept.iter().enumerate() {
        let data = members[k].1;
        out.extend_from_slice(format!("{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", format!("{n}.o/"), 0, 0, 0, 644, data.len()).as_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}
