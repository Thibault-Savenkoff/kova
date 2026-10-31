---
title: Calling C
description: Using a library of the system (SQLite, the C library, Win32…) with extern.
sidebar:
  order: 16
---

Most of what a system offers (databases like SQLite, graphics, the system's own functions) is a
library written in C. `extern` declares the functions of such a library once, with Kova types, and
the program then calls them like the functions of a module:

```kova
extern c from "c" {
    fn abs(n: Int32) -> Int32
    fn length(text: String) -> Int from "strlen"
    fn getenv(name: String) -> String?
}

fn main {
    print(c.abs(-42))
    print(c.length("héllo"))
    print(c.getenv("KOVA_SURELY_NOT_SET"))
}
```

```text
42
6
none
```

- `extern c from "c"` names the block `c`, for the library `"c"`.
- Each `fn` gives a C function a Kova signature. `from "strlen"` gives its C name when the Kova name
  differs.
- There is no C compiler involved, and no header file read: the declaration is the whole contract.

## Finding the library

A library is named without its prefix and suffix: `"sqlite3"` is `libsqlite3.so` on Linux,
`libsqlite3.dylib` on macOS and `sqlite3.dll` on Windows. `"c"` and `"m"` are the system's C
library. It is looked for when the program first calls it, so a program that never calls it runs
without it.

When a system names a library or a function otherwise, say so after its name: `extern sqlite from
"sqlite3" windows "winsqlite3"`, `fn gmtime(…) from "gmtime" windows "_gmtime64"`. The systems are
`linux`, `mac` and `windows`; each build takes its own names.

## Where the care goes

C has no safety net. A function declared with the wrong types, or called against its rules (a
pointer freed twice), can crash the program in ways Kova cannot see. So the danger is gathered in
one place: the `extern` block. Check each declaration against the library's documentation; the
calls then look and behave like any other.

Never declare a function that takes a variable number of arguments (`printf`): on some processors,
its arguments would go to the wrong places.

## What crosses

| Kova | C | Notes |
| --- | --- | --- |
| `Int` | `int64_t`, `long` (Linux, macOS), `size_t` | |
| `Int32` | `int` | an `Int` too large for it stops the program |
| `Float` | `double` | |
| `Bool` | `int` | not C99's `bool` |
| `String` | `const char*` | C gets a copy ended by a 0 byte; a `String` result is copied too |
| `WideString` | `const wchar_t*` (Windows' UTF-16) | the program still gives and gets `String`s |
| `Pointer` | any pointer | an address C gives and takes back; Kova never looks inside |
| `String?`, `Pointer?` | a pointer that may be `NULL` | `none` is `NULL` |

`Int32` exists only in `extern` declarations: the program sees an `Int`.

## C writing into a variable: `var`

Some functions give their result through a pointer, as `sqlite3_open` does with the database it
opens. A `var` parameter gives C the address of a variable; after the call, the variable holds what
C wrote:

```kova
extern sqlite from "sqlite3" {
    fn open(path: String, db: var Pointer?) -> Int32 from "sqlite3_open"
    fn close(db: Pointer) -> Int32 from "sqlite3_close"
}

fn main {
    var db: Pointer? := none
    print(sqlite.open("notes.db", var db))
    if db = none {
        return
    }
    print(sqlite.close(db))
}
```

After `if db = none { return }`, `db` is a plain `Pointer` until a line changes it.

## C calling the program back

Some C functions call a function of the program: `qsort` to compare two elements, a window system
when a button is clicked. The parameter has a [block](/guide/blocks/) type, and the call gives a
function by its name, or a block that uses nothing around it (C may call it after the call has
ended):

```kova
extern c from "c" {
    fn calloc(count: Int, size: Int) -> Pointer
    fn free(p: Pointer)
    fn strcpy(into: Pointer, text: String) -> Pointer
    // `strstr(p, "")` is `p` itself: here, a way to read the text at `p`.
    fn text(p: Pointer, nothing: String) -> String from "strstr"
    fn strncmp(a: Pointer, b: Pointer, n: Int) -> Int32
    fn qsort(base: Pointer, count: Int, size: Int, compare: {Pointer, Pointer -> Int32})
}

fn letters(a: Pointer, b: Pointer) -> Int {
    return c.strncmp(a, b, 1)
}

fn main {
    let p: Pointer := c.calloc(16, 1)
    c.strcpy(p, "kova")
    c.qsort(p, 4, 1, letters)
    print(c.text(p, ""))
    c.qsort(p, 4, 1) { a: Pointer, b: Pointer => c.strncmp(b, a, 1) }
    print(c.text(p, ""))
    c.free(p)
}
```

```text
akov
voka
```

The function sees Kova types (an `Int32` is an `Int`). What it needs from the program goes through
the `Pointer` that most C libraries pass back for that purpose (GTK's `data`, Win32's `lParam`).

## C structs

A C struct is declared inside the `extern` block, with its fields in C's order, then used like any
struct: `c.Time { … }`, `t.hour`. Kova lays it out exactly as C does.

```kova
extern c from "c" {
    struct Time {
        second: Int32
        minute: Int32
        hour: Int32
        day: Int32
        month: Int32
        year: Int32
        weekday: Int32
        yearday: Int32
        summer: Int32
        // Linux and macOS add these two; Windows' `tm` stops before them and ignores them.
        offset: Int
        zone: Pointer?
    }
    fn calloc(count: Int, size: Int) -> Pointer
    fn free(p: Pointer)
    fn strftime(into: Pointer, size: Int, format: String, time: Time) -> Int
    fn text(p: Pointer, nothing: String) -> String from "strstr"
}

fn main {
    let t := c.Time { second := 0, minute := 30, hour := 9, day := 1, month := 0, year := 126, weekday := 0, yearday := 0, summer := 0, offset := 0, zone := none }
    let p: Pointer := c.calloc(64, 1)
    c.strftime(p, 64, "%d/%m/%Y %H:%M", t)
    print(c.text(p, ""))
    c.free(p)
}
```

```text
01/01/2026 09:30
```

How a struct crosses:

| Declared | C sees |
| --- | --- |
| `t: Time` | `const struct tm*`: C only reads it |
| `t: var Time` | `struct tm*`: C fills it in, and the variable is read back after the call |
| `t: copy Time` | `struct tm`: the struct itself, by value (Apple's `CGRect`, C's `div_t`) |
| `-> Time?` | a `struct tm*` result, copied (`none` for `NULL`), as `localtime` gives |
| `-> copy Time` | a `struct tm` result, by value |

Fields may be numbers, `Bool`, `Pointer`, `Pointer?`, structs declared before, texts (`String`,
`WideString` and their `?`, lent to C for the call) and functions C calls (a block type, given a
function by its name, as Windows' `WNDCLASSEXW` wants its `lpfnWndProc`). A struct holding a text
cannot be given as `var`, since C could change a text it was only lent.

## Windows' UTF-16

Windows' `…W` functions take UTF-16 texts. `WideString` declares one, and the program keeps
writing ordinary texts:

```kova
extern win from "user32" {
    fn message(window: Pointer?, text: WideString, title: WideString, kind: Int32) -> Int32 from "MessageBoxW"
}

fn main {
    win.message(none, "Bonjour !", "Kova", 0)
}
```

## Running

A program with `extern` runs as native code, even under `kova run` and `kova test`: the interpreter
cannot call C. The debugger stops at the first call into C.

For windows, [`import ui`](/guide/windows/) does all of this for you, on every system.
