// Native backend (`kova build`): KIR → machine code with Cranelift → `main.o`,
// with the runtime's texts as data (`runtime_texts`), then Kova's own linker (src/link.rs) glues
// it and the runtime (in Rust, `Target::runtime`) into an executable: no C compiler anywhere.
//
// Each KIR local becomes a Cranelift `Variable`; each KIR block, a Cranelift block.

use std::collections::HashMap;
use std::path::Path;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{AbiParam, Block, FuncRef, GlobalValue, InstBuilder, MemFlagsData, Signature, TrapCode, Value};
use cranelift_codegen::isa::{CallConv, TargetFrontendConfig};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::kir::*;
use crate::locale::{self, t};

/// A system `kova build` makes executables for with its own linker, from any machine (PLAN Part 4
/// "Cross-compilation"): `--target linux`, `windows`, `mac`, `linux-arm`, `mac-intel`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    /// Linux on x86-64.
    Linux,
    /// Windows on x86-64.
    Windows,
    /// Every Mac: one file holding the arm64 and the x86-64 executables, macOS runs the one it needs.
    Mac,
    /// Linux on arm64 (a Raspberry Pi, servers).
    LinuxArm,
    /// Windows on arm64 (Surface and other Snapdragon PCs).
    WindowsArm,
    /// macOS on arm64 (Apple's own processors).
    MacArm,
    /// macOS on x86-64 (the Macs before 2020).
    MacIntel,
}

impl Target {
    pub const NAMES: [(&str, Target); 7] = [
        ("linux", Target::Linux),
        ("windows", Target::Windows),
        ("mac", Target::Mac),
        ("linux-arm", Target::LinuxArm),
        ("windows-arm", Target::WindowsArm),
        ("mac-arm", Target::MacArm),
        ("mac-intel", Target::MacIntel),
    ];

    pub fn named(name: &str) -> Option<Target> {
        Target::NAMES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
    }

    pub fn name(self) -> &'static str {
        Target::NAMES.iter().find(|(_, t)| *t == self).unwrap().0
    }

    /// This very machine: on a Mac, only its own half (`kova run` starts the program at once).
    pub fn native() -> Option<Target> {
        match Target::host()? {
            Target::Mac if cfg!(target_arch = "aarch64") => Some(Target::MacArm),
            Target::Mac => Some(Target::MacIntel),
            t => Some(t),
        }
    }

    /// The machine `kova` runs on, if it is one of them; on a Mac, every Mac (user, 2026-10-23: the
    /// executable can be given to anyone).
    pub fn host() -> Option<Target> {
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Target::Linux)
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Target::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Target::Mac)
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            Some(Target::LinuxArm)
        } else if cfg!(all(windows, target_arch = "aarch64")) {
            Some(Target::WindowsArm)
        } else {
            None
        }
    }

    /// Its name in `kova build`'s report.
    pub fn title(self) -> &'static str {
        match self {
            Target::Linux => "Linux",
            Target::Windows => "Windows",
            Target::Mac => "Mac",
            Target::LinuxArm => "Linux arm",
            Target::WindowsArm => "Windows arm",
            Target::MacArm => "Mac arm",
            Target::MacIntel => "Mac Intel",
        }
    }

    /// The end of its executables' names.
    pub fn suffix(self) -> &'static str {
        if matches!(self, Target::Windows | Target::WindowsArm) { ".exe" } else { "" }
    }

    /// What Cranelift and Rust call it (the first one: the one to install, see build.rs).
    fn triple(self) -> &'static str {
        match self {
            Target::Linux => "x86_64-unknown-linux-gnu",
            Target::Windows => "x86_64-pc-windows-msvc",
            Target::Mac | Target::MacArm => "aarch64-apple-darwin",
            Target::LinuxArm => "aarch64-unknown-linux-gnu",
            Target::WindowsArm => "aarch64-pc-windows-msvc",
            Target::MacIntel => "x86_64-apple-darwin",
        }
    }

    /// Its runtime, built by build.rs (empty if Rust's library for it was not installed).
    fn runtime(self) -> &'static [u8] {
        match self {
            Target::Linux => include_bytes!(concat!(env!("OUT_DIR"), "/runtime-linux.a")),
            Target::Windows => include_bytes!(concat!(env!("OUT_DIR"), "/runtime-windows.a")),
            Target::Mac | Target::MacArm => include_bytes!(concat!(env!("OUT_DIR"), "/runtime-mac-arm.a")),
            Target::LinuxArm => include_bytes!(concat!(env!("OUT_DIR"), "/runtime-linux-arm.a")),
            Target::WindowsArm => include_bytes!(concat!(env!("OUT_DIR"), "/runtime-windows-arm.a")),
            Target::MacIntel => include_bytes!(concat!(env!("OUT_DIR"), "/runtime-mac-intel.a")),
        }
    }
}

/// `lang` = the language forced by the `header`; otherwise the executable picks it at launch.
/// `target`: the system to make it for (the machine's own if none is given).
pub fn build(p: &Program, files: &[String], output: &str, lang: Option<usize>, target: Option<Target>) -> Result<(), String> {
    let Some(target) = target.or(Target::host()) else { return Err(t!("unknown_system")) };
    let name = Path::new(output).file_name().map_or("program".into(), |n| n.to_string_lossy().into_owned());
    write_executable(output, &executable(p, files, &name, lang, target)?)
}

