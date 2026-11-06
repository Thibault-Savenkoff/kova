# Kova

Kova is a compiled programming language with strict types and memory managed without a garbage collector.
Run a program at once with `kova run`, or build a small native executable with `kova build`.

Documentation: **https://thibault-savenkoff.github.io/kova/**, in English and French (getting started, the language guide, the library, the tools). Its source is [`docs/site`](docs/site/src/content/docs); to read it locally: `cd docs/site && npm install && npm run build && npm run preview`.

```kova
struct Dog {
    name: String
    age: Int
}

fn describe(d: Dog) -> String {
    return match d.age {
        0..=1 => "{d.name} is a puppy"
        _ => "{d.name} is {d.age}"
    }
}

fn main {
    let dogs: List<Dog> := [Dog { name := "Rex", age := 3 }, Dog { name := "Pip", age := 1 }]
    for d in dogs {
        print(describe(d))
    }
    print(dogs.map { d: Dog => d.age * 7 })
    match "12".to_int {
        none => print("not a number")
        n => print(n * 2)
    }
}
```

```text
Rex is 3
Pip is a puppy
[21, 7]
24
```

## What it is like

- **Types are written.** `age: Int`, `fn describe(d: Dog) -> String`. The compiler checks them; it
  guesses almost nothing.
- **`:=` gives, `=` compares.** No `==`.
- **Memory without a garbage collector, and without lifetimes.** Every value has one owner. A function
  borrows what it is given; when it changes or takes a value, the call says so: `sort(var names)`,
  `keep(move file)`.
- **No hidden failures.** A missing value is a `T?`, an operation that can fail gives a `T or E`, and an
  `Int` that overflows stops the program with a clear message instead of wrapping around.
- **Contracts and tests in the code**: `require`, `ensure`, `check`, `test "…" { … }`.
- **The same results everywhere.** `kova run` and `kova build` print the same digits, on Linux, Windows
  and macOS: Kova brings its own maths.
- **Kova speaks English and French** (`KOVA_LANG=fr`), and its programs can be translated too.

The [language guide](https://thibault-savenkoff.github.io/kova/guide/values/) explains the whole language, chapter by chapter.

## What it can do

Everything here comes with Kova, with nothing else to install unless noted.

| | |
| --- | --- |
| **Windows** | `import ui`: each system's native widgets, a canvas, OpenGL (`ui.gpu`), menus, dialogs, your own frames |
| **Web** | `import web`: HTTP(S) client and server; `import websocket`, `import net` (TCP/UDP) |
| **Concurrency** | `import threads`: tasks, channels, shared state, `select`; `l.map workers(8) { … }` |
| **Data** | JSON, CSV, TOML, XML, regular expressions, zip, compression; SQLite through the system's library |
| **Everyday** | files, paths, dates, command-line arguments, big integers, secure random, encryption, sound (WAV, MP3) |
| **C libraries** | `extern`: call any C function, with structs and callbacks |
| **Packages** | `import json from "github.com/ada/kova-json" version "1.2.0"`, locked by `kova.lock`; `kova search` finds them, no server involved |
| **Targets** | Linux, Windows and Mac (x86-64 and arm64), the browser (WebAssembly, one `.html`), Android (a signed `.apk`), all from any machine |

## Install

A ready-made `kova`, with nothing else to install (no Rust, no C compiler, no linker). On Linux and
macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.sh | sh
```

On Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.ps1 | iex
```

Each `kova` builds for every system (`kova build app.kova --target windows`, `linux`, `mac`, their
`-arm` variants, `web`, `android`); on a Mac, one executable runs on every Mac. To build Kova itself from a
clone: install Rust with [rustup](https://rustup.rs), then

```sh
rustup target add wasm32-unknown-unknown x86_64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-pc-windows-msvc
cargo install --path . --locked
```

## Use

```sh
kova run hello.kova          # runs it
kova build hello.kova        # makes the executable `hello`
kova test hello.kova         # runs its `test` blocks
kova fmt hello.kova          # lays it out
```

The VSCode / VSCodium extension in [`editors/vscode`](editors/vscode) gives colors, errors while typing,
completion, types on hover, renaming, run / build / test buttons and a debugger.

## Status

Kova is young (version 0.2) and still changes. Everything above works and is tested, but it has
not yet been proven by large programs or by other people's use. What to expect:

- **No references that outlive their block.** That is what removes lifetimes, and its price: a tree
  whose nodes point back at their parent, or a graph, is written with
  [identifiers as links](https://thibault-savenkoff.github.io/kova/guide/ownership/#trees-and-graphs):
  in a `Map`, a link to a removed node is `none`, never the wrong node, at the cost of a lookup.
- **Speed.** Native code through Cranelift, built in milliseconds, but less optimized than LLVM. In the browser,
  WebAssembly runs 1.3 to 4 times slower than Kova's native code.
- **Android** has only been run on emulators, and apps are not ready for the Play Store. **iOS** is
  not done.
- **The package index is new and empty**: yours could be the first.

## How Kova is made

Kova is a vibe-coded language, but a tested one: its author decides what Kova is (what a programmer
sees, what comes next), and an AI writes the compiler and chooses the technical details. No person
has reviewed the compiler. Every change must pass the tests: about 200 programs that `kova run` and
`kova build` must run with the same output, and about 200 programs whose errors must be the
expected ones. Linux, Windows and macOS are checked at each milestone.

## License

Kova is under either of the [MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your
choice. To report a security flaw, see [SECURITY.md](SECURITY.md).
