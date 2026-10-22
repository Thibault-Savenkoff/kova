// Builds the runtime glued to every Kova executable (runtime/kova.rs), with the Rust compiler that
// builds `kova`, once for each system `kova build --target` knows: `kova` carries them inside
// (src/codegen.rs, `Target::runtime`). A system whose Rust library is not installed gets an empty
// file (`kova build --target` then says what to install). `runtime-host.a`: the system `kova`
// runs on when it is none of these (an Intel Mac), linked by `cc`; empty on the others (Linux on
// arm64), where `kova build` compiles the C runtime.

use std::path::{Path, PathBuf};
use std::process::Command;

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
}