fn executable(p: &Program, files: &[String], name: &str, lang: Option<usize>, target: Target) -> Result<Vec<u8>, String> {
    if target == Target::Mac {
        let intel = executable(p, files, name, lang, Target::MacIntel)?;
        return Ok(crate::link::fat(&[(0x0100_0007, 3, &intel), (0x0100_000C, 0, &executable(p, files, name, lang, Target::MacArm)?)]));
    }
    let runtime = target.runtime();
    if runtime.is_empty() {
        return Err(t!("no_runtime", target = target.name(), triple = target.triple()));
    }
    // Windows on arm64: Cranelift cannot write its addresses in Microsoft's object format, so the
    // program's object is Linux's; arm64 Windows and Linux call functions the same way.
    let windows_arm = target == Target::WindowsArm;
    let triple = if windows_arm { "aarch64-unknown-linux-gnu" } else { target.triple() };
    let object = make_object(p, files, lang, triple.parse().unwrap(), windows_arm).map_err(|e| t!("codegen_failed", error = e))?;
    // Kova's own linker: no `cc`, nor Microsoft's or Apple's linker (src/link.rs).
    let exe = match target {
        Target::Linux => crate::link::elf(&object, runtime, false),
        Target::LinuxArm => crate::link::elf(&object, runtime, true),
        Target::Windows => crate::link::pe(&object, runtime, false),
        Target::WindowsArm => crate::link::pe(&object, runtime, true),
        Target::Mac | Target::MacArm => crate::link::macho(&object, runtime, name, true),
        Target::MacIntel => crate::link::macho(&object, runtime, name, false),
    };
    exe.map_err(|e| t!("codegen_failed", error = e))
}

