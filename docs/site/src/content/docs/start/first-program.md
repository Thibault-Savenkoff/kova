---
title: Your first program
description: Write, run and build a Kova program, and read an error.
sidebar:
  order: 2
---

## Hello

Make a file `hello.kova`:

```kova
fn main {
    print("Hello, Kova!")
}
```

Run it:

```sh
kova run hello.kova
```

```text
Hello, Kova!
```

`fn main` is where every program starts. It has no `()`: in Kova, a function without parameters
is written and called without parentheses (you will meet `x.to_float` and `rex.sound` the same
way). `print` writes a line.

## Run or build

There are two ways to start a program, and they always print the same thing:

- `kova run hello.kova` starts it at once. Kova reads the program, checks it, and runs it in its
  own interpreter. This is what you use while writing.
- `kova build hello.kova` makes a native executable, `hello` (`hello.exe` on Windows), next to the
  file. It is a small file (about 30 KB here) that runs on its own, on any machine of that system,
  without Kova installed:

```sh
kova build hello.kova
./hello
```

```console
✓ hello (Linux, 29 KB) in 3 ms
Hello, Kova!
```

Add `--target windows` (or `linux`, `mac`…) to make the executable for another system, from any
system. [Building](/tools/command/) lists them all.

## Giving the program words

What follows the file name on the command line reaches the program as `args`, a list of texts:

```kova
fn main {
    for word in args {
        print("Hello, {word}!")
    }
}
```

```sh
kova run hello.kova Ada Alan
```

```console
Hello, Ada!
Hello, Alan!
```

`{word}` inside a text puts the value of `word` there.

## When something is wrong

Kova checks the whole program before running any of it. A mistake is shown with its place and,
often, a way out:

```kova
fn main {
    let name: String := "Ada"
    print("Hello, " + name.length)
}
```

```txt
error: different types: `String` expected, `Int` given
 --> main.kova:3:23
  |
3 |     print("Hello, " + name.length)
  |                       ^^^^^^^^^^^
  = help: put the value inside the text: `"total: {x}"`
```

`name.length` is a number, and `+` joins texts only with texts: Kova never turns one type into
another without being asked. Here, write `"Hello, {name.length}"`, or `name.length.to_string`.

In the editor, the same message appears under the code while you type.

A program can also stop while it runs: an index past the end of a list, a number too large for an
`Int`, a [contract](/guide/contracts-and-tests/) that fails. It then says what happened and where,
and ends with the exit code 101.

Next: [a tour of Kova](/start/tour/), or go straight to the [language guide](/guide/values/).
