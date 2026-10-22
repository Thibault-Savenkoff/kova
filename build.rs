// Builds the runtime glued to every Kova executable (runtime/kova.rs) for the system `kova` is
// built for, with the same Rust compiler: `kova` carries it inside (src/codegen.rs, `RUNTIME`).
// A system it doesn't know yet gets an empty file: `kova build` then compiles the C runtime.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let lib = PathBuf::from(var("OUT_DIR")).join("runtime.a");
    for f in ["runtime", "src/shared.rs", "src/bigint.rs"] {
        println!("cargo:rerun-if-changed={f}");
    }
    let known = var("CARGO_CFG_TARGET_OS") == "linux" && var("CARGO_CFG_TARGET_ARCH") == "x86_64";
    if !known {
        std::fs::write(&lib, []).unwrap();
        return;
    }
    let target = var("TARGET");
    let status = Command::new(var("RUSTC"))
        .args(["--edition", "2024", "--crate-type", "staticlib", "--crate-name", "kova_runtime", "--target", &target])
        .args(["-C", "panic=abort", "-C", "opt-level=2", "-C", "lto=fat", "-C", "codegen-units=1"])
        // Linux executables are static, not "position independent".
        .args(["-C", "relocation-model=static"])
        .arg("-o")
        .arg(&lib)
        .arg("runtime/kova.rs")
        .status()
        .expect("the Rust compiler");
    assert!(status.success(), "the runtime (runtime/kova.rs) does not compile");
}