/// The executable, which may be run.
fn write_executable(output: &str, exe: &[u8]) -> Result<(), String> {
    let _ = std::fs::remove_file(output); // a running one cannot be written over
    std::fs::write(output, exe).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The C runtime functions called by the generated code.
/// How many `StrOp`s the runtime has (`str_op` numbers them).
const STR_OPS: usize = 79;

struct Runtime {
    print: FuncId,
    fault: FuncId,
    write_text: FuncId,
    write_label: FuncId,
    write_value: FuncId,
    fail: FuncId,
    exit: FuncId,
    /// `kova_str_new`, `kova_str_free`, then one per `StrOp` (in `str_op` order).
    str_new: FuncId,
    str_free: FuncId,
    /// `kova_show_int`… (in `Show` order): given by address to `kova_print` and the others, so
    /// that only the ones a program uses are linked (no Float formatting to print a String).
    shows: [FuncId; 6],
    str_ops: [FuncId; STR_OPS],
    lists: [FuncId; 10],
    math: FuncId,
}

/// The runtime functions for Lists (`kova_list_*` in `runtime/kova.rs`):
/// (name, number of arguments, number of results), in the order of the `L_*` numbers.
const LIST_FUNCTIONS: [(&str, usize, usize); 10] = [
    ("kova_list_new", 1, 1),
    ("kova_list_length", 1, 1),
    ("kova_list_check", 4, 0),
    ("kova_list_load", 3, 1),
    ("kova_list_store", 4, 0),
    ("kova_list_grow", 1, 1),
    ("kova_list_remove", 2, 0),
    ("kova_list_free", 1, 0),
    ("kova_list_index", 1, 1),
    ("kova_list_set_index", 2, 0),
];
const L_NEW: usize = 0;
const L_LENGTH: usize = 1;
const L_CHECK: usize = 2;
const L_LOAD: usize = 3;
const L_STORE: usize = 4;
const L_GROW: usize = 5;
const L_REMOVE: usize = 6;
const L_FREE: usize = 7;
const L_INDEX: usize = 8;
const L_SET_INDEX: usize = 9;

/// The runtime function of a String operation, and its number of arguments.
fn str_op(op: StrOp) -> (usize, &'static str, usize) {
    match op {
        StrOp::Concat => (0, "kova_str_concat", 2),
        StrOp::Eq => (1, "kova_str_eq", 2),
        StrOp::Length => (2, "kova_str_length", 1),
        StrOp::Clone => (3, "kova_str_clone", 1),
        // The value and its `Show`.
        StrOp::Format(_) => (4, "kova_str_format", 2),
        StrOp::IsInt => (5, "kova_str_is_int", 1),
        StrOp::ToInt => (6, "kova_str_to_int", 1),
        StrOp::ReadFile => (7, "kova_read_file", 1),
        StrOp::ReadLine => (8, "kova_read_line", 0),
        StrOp::Less => (9, "kova_str_less", 2),
        StrOp::Trim => (10, "kova_str_trim", 1),
        StrOp::Contains => (11, "kova_str_contains", 2),
        StrOp::StartsWith => (12, "kova_str_starts_with", 2),
        StrOp::EndsWith => (13, "kova_str_ends_with", 2),
        StrOp::Replace => (14, "kova_str_replace", 3),
        StrOp::Split => (15, "kova_str_split", 2),
        StrOp::Words => (16, "kova_str_words", 1),
        StrOp::Chars => (17, "kova_str_chars", 1),
        StrOp::Args => (18, "kova_args", 0),
        StrOp::BigAdd => (19, "kova_big_add", 2),
        StrOp::BigSub => (20, "kova_big_sub", 2),
        StrOp::BigMul => (21, "kova_big_mul", 2),
        StrOp::BigDiv => (22, "kova_big_div", 2),
        StrOp::BigRem => (23, "kova_big_rem", 2),
        StrOp::BigLess => (24, "kova_big_less", 2),
        StrOp::Hash => (25, "kova_str_hash", 1),
        StrOp::WriteFile => (26, "kova_write_file", 2),
        StrOp::AppendFile => (27, "kova_append_file", 2),
        StrOp::MakeDir => (28, "kova_make_dir", 1),
        StrOp::RemoveFile => (29, "kova_remove_file", 1),
        StrOp::ListDir => (30, "kova_list_dir", 1),
        StrOp::Exists => (31, "kova_exists", 1),
        StrOp::ToUpper => (32, "kova_str_to_upper", 1),
        StrOp::ToLower => (33, "kova_str_to_lower", 1),
        StrOp::Find => (34, "kova_str_find", 2),
        StrOp::Slice => (35, "kova_str_slice", 3),
        StrOp::Repeat => (36, "kova_str_repeat", 2),
        StrOp::Join => (37, "kova_str_join", 2),
        StrOp::IsFloat => (38, "kova_str_is_float", 1),
        StrOp::ToFloat => (39, "kova_str_to_float", 1),
        StrOp::Now => (40, "kova_now", 0),
        StrOp::Sleep => (41, "kova_sleep", 1),
        StrOp::Random => (42, "kova_random", 2),
        StrOp::LocalNow => (43, "kova_local_now", 0),
        StrOp::ToDate => (44, "kova_str_to_date", 1),
        StrOp::ToDateTime => (45, "kova_str_to_datetime", 1),
        StrOp::Web => (46, "kova_web", 6),
        StrOp::JsonParse => (47, "kova_json_parse", 1),
        StrOp::JsonQuote => (48, "kova_json_quote", 1),
        StrOp::Listen => (49, "kova_listen", 4),
        StrOp::Accept => (50, "kova_accept", 1),
        StrOp::Reply => (51, "kova_reply", 5),
        StrOp::UrlPath => (52, "kova_url_path", 1),
        StrOp::UrlQuery => (53, "kova_url_query", 2),
        StrOp::StatusText => (54, "kova_status_text", 1),
        StrOp::FileToServe => (55, "kova_file_to_serve", 1),
        StrOp::Workers => (56, "kova_workers", 3),
        StrOp::Parallel => (57, "kova_parallel", 4),
        StrOp::Next => (58, "kova_next", 1),
        StrOp::CopyFile => (59, "kova_copy_file", 2),
        StrOp::MoveFile => (60, "kova_move_file", 2),
        StrOp::FileSize => (61, "kova_file_size", 1),
        StrOp::Modified => (62, "kova_modified", 1),
        StrOp::IsFolder => (63, "kova_is_folder", 1),
        StrOp::Matching => (64, "kova_matching", 1),
        StrOp::Env => (65, "kova_env", 1),
        StrOp::Run => (66, "kova_run", 2),
        StrOp::Wrap => (67, "kova_str_wrap", 2),
        StrOp::Matches => (68, "kova_str_matches", 2),
        StrOp::FindAll => (69, "kova_str_find_all", 2),
        StrOp::ReplaceAll => (70, "kova_str_replace_all", 3),
        StrOp::Groups => (71, "kova_str_groups", 2),
        StrOp::CsvRead => (72, "kova_csv_read", 2),
        StrOp::CsvWrite => (73, "kova_csv_write", 3),
        StrOp::Mean => (74, "kova_list_mean", 2),
        StrOp::Median => (75, "kova_list_median", 2),
        StrOp::SecureRandom => (76, "kova_secure_random", 2),
        StrOp::SecureToken => (77, "kova_secure_token", 1),
        StrOp::Uuid => (78, "kova_uuid", 0),
    }
}

fn make_object(p: &Program, files: &[String], lang: Option<usize>, triple: target_lexicon::Triple, windows_arm: bool) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed")?;
    // Code that runs at any address, as macOS wants it; not for Windows on arm64, where Cranelift
    // writes addresses instead (Windows moves them with the program: `link::pe`'s `.reloc`), and
    // touches each page of a big stack frame, as Windows wants.
    flags.set("is_pic", if windows_arm { "false" } else { "true" })?;
    if windows_arm {
        flags.set("enable_probestack", "true")?;
        flags.set("probestack_strategy", "inline")?;
    }
    // A function returning many values (big tuple): Cranelift stores them in memory.
    flags.set("enable_multi_ret_implicit_sret", "true")?;
    let isa = cranelift_codegen::isa::lookup(triple)?.finish(settings::Flags::new(flags))?;
    // One section per function, so that the linker drops the unused ones (`--gc-sections`).
    let mut builder = ObjectBuilder::new(isa, "kova", default_libcall_names())?;
    builder.per_function_section(true);
    let mut module = ObjectModule::new(builder);

    let call_conv = module.isa().default_call_conv();
    let config = module.isa().frontend_config();
    let signature = |params: &[Scalar], returns: &[Scalar]| {
        let mut s = Signature::new(call_conv);
        s.params.extend(params.iter().map(|&t| AbiParam::new(machine_type(t))));
        s.returns.extend(returns.iter().map(|&t| AbiParam::new(machine_type(t))));
        s
    };
    let ints = [Scalar::Int; 6];
    let rt = Runtime {
        print: module.declare_function("kova_print", Linkage::Import, &signature(&ints[..3], &[]))?,
        fault: module.declare_function("kova_fault", Linkage::Import, &signature(&ints[..3], &[]))?,
        write_text: module.declare_function("kova_write_text", Linkage::Import, &signature(&ints[..2], &[]))?,
        write_label: module.declare_function("kova_write_label", Linkage::Import, &signature(&ints[..1], &[]))?,
        write_value: module.declare_function("kova_write_value", Linkage::Import, &signature(&ints[..2], &[]))?,
        fail: module.declare_function("kova_fail", Linkage::Import, &signature(&[], &[]))?,
        exit: module.declare_function("kova_exit", Linkage::Import, &signature(&ints[..1], &[]))?,
        math: module.declare_function("kova_math", Linkage::Import, &signature(&[Scalar::Int, Scalar::Float, Scalar::Float], &[Scalar::Float]))?,
        str_new: module.declare_function("kova_str_new", Linkage::Import, &signature(&ints[..2], &ints[..1]))?,
        str_free: module.declare_function("kova_str_free", Linkage::Import, &signature(&ints[..1], &[]))?,
        shows: {
            let mut ids = vec![];
            for name in ["int", "bool", "float", "char", "string", "site"] {
                ids.push(module.declare_function(&format!("kova_show_{name}"), Linkage::Import, &signature(&ints[..2], &[]))?);
            }
            ids.try_into().unwrap()
        },
        str_ops: {
            let mut ids = vec![];
            for op in [StrOp::Concat, StrOp::Eq, StrOp::Length, StrOp::Clone, StrOp::Format(Show::Int), StrOp::IsInt, StrOp::ToInt, StrOp::ReadFile, StrOp::ReadLine, StrOp::Less, StrOp::Trim, StrOp::Contains, StrOp::StartsWith, StrOp::EndsWith, StrOp::Replace, StrOp::Split, StrOp::Words, StrOp::Chars, StrOp::Args, StrOp::BigAdd, StrOp::BigSub, StrOp::BigMul, StrOp::BigDiv, StrOp::BigRem, StrOp::BigLess, StrOp::Hash, StrOp::WriteFile, StrOp::AppendFile, StrOp::MakeDir, StrOp::RemoveFile, StrOp::ListDir, StrOp::Exists, StrOp::ToUpper, StrOp::ToLower, StrOp::Find, StrOp::Slice, StrOp::Repeat, StrOp::Join, StrOp::IsFloat, StrOp::ToFloat, StrOp::Now, StrOp::Sleep, StrOp::Random, StrOp::LocalNow, StrOp::ToDate, StrOp::ToDateTime, StrOp::Web, StrOp::JsonParse, StrOp::JsonQuote, StrOp::Listen, StrOp::Accept, StrOp::Reply, StrOp::UrlPath, StrOp::UrlQuery, StrOp::StatusText, StrOp::FileToServe, StrOp::Workers, StrOp::Parallel, StrOp::Next, StrOp::CopyFile, StrOp::MoveFile, StrOp::FileSize, StrOp::Modified, StrOp::IsFolder, StrOp::Matching, StrOp::Env, StrOp::Run, StrOp::Wrap, StrOp::Matches, StrOp::FindAll, StrOp::ReplaceAll, StrOp::Groups, StrOp::CsvRead, StrOp::CsvWrite, StrOp::Mean, StrOp::Median, StrOp::SecureRandom, StrOp::SecureToken, StrOp::Uuid] {
                let (_, name, n) = str_op(op);
                ids.push(module.declare_function(name, Linkage::Import, &signature(&ints[..n], &ints[..1]))?);
            }
            ids.try_into().unwrap()
        },
        lists: {
            let mut ids = vec![];
            for (name, n, r) in LIST_FUNCTIONS {
                ids.push(module.declare_function(name, Linkage::Import, &signature(&ints[..n], &ints[..r]))?);
            }
            ids.try_into().unwrap()
        },
    };
    // The texts of contract messages and String literals, a single copy of each in the executable.
    let mut data: HashMap<String, DataId> = HashMap::new();
    let mut ids = vec![];
    for (i, f) in p.functions.iter().enumerate() {
        let sig = signature(&f.locals[..f.nb_params], &f.returns);
        let id = if Some(i) == p.main {
            module.declare_function("kova_main", Linkage::Export, &sig)?
        } else {
            module.declare_function(&format!("kova_f{i}"), Linkage::Local, &sig)?
        };
        ids.push(id);
    }

    let mut ctx = module.make_context();
    let mut fbc = FunctionBuilderContext::new();
    for (i, f) in p.functions.iter().enumerate() {
        ctx.func.signature = signature(&f.locals[..f.nb_params], &f.returns);
        let b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
        let refs: Vec<FuncRef> = ids.iter().map(|&id| module.declare_func_in_func(id, b.func)).collect();
        let rt_print = module.declare_func_in_func(rt.print, b.func);
        let rt_fault = module.declare_func_in_func(rt.fault, b.func);
        let mut texts = HashMap::new();
        let used = f.blocks.iter().flat_map(|b| &b.insts).flat_map(|i| match i {
            Inst::Fail(m) => m
                .iter()
                .filter_map(|m| match m {
                    Piece::Text(t) => Some(t.as_str()),
                    Piece::File(i) => Some(files[*i].as_str()),
                    Piece::Value(..) | Piece::Label(_) => None,
                })
                .collect(),
            Inst::Text(_, t) => vec![t.as_str()],
            _ => vec![],
        });
        for t in used {
            if !data.contains_key(t) {
                let id = module.declare_data(&format!("kova_text{}", data.len()), Linkage::Local, false, false)?;
                let mut d = DataDescription::new();
                // A final 0 byte: an empty text still gets an address.
                d.define([t.as_bytes(), &[0]].concat().into());
                module.define_data(id, &d)?;
                data.insert(t.to_string(), id);
            }
            texts.entry(t.to_string()).or_insert_with(|| module.declare_data_in_func(data[t], b.func));
        }
        let strings = Strings {
            new: module.declare_func_in_func(rt.str_new, b.func),
            free: module.declare_func_in_func(rt.str_free, b.func),
            ops: rt.str_ops.map(|id| module.declare_func_in_func(id, b.func)),
            shows: rt.shows.map(|id| module.declare_func_in_func(id, b.func)),
        };
        let fail = Fail {
            write_text: module.declare_func_in_func(rt.write_text, b.func),
            write_label: module.declare_func_in_func(rt.write_label, b.func),
            write_value: module.declare_func_in_func(rt.write_value, b.func),
            stop: module.declare_func_in_func(rt.fail, b.func),
            exit: module.declare_func_in_func(rt.exit, b.func),
            texts,
            files: files.to_vec(),
        };
        let lists = rt.lists.map(|id| module.declare_func_in_func(id, b.func));
        let math = module.declare_func_in_func(rt.math, b.func);
        Generator { b, vars: vec![], types: f.locals.clone(), call_conv, rt_fault, fail, strings, lists, math }.function(f, &refs, rt_print, config);
        module.define_function(ids[i], &mut ctx)?;
        module.clear_context(&mut ctx);
    }
    runtime_texts(&mut module, lang, files)?;
    Ok(module.finish().emit()?)
}

