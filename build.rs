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
    for f in ["runtime", "vendor", "src/shared.rs", "src/bigint.rs", "src/http.rs", "src/json.rs", "src/deflate.rs", "src/regex.rs", "src/csv.rs", "src/crypto.rs", "src/openssl.rs", "src/schannel.rs", "src/network_framework.rs"] {
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
    // Every runtime is compiled on its own thread: they share nothing (each has its own files,
    // named after its target and variant), and a build used to wait for them one by one.
    let mut jobs: Vec<(String, PathBuf, &str)> = Vec::new();
    let empty = |lib: &Path| std::fs::write(lib, []).unwrap();
    for (name, triples) in targets {
        let lib = out.join(format!("runtime-{name}.a"));
        match triples.iter().find(|t| installed(t)) {
            Some(triple) => jobs.push((triple.to_string(), lib, "")),
            None => {
                println!("cargo:warning=no runtime for `kova build --target {name}`: `rustup target add {}`, then build kova again", triples[0]);
                empty(&lib);
            }
        }
    }
    // Android: Linux's runtime, loaded at any address. Its system calls are Linux's and it brings
    // its own Rust library, so Rust's `linux` targets are all it needs.
    for (name, triple) in [("android-arm", "aarch64-unknown-linux-gnu"), ("android-x86", "x86_64-unknown-linux-gnu")] {
        let lib = out.join(format!("runtime-{name}.a"));
        if installed(triple) {
            jobs.push((triple.to_string(), lib, "android"));
        } else {
            empty(&lib);
        }
    }
    // iOS's simulator: the Mac arm64 runtime again, with its own windows (runtime/uikit.rs).
    let ios = out.join("runtime-ios-sim.a");
    if installed("aarch64-apple-darwin") {
        jobs.push(("aarch64-apple-darwin".into(), ios, "ios"));
    } else {
        empty(&ios);
    }
    let web = out.join("runtime-web.wasm");
    if installed("wasm32-unknown-unknown") {
        jobs.push(("wasm32-unknown-unknown".into(), web, ""));
    } else {
        println!("cargo:warning=no runtime for `kova build --target web`: `rustup target add wasm32-unknown-unknown`, then build kova again");
        empty(&web);
    }
    std::thread::scope(|scope| {
        for (triple, lib, variant) in &jobs {
            let (rustc, libm, out) = (&rustc, &libm, &out);
            scope.spawn(move || compile(rustc, triple, lib, libm, out, variant));
        }
    });
}

/// `libm`'s sources, which cargo downloaded for Cargo.toml before running build.rs.
fn libm_source() -> PathBuf {
    let home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| PathBuf::from(h).join(".cargo"))
    });
    let registry = home.expect("no home directory").join("registry").join("src");
    found_crate("libm-0.2.16", &registry)
}

fn found_crate(name: &str, registry: &Path) -> PathBuf {
    let found = std::fs::read_dir(registry).into_iter().flatten().flatten().map(|d| d.path().join(name).join("src").join("lib.rs")).find(|p| p.is_file());
    found.unwrap_or_else(|| panic!("`{name}` not found in {}", registry.display()))
}

fn registry_dir() -> PathBuf {
    let home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| PathBuf::from(h).join(".cargo"))
    });
    home.expect("no home directory").join("registry").join("src")
}

