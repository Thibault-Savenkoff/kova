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

## Install

Kova is written in Rust. Install Rust with [rustup](https://rustup.rs), then:

```sh
cargo install --git https://github.com/Thibault-Savenkoff/kova --locked
```

`kova build` needs nothing else: Kova links the executables itself, on Linux, Windows and macOS (x86-64
and arm64 each). On a Mac, one executable runs on every Mac (Apple chips and Intel).

To build for other systems (`kova build app.kova --target windows`), add their Rust targets **before**
installing Kova:

```sh
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-pc-windows-msvc
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

Kova is young (version 0.1): the language works and is tested on the three systems, but it still changes.
The web (`import web`: `web.get`, `web.post`, a server with `web.serve`, `https://`) and files
(`import files`) are Kova's own modules.

## How Kova is made

Kova is a vibe-coded language, but a tested one: its author decides what Kova is (its syntax, its keywords,
what a programmer sees, what comes next), and an AI writes the compiler and chooses the technical details.
Every change must pass the tests: 107 programs that `kova run` and `kova build` must run with the same
output on Linux, Windows and macOS, and 132 programs whose errors must be the expected ones.

## License

Kova is under either of the [MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your
choice. To report a security flaw, see [SECURITY.md](SECURITY.md).