/// What the runtime reads to write its messages (runtime/kova.rs: `kova_texts`, `kova_langs`…):
/// `kova_nb_texts` texts per language, in the order of `locale::RUNTIME`; the languages' codes;
/// the source files' paths; the `header`'s language (`kova_forced_lang`), or -1. Each text ends
/// with a 0 byte, as C wants it.
fn runtime_texts(module: &mut ObjectModule, forced: Option<usize>, files: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut made = 0;
    let mut table = |module: &mut ObjectModule, name: &str, items: Vec<String>| -> Result<(), Box<dyn std::error::Error>> {
        let mut d = DataDescription::new();
        // Not `define_zeroinit`: that would be a zeroed section, which keeps no addresses.
        d.define(vec![0; items.len().max(1) * 8].into());
        d.set_align(8);
        for (i, item) in items.iter().enumerate() {
            let id = module.declare_data(&format!("kova_runtime_text{made}"), Linkage::Local, false, false)?;
            made += 1;
            let mut text = DataDescription::new();
            text.define([item.as_bytes(), &[0]].concat().into());
            module.define_data(id, &text)?;
            let address = module.declare_data_in_data(id, &mut d);
            d.write_data_addr(i as u32 * 8, address, 0);
        }
        let id = module.declare_data(name, Linkage::Export, false, false)?;
        Ok(module.define_data(id, &d)?)
    };
    let langs = locale::LANGUAGES.len();
    // The holes the runtime fills stay written `{name}`.
    let kept = |k: &str| locale::RUNTIME_HOLES.iter().find(|(key, _)| *key == k).map_or(vec![], |(_, h)| h.iter().map(|h| (*h, format!("{{{h}}}"))).collect());
    let texts = (0..langs)
        .flat_map(|l| {
            locale::RUNTIME.iter().map(move |k| {
                let holes = kept(k);
                let args: Vec<(&str, &dyn std::fmt::Display)> = holes.iter().map(|(h, v)| (*h, v as &dyn std::fmt::Display)).collect();
                locale::text_in(l, k, &args)
            })
        })
        .collect();
    table(module, "kova_texts", texts)?;
    table(module, "kova_langs", locale::LANGUAGES.iter().map(|(code, _)| code.to_string()).collect())?;
    table(module, "kova_files", files.to_vec())?;
    for (name, value) in [("kova_nb_langs", langs as i32), ("kova_nb_texts", locale::RUNTIME.len() as i32), ("kova_forced_lang", forced.map_or(-1, |l| l as i32))] {
        let mut d = DataDescription::new();
        d.define(value.to_le_bytes().into());
        d.set_align(4); // arm64 reads it with an instruction that needs it
        let id = module.declare_data(name, Linkage::Export, false, false)?;
        module.define_data(id, &d)?;
    }
    Ok(())
}