/// `variant`: "" the plain runtime, "android" (Linux's, loaded anywhere), "ios" (the Mac arm64 runtime
/// with UIKit in place of AppKit).
fn compile(rustc: &str, triple: &str, lib: &Path, libm: &Path, out: &Path, variant: &str) {
    // Linux executables are static, not "position independent"; Android's libraries are.
    let model: &[&str] = match (variant, triple.contains("linux")) {
        ("android", _) => &["-C", "relocation-model=pic", "--cfg", "android"],
        ("ios", _) => &["--cfg", "ios"],
        (_, true) => &["-C", "relocation-model=static"],
        _ => &[],
    };
    let codegen = ["-C", "panic=abort", "-C", "opt-level=2", "-C", "codegen-units=1"];
    let rlib = out.join(format!("libm-{triple}-{variant}.rlib"));
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
    // For a page, a whole module: src/wasm.rs adds the program to it.
    let web = triple.starts_with("wasm");
    // The decoders (no_std, in cargo's registry): each one is compiled as `libm` is, after the ones
    // it needs. (crate, folder, edition, features, crates it needs)
    let decoders: [(&str, &str, &str, &[&str], &[&str]); 10] = [
        ("nanomp3_core", "nanomp3-core-0.2.0", "2021", &["layer12"], &[]),
        ("nanomp3", "nanomp3-0.2.0", "2021", &["alloc", "layer12"], &["nanomp3_core"]),
        ("bytemuck", "bytemuck-1.25.2", "2018", &[], &[]),
        ("safe_arch", "safe_arch-1.2.0", "2024", &["bytemuck"], &["bytemuck"]),
        ("wide", "wide-1.7.1", "2024", &[], &["bytemuck", "safe_arch"]),
        ("nanoaudio_core", "nanoaudio-core-0.1.1", "2024", &[], &[]),
        ("nanoaudio_flac", "nanoaudio-flac-0.1.2", "2024", &[], &["nanoaudio_core", "wide"]),
        ("opuscule", "opuscule-0.2.2", "2024", &["fixed-point"], &[]),
        // Not from the registry: the copy made no_std in `vendor/` (see its README).
        ("rusty_aac", "vendor/rusty_aac", "2021", &[], &["libm"]),
        ("lewton", "vendor/lewton", "2015", &[], &["libm"]),
    ];
    let registry = registry_dir();
    let mut externs: Vec<(String, PathBuf)> = vec![("libm".to_string(), rlib.clone())];
    for (name, folder, edition, features, needs) in decoders {
        let path = out.join(format!("lib{name}-{triple}-{variant}.rlib"));
        let mut command = Command::new(rustc);
        command.args(["--edition", edition, "--crate-type", "rlib", "--crate-name", name, "--cap-lints", "allow", "--target", triple]).args(codegen).args(model);
        for f in features {
            command.args(["--cfg", &format!("feature=\"{f}\"")]);
        }
        // `safe_arch` is only for the x86 processors.
        let x86 = triple.starts_with("x86_64") || triple.starts_with("i686");
        if name == "safe_arch" && !x86 {
            continue;
        }
        command.arg("-L").arg(format!("dependency={}", out.display()));
        for need in needs.iter().filter(|n| **n != "safe_arch" || x86) {
            let found = externs.iter().find(|(n, _)| n == need).expect("built before");
            command.arg("--extern").arg(format!("{}={}", found.0, found.1.display()));
        }
        let status = command.arg("-o").arg(&path).arg(if folder.starts_with("vendor/") { PathBuf::from(folder).join("src").join("lib.rs") } else { found_crate(folder, &registry) }).status().expect("the Rust compiler");
        assert!(status.success(), "`{name}` does not compile for {triple}");
        externs.push((name.to_string(), path));
    }
    let mp3 = externs.iter().find(|(n, _)| n == "nanomp3").unwrap().1.clone();
    let flac = externs.iter().find(|(n, _)| n == "nanoaudio_flac").unwrap().1.clone();
    let status = Command::new(rustc)
        .args(["--edition", "2024", "--crate-type", if web { "cdylib" } else { "staticlib" }, "--crate-name", "kova_runtime", "--target", triple])
        .args(codegen)
        .args(["-C", "lto=fat"])
        .args(model)
        .arg("--extern")
        .arg(format!("libm={}", rlib.display()))
        .arg("--extern")
        .arg(format!("lewton={}", externs.iter().find(|(n, _)| n == "lewton").unwrap().1.display()))
        .arg("--extern")
        .arg(format!("rusty_aac={}", externs.iter().find(|(n, _)| n == "rusty_aac").unwrap().1.display()))
        .arg("--extern")
        .arg(format!("nanomp3={}", mp3.display()))
        .arg("--extern")
        .arg(format!("opuscule={}", externs.iter().find(|(n, _)| n == "opuscule").unwrap().1.display()))
        .arg("--extern")
        .arg(format!("nanoaudio_flac={}", flac.display()))
        .arg("--extern")
        .arg(format!("nanoaudio_core={}", externs.iter().find(|(n, _)| n == "nanoaudio_core").unwrap().1.display()))
        .arg("-L")
        .arg(format!("dependency={}", out.display()))
        .arg("-o")
        .arg(lib)
        .arg("runtime/kova.rs")
        .status()
        .expect("the Rust compiler");
    assert!(status.success(), "the runtime (runtime/kova.rs) does not compile for {triple}");
    if web {
        return;
    }
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
