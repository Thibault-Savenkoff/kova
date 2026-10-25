# How Kova is built

For people who want to read or change the compiler. Everything is in the repository's `src/` and
`runtime/` folders, written in Rust.

## The pipeline

```text
text ─ lexer.rs ─ parser.rs ─ types.rs ─ ownership.rs ─ kir.rs ─┬─ interp.rs   (kova run, kova test)
                                                                └─ codegen.rs  (kova build) ─ link.rs
```

1. **`lexer.rs`** cuts the text into tokens, each with its position.
2. **`parser.rs`** builds the tree (`ast.rs`). It keeps going after an error, so the editor can show
   several at once.
3. **`types.rs`** is the checker: it types every expression and records what the later passes need
   (`Info`). Types are mostly written by the programmer; almost nothing is inferred.
4. **`ownership.rs`** is the only judge of moves and borrows (the "mutable value semantics" of
   chapter 7: no lifetimes, markers `var` / `move` at each call).
5. **`kir.rs`** turns the checked tree into KIR, a small typed instruction set shared by both
   backends. It also makes copies of generic functions and builds the helper functions (drop, clone,
   show, equal) of each type.
6. **`interp.rs`** runs KIR directly. **`codegen.rs`** turns it into machine code with Cranelift for
   x86-64 and arm64.
7. **`link.rs`** is Kova's own linker: it writes ELF (Linux), PE (Windows) and Mach-O (macOS)
   executables from the program's object and the runtime. No C compiler or system linker is needed.

`kova run` and `kova build` must print exactly the same: the tests run every program both ways.

## The runtime

`runtime/kova.rs` is a `no_std` Rust library (texts, lists, files, the web, regular expressions, JSON,
cryptography…), with one file per system for what it asks of it: `linux.rs` (raw system calls, its own
memory allocator), `windows.rs` (`kernel32.dll` and a few others), `macos.rs` (`libSystem`).
Code both the interpreter and the runtime need (number formatting, dates, HTTP, `src/shared.rs`,
`src/json.rs`, `src/regex.rs`, `src/crypto.rs`…) is written once and included in both, so that they
cannot differ.

## The editor side

`lsp.rs` (language server) and `debug.rs` (debug adapter) call the same `analyze` function the
command line does, on text that is not saved yet.

## Messages and languages

Every text Kova shows is a key in `locales/en.kova` and `locales/fr.kova`; the unit test
`complete_catalogs` checks that both have the same keys and holes. The runtime's own texts are
written into each executable, which picks its language when it starts.

## Tests

- `cargo test` runs the unit tests and `tests/kova.rs`: every `tests/ok/*.kova` (with `// expect:`
  lines, run **and** built), every `tests/error/*.kova` (with `// error:` lines), the guide's
  examples, the language server and the debugger.
- A program of several files is a folder `tests/ok/<name>/main.kova`. `// stdin:` and `// args:` feed
  the program.
- Code behind `cfg(target_os = …)` is checked from any machine with
  `cargo check --tests --target aarch64-apple-darwin` and `--target x86_64-pc-windows-msvc`.
- `.github/workflows/tests.yml` runs everything on Linux, Windows and macOS (x86-64 and arm64) when
  started by hand.

## Adding something

- A message: a key in both `locales/en.kova` and `locales/fr.kova`.
- A built-in method: its signature in `types.rs`, its translation in `kir.rs`, a test in `tests/ok`.
- A runtime function: write it once in `src/shared.rs` when both backends need it.