/// The number of a text of the runtime (`kova_texts` in the generated C).
fn runtime_text(key: &str) -> i64 {
    locale::RUNTIME.iter().position(|k| *k == key).expect("key of locale::RUNTIME") as i64
}

fn machine_type(t: Scalar) -> cranelift_codegen::ir::Type {
    match t {
        Scalar::Int => I64,
        Scalar::Float => F64,
    }
}

struct Generator<'a> {
    b: FunctionBuilder<'a>,
    vars: Vec<Variable>,
    types: Vec<Scalar>,
    /// For the signature of a block called through its address (`CallIndirect`).
    call_conv: CallConv,
    rt_fault: FuncRef,
    fail: Fail,
    strings: Strings,
    /// See `LIST_FUNCTIONS`.
    lists: [FuncRef; 10],
    /// `kova_math(op, x, y)`: `sin`, `pow`… (`shared::math`).
    math: FuncRef,
}

/// The runtime functions for Strings (`kova_str_*` in `runtime/kova.rs`).
struct Strings {
    new: FuncRef,
    free: FuncRef,
    ops: [FuncRef; STR_OPS],
    shows: [FuncRef; 6],
}

/// What is needed to write the message of a broken contract.
struct Fail {
    write_text: FuncRef,
    write_label: FuncRef,
    write_value: FuncRef,
    stop: FuncRef,
    /// `kova_exit`, for `exit(code)`.
    exit: FuncRef,
    texts: HashMap<String, GlobalValue>,
    files: Vec<String>,
}

