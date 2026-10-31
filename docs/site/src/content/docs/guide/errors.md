---
title: Missing values and errors
description: T? for a value that may be missing, T or E for an operation that may fail, and the ? that passes them on.
sidebar:
  order: 9
---

Two things go wrong in every program: a value that is not there (no such key, a text that is not a
number), and an operation that fails (a missing file, a site that does not answer). Many languages
let both pass unseen, with a `null` that crashes far from where it came from, or an exception that
nothing in the code announced. In Kova, both are written in the **type**, so the compiler makes
sure each one is handled.

## `T?`: a value that may be missing

`Int?` is an `Int` or `none`. `"42".to_int` gives an `Int?`, since the text might not be a number;
so does `ages["Zed"]` on a map:

```kova
fn main {
    let a: Int? := "42".to_int
    let b: Int? := "forty".to_int
    print(a)
    print(b)
}
```

```text
42
none
```

An `Int?` cannot be used as an `Int` until the program has checked it:

```kova
fn main {
    let a: Int? := "4".to_int
    print(a + 1)
}
```

```txt
error: `+` applies to `Int`, `Float`, `String` or `BigInt`, not to `Int?`
 --> main.kova:3:11
  |
3 |     print(a + 1)
  |           ^
```

Three ways to check it:

```kova
fn main {
    let a: Int? := "4".to_int
    // 1. Inside `if a != none`, `a` is an `Int`.
    if a != none {
        print(a + 1)
    }
    // 2. `match` names the value when there is one.
    match "x".to_int {
        none => print("not a number")
        n => print(n * 2)
    }
    // 3. After leaving when it is missing, `a` is an `Int` until the end.
    if a = none {
        return
    }
    print(a * 10)
}
```

```text
5
not a number
40
```

This is called **narrowing**: after the check, the compiler knows the value is there, and the
type becomes `Int`. Nothing is unwrapped by hand, and nothing can be forgotten.

A field, a parameter or a result that may be missing says so the same way: `born: Int?`,
`-> String?`. `none` is the missing value, and a plain `Int` is accepted wherever an `Int?` is
expected.

## `T or E`: an operation that may fail

A function that may fail returns `T or E`: a result of type `T`, or an error of type `E`. It gives
`ok(value)` or `err(error)`:

```kova
fn half(n: Int) -> Int or String {
    if n % 2 != 0 {
        return err("{n} is odd")
    }
    return ok(n / 2)
}

fn main {
    match half(10) {
        ok(h) => print(h)
        err(e) => print("error: {e}")
    }
    match half(7) {
        ok(h) => print(h)
        err(e) => print("error: {e}")
    }
}
```

```text
5
error: 7 is odd
```

The error can be any type: a text, or better an enum listing what can go wrong, as the library
does (`files.IoError` is `NotFound`, `Denied`, …).

A result is never silently dropped. Calling a function that may fail without looking at its result
is refused, since the failure would go unseen:

```kova
import files

fn main {
    files.write("note.txt", "hello")
}
```

```txt
error: this result is not looked at: a failure would go unseen
 --> main.kova:4:5
  |
4 |     files.write("note.txt", "hello")
  |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  = help: write `files.write("note.txt", "hello")?` to pass the failure on, or `match` it
```

An operation that gives nothing when it works returns `Nothing or E`, and a bare `ok`:

```kova
fn validate(age: Int) -> Nothing or String {
    if age < 0 {
        return err("an age is never negative")
    }
    return ok
}

fn main {
    match validate(-3) {
        ok => print("fine")
        err(e) => print(e)
    }
}
```

```text
an age is never negative
```

## `?`: passing it on

Most of the time, a function that meets an error cannot do anything about it but give it to its
caller. `?` after a call does exactly that: if the call failed, the function returns the error at
once; otherwise, `?` gives the value.

```kova
fn half(n: Int) -> Int or String {
    if n % 2 != 0 {
        return err("{n} is odd")
    }
    return ok(n / 2)
}

fn quarter(n: Int) -> Int or String {
    let h: Int := half(n)?
    return half(h)
}

fn main {
    print(quarter(12))
    print(quarter(6))
}
```

```text
ok(3)
err("3 is odd")
```

`?` is visible: every place where a function may stop early is marked, unlike an exception that
can leave from any call. It works on `T?` too, in a function that returns a `T?`: a `none` is then
returned at once.

The function's own result must allow it, and the compiler says so when it does not, as in a
function that returns nothing:

```kova
import files

fn show {
    let text: String := files.read("note.txt")?
    print(text)
}

fn main {
    show
}
```

```txt
error: `?` gives the error (`files.IoError`) back to the caller: the function must return `… or files.IoError`; it returns nothing
 --> main.kova:4:25
  |
4 |     let text: String := files.read("note.txt")?
  |                         ^^^^^^^^^^^^^^^^^^^^^^^
  = help: or handle the error yourself: `match … { ok(v) => …, err(e) => … }`
```

`main` and tests have nobody to give the error to, so there `?` stops the program and says why.
A small script can read its file in one line:

```kova
import files

fn main {
    let text: String := files.read("note.txt")?
    print(text)
}
```

```txt
error: NotFound("note.txt")
  --> main.kova:4:25
```

## `else`: a value to fall back on

`else` after an optional or a result gives the value inside it, or the one on its right when
there is none. It says in one line what the program does when the value is missing:

```kova
fn main {
    let ages: Map<String, Int> := ["Rex" := 3]
    print(ages["Rex"] else 0)
    print(ages["Fido"] else 0)
    print("12x".to_int else -1)
}
```

```text
3
0
-1
```

The right side is only computed when it is needed. `x` stays usable afterwards: `else` reads it
and copies the value out.

### Errors of your own

A program usually has its own error enum. When one of its cases holds the error met, `?` puts it
there by itself:

```kova
import files

enum AppError {
    Empty
    Io(error: files.IoError)
}

fn load(path: String) -> String or AppError {
    let text: String := files.read(path)?
    if text = "" {
        return err(AppError.Empty)
    }
    return ok(text)
}

fn main {
    match load("missing.txt") {
        ok(text) => print(text)
        err(e) => print(e)
    }
}
```

```text
AppError.Io(NotFound("missing.txt"))
```

## When the program itself is wrong

`T?` and `T or E` are for what may happen to a correct program: a user typing a word instead of a
number, a file that was deleted. A mistake in the program itself (a position past the end of a list,
a division by zero, a [contract](/guide/contracts-and-tests/) broken) is not something to handle:
the program stops at once, says what and where, and ends with the exit code 101. To stop on
purpose, `system.exit(code)` ends the program with that code.
