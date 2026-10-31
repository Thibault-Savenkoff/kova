---
title: Modules
description: Programs of several files, what each file shows to the others, and Kova's own modules.
sidebar:
  order: 13
---

A program grows beyond one file by **importing** others. Each file is a module: its name is the
file's name, and it shows the others only what it chooses.

```kova
// main.kova
import geometry

fn main {
    let p := geometry.Point { x := 3.0, y := 4.0 }
    print(geometry.length(p))
}
```

```kova
// geometry.kova, next to main.kova
public:

struct Point {
public:
    x: Float
    y: Float
}

fn length(p: Point) -> Float {
    return (square(p.x) + square(p.y)).sqrt
}

private:

fn square(x: Float) -> Float {
    return x * x
}
```

```text
5.0
```

`kova run main.kova` finds `geometry.kova` by itself: there is no project file, no list of files to
keep up to date. The names of a module are always written with it, `geometry.Point`,
`geometry.length`, so a reader always knows where a name comes from.

## Public and private

In an imported file, everything is **private** unless written under `public:`: only that file can
use it. `private:` hides again what follows. A struct's fields follow the same rule inside the
struct, so a type can show its methods and keep its fields to itself.

```kova
// main.kova
import geometry

fn main {
    print(geometry.square(2.0))
}
```

```kova
// geometry.kova
fn square(x: Float) -> Float {
    return x * x
}
```

```txt
error: `geometry.square` is private in geometry.kova
 --> main.kova:5:11
  |
5 |     print(geometry.square(2.0))
  |           ^^^^^^^^^^^^^^^
  = help: to use it from another file, write it under `public:` in geometry.kova
```

Private by default means a module's inside can change without breaking the files that use it:
only what is public is a promise. The compiler also refuses a public function that takes or gives
a private type, since no one outside could use it.

## Other names, other folders

- `import geometry as geo` gives the module a shorter name: `geo.Point`.
- A file in a folder is written with dots: `import shapes.circle` reads `shapes/circle.kova`, and
  calls it `circle` (`circle.Circle`).
- Every `import` starts from the folder of the program's main file, even inside
  `shapes/circle.kova`: `import shapes.point`, never a path that depends on where the importing
  file is.
- Two files cannot import each other, directly or through others: the compiler shows the circle.

## Kova's own modules

Some of the library is in modules that come with Kova. They need an `import`, but no file:

| Module | For |
| --- | --- |
| [`files`](/library/files/) | reading and writing files, folders |
| [`system`](/library/system/) | environment variables, running other programs |
| [`web`](/library/web/) | asking sites, and [answering](/library/web-server/) as a server |
| [`secure`](/library/secure/) | secure chance, password hashes, encryption |
| [`log`](/library/log/) | dated lines on the error output, by level |
| [`ui`](/library/ui/) | native windows |

```kova
import files

fn main {
    print(files.exists("surely-not-here.txt"))
}
```

```text
false
```

Without the `import`, the compiler says which one to write. What every program has without an
import (`print`, `random`, texts, lists, dates, JSON…) is listed in
[the library](/library/overview/).

## Tests and modules

A module has no `main`: it is not run on its own. `kova test geometry.kova` runs its tests, and
the editor checks it like any file.

## Packages

A **package** is a module someone else wrote, published as a git repository. The `import` names
it by its address and an exact version:

```kova
import json from "github.com/ada/kova-json" version "1.2.0"

fn main {
    print(json.pretty("[1, 2]"))
}
```

How it behaves:

- **The first `kova run`, `kova build` or `kova test`** downloads it with `git` (which must be
  installed): the tag `v1.2.0`, or else `1.2.0`, of `https://github.com/ada/kova-json`. It is
  kept in a cache every program shares (`~/.cache/kova/packages` on Linux,
  `~/Library/Caches/kova/packages` on a Mac, `%LOCALAPPDATA%\kova\packages` on Windows; the
  variable `KOVA_PACKAGES` chooses another folder). After that, no network is needed.
- **`kova.lock`**, written next to the program, notes each package's commit and a SHA-256 of its
  files. If those files are ever different (the tag moved, the cache was edited), the program is
  refused:

  ```plaintext
  error: `github.com/ada/kova-json` 1.2.0 is not what it was when `kova.lock` noted it: its files changed. If that is expected, remove its line from `kova.lock`
  ```

  Put `kova.lock` in your own repository: whoever builds your program gets exactly your packages.
- **The editor never downloads**: until the first `kova run`, it says the package is not
  downloaded yet.
- **A program uses one version of a package.** Importing it twice with two versions is an error.

A package's repository holds a `package.kova`: that is what `import` reads. Its other files are
its own modules (`import words` inside it reads its `words.kova`), and like any module it shows
only what is under `public:`.

:::caution[A package runs with your rights]
A package's code is code your program runs: it can read and write your files, run programs
(`system.run`) and call C (`extern`). Import only packages you trust, at a version you have read.
:::