impl Generator<'_> {
    fn function(mut self, f: &Function, refs: &[FuncRef], rt_print: FuncRef, config: TargetFrontendConfig) {
        let b = &mut self.b;
        self.vars = f.locals.iter().map(|&t| b.declare_var(machine_type(t))).collect();
        let blocks: Vec<Block> = f.blocks.iter().map(|_| b.create_block()).collect();

        // Entry block: stores the parameters in their variables, then jumps to block 0.
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let params = b.block_params(entry).to_vec();
        for (i, v) in params.into_iter().enumerate() {
            b.def_var(self.vars[i], v);
        }
        for (i, &t) in f.locals.iter().enumerate().skip(f.nb_params) {
            let zero = match t {
                Scalar::Int => b.ins().iconst(I64, 0),
                Scalar::Float => b.ins().f64const(0.0),
            };
            b.def_var(self.vars[i], zero);
        }
        b.ins().jump(blocks[0], &[]);

        for (k, block) in f.blocks.iter().enumerate() {
            self.b.switch_to_block(blocks[k]);
            for inst in &block.insts {
                self.inst(inst, refs, rt_print);
            }
            let b = &mut self.b;
            match &block.end {
                End::Jump(s) => {
                    b.ins().jump(blocks[*s], &[]);
                }
                End::Branch(c, yes, no) => {
                    let c = b.use_var(self.vars[*c]);
                    b.ins().brif(c, blocks[*yes], &[], blocks[*no], &[]);
                }
                End::Return(v) => {
                    let vals: Vec<Value> = v.iter().map(|&l| b.use_var(self.vars[l])).collect();
                    b.ins().return_(&vals);
                }
                End::Unreachable => {
                    b.ins().trap(TrapCode::unwrap_user(1));
                }
            }
        }
        self.b.seal_all_blocks();
        self.b.finalize(config);
    }

    fn read(&mut self, l: Local) -> Value {
        self.b.use_var(self.vars[l])
    }

    /// If `cond` is true: calls `kova_fault` (which stops the program).
    fn fault_if(&mut self, cond: Value, fault: Fault, loc: Loc) {
        let b = &mut self.b;
        let (fault_block, next) = (b.create_block(), b.create_block());
        b.set_cold_block(fault_block);
        b.ins().brif(cond, fault_block, &[], next, &[]);
        b.switch_to_block(fault_block);
        let args = [runtime_text(fault.key()), loc.packed(), loc.col as i64].map(|v| b.ins().iconst(I64, v));
        b.ins().call(self.rt_fault, &args);
        b.ins().trap(TrapCode::unwrap_user(1));
        b.switch_to_block(next);
    }

    fn inst(&mut self, inst: &Inst, refs: &[FuncRef], rt_print: FuncRef) {
        match inst {
            Inst::Const(d, v) => {
                let v = match self.types[*d] {
                    Scalar::Int => self.b.ins().iconst(I64, *v),
                    Scalar::Float => self.b.ins().f64const(f64::from_bits(*v as u64)),
                };
                self.b.def_var(self.vars[*d], v);
            }
            Inst::FBinary(d, op, a, b, loc) => {
                let (a, b) = (self.read(*a), self.read(*b));
                let v = self.float_op(*op, a, b, *loc);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::FNeg(d, a) => {
                let a = self.read(*a);
                let v = self.b.ins().fneg(a);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Math(d, m, x, y) => {
                let op = self.b.ins().iconst(I64, *m as i64);
                let (x, y) = (self.read(*x), self.read(*y));
                let call = self.b.ins().call(self.math, &[op, x, y]);
                let v = self.b.inst_results(call)[0];
                self.b.def_var(self.vars[*d], v);
            }
            Inst::FUnary(d, op, a) => {
                let a = self.read(*a);
                let ins = self.b.ins();
                let v = match op {
                    FloatOp::Abs => ins.fabs(a),
                    FloatOp::Sqrt => ins.sqrt(a),
                    FloatOp::Floor => ins.floor(a),
                    FloatOp::Ceil => ins.ceil(a),
                    // Like Rust's `round` (interp.rs): the whole part, one further when the rest
                    // is at least a half (`x - trunc(x)` is exact).
                    FloatOp::Round => {
                        let t = ins.trunc(a);
                        let rest = self.b.ins().fsub(a, t);
                        let rest = self.b.ins().fabs(rest);
                        let half = self.b.ins().f64const(0.5);
                        let up = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, rest, half);
                        let one = self.b.ins().f64const(1.0);
                        let step = self.b.ins().fcopysign(one, a);
                        let further = self.b.ins().fadd(t, step);
                        self.b.ins().select(up, further, t)
                    }
                };
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Convert(d, c, a, loc) => {
                let a = self.read(*a);
                let v = match c {
                    Conversion::IntToFloat => self.b.ins().fcvt_from_sint(F64, a),
                    Conversion::FromBits => self.b.ins().bitcast(F64, MemFlagsData::new(), a),
                    Conversion::ToBits => self.b.ins().bitcast(I64, MemFlagsData::new(), a),
                    Conversion::FloatToInt => {
                        // "Ordered" comparisons: false for NaN, so NaN is refused.
                        let min = self.b.ins().f64const(-9223372036854775808.0);
                        let max = self.b.ins().f64const(9223372036854775808.0);
                        let big_enough = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, a, min);
                        let small_enough = self.b.ins().fcmp(FloatCC::LessThan, a, max);
                        let ok = self.b.ins().band(big_enough, small_enough);
                        let ko = self.b.ins().bxor_imm_s(ok, 1);
                        self.fault_if(ko, Fault::BadConversion, *loc);
                        self.b.ins().fcvt_to_sint(I64, a)
                    }
                };
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Copy(d, s) => {
                let v = self.read(*s);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Not(d, a) => {
                let a = self.read(*a);
                let v = self.b.ins().bxor_imm_s(a, 1);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Neg(d, a, loc) => {
                let a = self.read(*a);
                let zero = self.b.ins().iconst(I64, 0);
                let (v, overflow) = self.b.ins().ssub_overflow(zero, a);
                self.fault_if(overflow, Fault::Overflow, *loc);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Binary(d, op, a, b, loc) => {
                let (a, b) = (self.read(*a), self.read(*b));
                let v = self.int_op(*op, a, b, *loc);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Call(d, f, args) => {
                let vals: Vec<Value> = args.iter().map(|&l| self.read(l)).collect();
                let call = self.b.ins().call(refs[*f], &vals);
                let results = self.b.inst_results(call).to_vec();
                for (&d, r) in d.iter().zip(results) {
                    self.b.def_var(self.vars[d], r);
                }
            }
            Inst::FuncRef(d, f) => {
                let address = self.b.ins().func_addr(I64, refs[*f]);
                self.b.def_var(self.vars[*d], address);
            }
            // The signature comes from the slots given and received.
            Inst::CallIndirect(d, f, args) => {
                let mut sig = Signature::new(self.call_conv);
                sig.params.extend(args.iter().map(|&l| AbiParam::new(machine_type(self.types[l]))));
                sig.returns.extend(d.iter().map(|&l| AbiParam::new(machine_type(self.types[l]))));
                let sig = self.b.import_signature(sig);
                let callee = self.read(*f);
                let vals: Vec<Value> = args.iter().map(|&l| self.read(l)).collect();
                let call = self.b.ins().call_indirect(sig, callee, &vals);
                let results = self.b.inst_results(call).to_vec();
                for (&d, r) in d.iter().zip(results) {
                    self.b.def_var(self.vars[d], r);
                }
            }
            Inst::Text(d, t) => {
                let address = self.b.ins().symbol_value(I64, self.fail.texts[t]);
                let n = self.b.ins().iconst(I64, t.len() as i64);
                let call = self.b.ins().call(self.strings.new, &[address, n]);
                let v = self.b.inst_results(call)[0];
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Str(d, op, args) => {
                let vals: Vec<Value> = match op {
                    StrOp::Format(show) => {
                        let (v, show) = self.value_and_show(args[0], *show);
                        vec![v, show]
                    }
                    _ => args.iter().map(|&l| self.read(l)).collect(),
                };
                let call = self.b.ins().call(self.strings.ops[str_op(*op).0], &vals);
                let v = self.b.inst_results(call)[0];
                self.b.def_var(self.vars[*d], v);
            }
            Inst::Free(l) => {
                let v = self.read(*l);
                self.b.ins().call(self.strings.free, &[v]);
                let zero = self.b.ins().iconst(I64, 0);
                self.b.def_var(self.vars[*l], zero);
            }
            Inst::ListNew(d, width) => {
                let w = self.b.ins().iconst(I64, *width as i64);
                let v = self.list_call(L_NEW, &[w]);
                self.b.def_var(self.vars[*d], v.unwrap());
            }
            Inst::ListLength(d, l) => {
                let l = self.read(*l);
                let v = self.list_call(L_LENGTH, &[l]);
                self.b.def_var(self.vars[*d], v.unwrap());
            }
            Inst::FaultIf(c, f, loc) => {
                let c = self.read(*c);
                self.fault_if(c, *f, *loc);
            }
            Inst::ListCheck(l, i, loc) => {
                let (l, i) = (self.read(*l), self.read(*i));
                let line = self.b.ins().iconst(I64, loc.packed());
                let col = self.b.ins().iconst(I64, loc.col as i64);
                self.list_call(L_CHECK, &[l, i, line, col]);
            }
            Inst::ListLoad(d, l, i, k) => {
                let (l, i) = (self.read(*l), self.read(*i));
                let k = self.b.ins().iconst(I64, *k as i64);
                let mut v = self.list_call(L_LOAD, &[l, i, k]).unwrap();
                if self.types[*d] == Scalar::Float {
                    v = self.b.ins().bitcast(F64, MemFlagsData::new(), v);
                }
                self.b.def_var(self.vars[*d], v);
            }
            Inst::ListStore(l, i, k, value) => {
                let (l, i) = (self.read(*l), self.read(*i));
                let k = self.b.ins().iconst(I64, *k as i64);
                let mut v = self.read(*value);
                if self.types[*value] == Scalar::Float {
                    v = self.b.ins().bitcast(I64, MemFlagsData::new(), v);
                }
                self.list_call(L_STORE, &[l, i, k, v]);
            }
            Inst::ListGrow(d, l) => {
                let l = self.read(*l);
                let v = self.list_call(L_GROW, &[l]);
                self.b.def_var(self.vars[*d], v.unwrap());
            }
            Inst::ListRemove(l, i) => {
                let (l, i) = (self.read(*l), self.read(*i));
                self.list_call(L_REMOVE, &[l, i]);
            }
            Inst::ListIndex(d, l) => {
                let l = self.read(*l);
                let v = self.list_call(L_INDEX, &[l]).unwrap();
                self.b.def_var(self.vars[*d], v);
            }
            Inst::ListSetIndex(l, v) => {
                let (l, v) = (self.read(*l), self.read(*v));
                self.list_call(L_SET_INDEX, &[l, v]);
            }
            // Like `interp::mix`.
            Inst::Mix(d, a, b) => {
                let (a, b) = (self.read(*a), self.read(*b));
                let mut x = self.b.ins().bxor(a, b);
                for (shift, times) in [(30, 0xbf58476d1ce4e5b9u64 as i64), (27, 0x94d049bb133111ebu64 as i64)] {
                    let high = self.b.ins().ushr_imm_u(x, shift);
                    let y = self.b.ins().bxor(x, high);
                    x = self.b.ins().imul_imm_u(y, times);
                }
                let high = self.b.ins().ushr_imm_u(x, 31);
                let x = self.b.ins().bxor(x, high);
                let v = self.b.ins().ushr_imm_u(x, 1);
                self.b.def_var(self.vars[*d], v);
            }
            Inst::ListFree(l) => {
                let v = self.read(*l);
                self.list_call(L_FREE, &[v]);
                let zero = self.b.ins().iconst(I64, 0);
                self.b.def_var(self.vars[*l], zero);
            }
            Inst::Print { value, show, newline } => {
                let (v, show) = self.value_and_show(*value, *show);
                let newline = self.b.ins().iconst(I64, *newline as i64);
                self.b.ins().call(rt_print, &[v, show, newline]);
            }
            Inst::Line(..) => {}
            Inst::Exit(code) => {
                let code = self.read(*code);
                self.b.ins().call(self.fail.exit, &[code]);
            }
            Inst::Fail(pieces) => {
                for m in pieces {
                    let t = match m {
                        Piece::Value(l, show) => {
                            let (v, show) = self.value_and_show(*l, *show);
                            self.b.ins().call(self.fail.write_value, &[v, show]);
                            continue;
                        }
                        Piece::Label(k) => {
                            let i = self.b.ins().iconst(I64, runtime_text(k));
                            self.b.ins().call(self.fail.write_label, &[i]);
                            continue;
                        }
                        Piece::Text(t) => t,
                        Piece::File(i) => &self.fail.files[*i],
                    };
                    let address = self.b.ins().symbol_value(I64, self.fail.texts[t]);
                    let n = self.b.ins().iconst(I64, t.len() as i64);
                    self.b.ins().call(self.fail.write_text, &[address, n]);
                }
                self.b.ins().call(self.fail.stop, &[]);
            }
        }
    }

    /// Calls the List function `k` of `LIST_FUNCTIONS`; its result, if it has one.
    fn list_call(&mut self, k: usize, args: &[Value]) -> Option<Value> {
        let call = self.b.ins().call(self.lists[k], args);
        self.b.inst_results(call).first().copied()
    }

    /// A slot and its display kind, ready for the runtime.
    fn value_and_show(&mut self, l: Local, show: Show) -> (Value, Value) {
        let mut v = self.read(l);
        if show == Show::Float {
            // The runtime receives the 64 bits of the Float in an integer.
            v = self.b.ins().bitcast(I64, MemFlagsData::new(), v);
        }
        (v, self.b.ins().func_addr(I64, self.strings.shows[show as usize]))
    }

    /// Same rules as `interp::compute_float`.
    fn float_op(&mut self, op: Op, a: Value, b: Value, loc: Loc) -> Value {
        let compare = |g: &mut Self, cc: FloatCC| {
            let c = g.b.ins().fcmp(cc, a, b);
            g.b.ins().uextend(I64, c)
        };
        match op {
            Op::Plus => self.b.ins().fadd(a, b),
            Op::Minus => self.b.ins().fsub(a, b),
            Op::Times => self.b.ins().fmul(a, b),
            Op::Div => {
                let zero = self.b.ins().f64const(0.0);
                let by_zero = self.b.ins().fcmp(FloatCC::Equal, b, zero); // true for -0.0 too
                self.fault_if(by_zero, Fault::DivisionByZero, loc);
                self.b.ins().fdiv(a, b)
            }
            Op::Eq => compare(self, FloatCC::Equal),
            Op::Ne => compare(self, FloatCC::NotEqual), // true for NaN, as in Rust
            Op::Lt => compare(self, FloatCC::LessThan),
            Op::Le => compare(self, FloatCC::LessThanOrEqual),
            Op::Gt => compare(self, FloatCC::GreaterThan),
            Op::Ge => compare(self, FloatCC::GreaterThanOrEqual),
            Op::Rem => unreachable!("`%` refused on Floats by the type checker"),
        }
    }

    /// Same rules as `interp::compute`.
    fn int_op(&mut self, op: Op, a: Value, b: Value, loc: Loc) -> Value {
        let compare = |g: &mut Self, cc: IntCC| {
            let c = g.b.ins().icmp(cc, a, b);
            g.b.ins().uextend(I64, c)
        };
        match op {
            Op::Plus | Op::Minus | Op::Times => {
                let ins = self.b.ins();
                let (v, overflow) = match op {
                    Op::Plus => ins.sadd_overflow(a, b),
                    Op::Minus => ins.ssub_overflow(a, b),
                    _ => ins.smul_overflow(a, b),
                };
                self.fault_if(overflow, Fault::Overflow, loc);
                v
            }
            Op::Div | Op::Rem => {
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, b, 0);
                self.fault_if(zero, Fault::DivisionByZero, loc);
                let minus_one = self.b.ins().icmp_imm_s(IntCC::Equal, b, -1);
                if op == Op::Div {
                    // -9223372036854775808 / -1 doesn't fit in an Int.
                    let min = self.b.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
                    let overflow = self.b.ins().band(min, minus_one);
                    self.fault_if(overflow, Fault::Overflow, loc);
                    self.b.ins().sdiv(a, b)
                } else {
                    // x % -1 = x % 1 = 0; avoids MIN % -1, which crashes the processor.
                    let one = self.b.ins().iconst(I64, 1);
                    let b = self.b.ins().select(minus_one, one, b);
                    self.b.ins().srem(a, b)
                }
            }
            Op::Eq => compare(self, IntCC::Equal),
            Op::Ne => compare(self, IntCC::NotEqual),
            Op::Lt => compare(self, IntCC::SignedLessThan),
            Op::Le => compare(self, IntCC::SignedLessThanOrEqual),
            Op::Gt => compare(self, IntCC::SignedGreaterThan),
            Op::Ge => compare(self, IntCC::SignedGreaterThanOrEqual),
        }
    }
}
