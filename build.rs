// Builds the runtime glued to every Kova executable (runtime/kova.rs), with the Rust compiler that
// builds `kova`, once for each system `kova build --target` knows: `kova` carries them inside
// (src/codegen.rs, `Target::runtime`). A system whose Rust library is not installed gets an empty
// file (`kova build --target` then says what to install).
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
    for f in ["runtime", "src/shared.rs", "src/bigint.rs", "src/http.rs", "src/json.rs", "src/regex.rs", "src/csv.rs", "src/crypto.rs", "src/openssl.rs", "src/schannel.rs", "src/network_framework.rs"] {
        println!("cargo:rerun-if-changed={f}");
    }
    let rustc = var("RUSTC");
    let libm = libm_source();
    let installed = |triple: &str| {
        let dir = Command::new(&rustc).args(["--print", "target-libdir", "--target", triple]).output().ok();
        dir.is_some_and(|d| d.status.success() && Path::new(String::from_utf8_lossy(&d.stdout).trim()).is_dir())
    };
    // The same names as `Target::triples` in src/codegen.rs; Windows: Microsoft's or GNU's
    // Rust library, whichever is installed (the runtime asks nothing of either).
    let targets: [(&str, &[&str]); 6] = [
        ("linux", &["x86_64-unknown-linux-gnu"]),
        ("windows", &["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu"]),
        ("windows-arm", &["aarch64-pc-windows-msvc", "aarch64-pc-windows-gnullvm"]),
        ("mac-arm", &["aarch64-apple-darwin"]),
        ("linux-arm", &["aarch64-unknown-linux-gnu"]),
        ("mac-intel", &["x86_64-apple-darwin"]),
    ];
    for (name, triples) in targets {
        let lib = out.join(format!("runtime-{name}.a"));
        match triples.iter().find(|t| installed(t)) {
            Some(triple) => compile(&rustc, triple, &lib, &libm, &out),
            None => {
                println!("cargo:warning=no runtime for `kova build --target {name}`: `rustup target add {}`, then build kova again", triples[0]);
                std::fs::write(&lib, []).unwrap();
            }
        }
    }
}

/// `libm`'s sources, which cargo downloaded for Cargo.toml before running build.rs.
fn libm_source() -> PathBuf {
    let home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| PathBuf::from(h).join(".cargo"))
    });
    let registry = home.expect("no home directory").join("registry").join("src");
    let found = std::fs::read_dir(&registry).into_iter().flatten().flatten().map(|d| d.path().join("libm-0.2.16").join("src").join("lib.rs")).find(|p| p.is_file());
    found.unwrap_or_else(|| panic!("`libm` 0.2.16 not found in {}", registry.display()))
}

fn compile(rustc: &str, triple: &str, lib: &Path, libm: &Path, out: &Path) {
    // Linux executables are static, not "position independent".
    let model: &[&str] = if triple.contains("linux") { &["-C", "relocation-model=static"] } else { &[] };
    let codegen = ["-C", "panic=abort", "-C", "opt-level=2", "-C", "codegen-units=1"];
    let rlib = out.join(format!("libm-{triple}.rlib"));
    let status = Command::new(rustc)
        .args(["--edition", "2021", "--crate-type", "rlib", "--crate-name", "libm", "--cap-lints", "allow", "--target", triple])
        .args(codegen)
        .args(model)
        .arg("-o")
        .arg(&rlib)
        .arg(libm)
        .status()
        .expect("the Rust compiler");
    assert!(status.success(), "`libm` does not compile for {triple}");
    let status = Command::new(rustc)
        .args(["--edition", "2024", "--crate-type", "staticlib", "--crate-name", "kova_runtime", "--target", triple])
        .args(codegen)
        .args(["-C", "lto=fat"])
        .args(model)
        .arg("--extern")
        .arg(format!("libm={}", rlib.display()))
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
