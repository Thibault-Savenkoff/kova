---
title: Values and types
description: Variables, the basic types, numbers, and a type written once.
sidebar:
  order: 1
---

## Variables

A variable is a name for a value. `let` makes one that never changes, `var` one that may:

```kova
fn main {
    let name: String := "Ada"
    var age: Int := 36
    age := age + 1
    age += 1
    print("{name} is {age}")
}
```

```text
Ada is 38
```

Two signs, never confused:

- `:=` **gives** a value: `age := 37`.
- `=` **compares**: `age = 37` is `true` or `false`.

Most languages use `=` to give and `==` to compare, and a single missing `=` then turns a test into
a change (`if (x = 5)` in C). In Kova that mistake cannot be written: `==` does not exist, and `:=`
is not allowed where a value is expected.

`+=`, `-=`, `*=` and `/=` change a `var` in place: `age += 1` is `age := age + 1`.

Prefer `let`: a value that cannot change is one less thing to follow when reading the code. The
compiler refuses to change a `let`, and says to declare it with `var` if that was meant. The other
way round, a `var` that never changes gets a warning (in the editor and from `kova build`) saying it
can be a `let`.

### Names never used

A name declared and never used afterwards is often a mistake: a value computed for nothing, or the
wrong name used further down. The editor greys it out, and `kova build` warns, then builds all the
same:

```console
warning: `i` is never used
 --> app.kova:2:9
  |
2 |     for i in 0..3 {
  |         ^
  = help: remove it, or write `_` in its place if the value must still be computed
```

`_` says "no name here, on purpose": `for _ in 0..3`, `let _: Int := list.remove(0)`, `.Rect(w, _) =>`. A
name that starts with `_` (`_spare`) is not warned about either. Changing a `var` counts as using
it. Parameters are never warned about: a function or a block often has to take what it is given.
`kova run` says nothing, so trying something out stays quiet.

The same goes for a function of the program never called (a file without `fn main` is a module:
its functions are for the files importing it) and for an `import` whose name is never written.

### Constants

A `let` at the top of a file is a constant, for a value that has one meaning in the whole program.
It is made of written values and other constants, and its name is free: `limit` or `LIMIT`.

```kova
let LIMIT: Int := 3
let double: Int := LIMIT * 2

fn main {
    print(LIMIT)
    print(double)
}
```

```text
3
6
```

Under `public:`, another file reads it as `settings.limit`.

## Types are written once

A type is written where a name meets the outside: a parameter (`name: String`), a result (`-> Int`), a field
(`age: Int`), a constant. Inside a function, a `let` takes the type of the value written on the right:

```kova
fn main {
    let x := 5
    var words := "a b".words
    let half := x / 2
    print("{x} {words.length} {half}")
}
```

```text
5 2 2
```

`x` is an `Int`, `words` a `List<String>`: the compiler checks every use against that, as if it was written.
The editor shows it in grey after the name, so a reader never has to guess.

A value that does not say its type must be given one, because there is nothing to find:

```kova
fn main {
    let nothing := print("x")
}
```

```txt
error: `nothing` would hold nothing: this gives no value
 --> main.kova:2:9
  |
2 |     let nothing := print("x")
  |         ^^^^^^^
  = help: call it on a line of its own, without `let`
```

An empty list (`let l: List<Int> := []`) and `none` (`let n: Int? := none`) are written the same way.

A name cannot be declared again while it is still visible, even in an inner block: two different
`total`s in the same function are a classic source of confusion, so Kova asks for another name.

## Naming a long type

Since types are written at the edges of a function, a long one can take a shorter name with `type`. It stays the same type: a
`Grid` and a `List<List<Int>>` are one thing, and the name is known in its file.

```kova
type Grid := List<List<Int>>

fn cells(g: Grid) -> Int {
    return g.fold(0) { sum: Int, row: List<Int> => sum + row.length }
}

fn main {
    let g: Grid := [[1, 2], [3]]
    print(cells(g))
}
```

```text
3
```

## The basic types

| Type | Values | Written |
| --- | --- | --- |
| `Int` | whole numbers, 64 bits (about ±9.2 × 10¹⁸) | `42`, `-7`, `1_000_000`, `0xff` |
| `Float` | numbers with a fraction, 64 bits | `3.14`, `2.0`, `1e-3` |
| `Bool` | `true`, `false` | |
| `Char` | one Unicode character | `'a'`, `'é'` |
| `String` | a text | `"hello"`, `"""raw"""` ([String](/library/string/)) |
| `BigInt` | whole numbers without a limit | `123456789012345678901234567890` |

Dates, lists, maps and the types of your own come in the next chapters.

## Numbers

```kova
fn main {
    print(7 / 2)
    print(-7 / 2)
    print(-7 % 2)
    print(7.0 / 2.0)
    print(0.1 + 0.2)
    print(2.0.sqrt)
    print(3.7.to_int)
    print(2.5.round)
}
```

```text
3
-3
-1
3.5
0.30000000000000004
1.4142135623730951
3
3
```

- `/` between `Int`s gives an `Int`, cut towards zero; `%` is what is left, with the sign of the
  number divided.
- `Float`s follow the usual IEEE 754 rules, so `0.1 + 0.2` is not exactly `0.3`, as in every
  language: the display simply does not hide it.
- `Int` and `Float` never mix by themselves: `n.to_float` and `x.to_int` (cut towards zero),
  `x.round`, `x.floor`, `x.ceil` say what is wanted. A number written in the code becomes a
  `Float` where one is expected (`let x: Float := 2`), but a variable is never converted silently:

```kova
fn main {
    let a: Int := 7
    let b: Float := a
}
```

```txt
error: different types: `Float` expected, `Int` given
 --> main.kova:3:21
  |
3 |     let b: Float := a
  |                     ^
  = help: Kova never converts a variable by itself: use `.to_float` or `.to_int`
```

### No silent overflow

An `Int` that goes past its limit does not wrap around to a negative number, as it does in C, Java
or Go: the program stops and says where. The same for a division by zero, `Int` or `Float`:

```kova
fn main {
    let n: Int := 9223372036854775807
    print(n + 1)
}
```

```txt
error: integer overflow
  --> main.kova:3:13
```

A wrong result that goes unnoticed is worse than a stop. When numbers really are large, `BigInt`
has no limit:

```kova
fn main {
    let x: BigInt := 123456789012345678901234567890
    print(x * x)
}
```

```text
15241578753238836750495351562536198787501905199875019052100
```

### The same digits everywhere

`kova run`, `kova build`, Linux, Windows, macOS, Intel or ARM: a program prints the same digits,
`sin` and `pow` included. Kova carries its own maths instead of using each system's, which
differ in the last digit. A test written on one machine passes on all.

The methods of numbers (`abs`, `min`, `pow`, `sqrt`, `sin`…) are listed in
[Numbers](/library/numbers/).

## Logic

`&&` (and), `||` (or) and `!` (not) combine `Bool`s; the right side of `&&` and `||` is only
computed when needed. Comparisons are `=`, `!=`, `<`, `<=`, `>`, `>=`, on numbers, `Char`s,
texts, and any type that [allows them](/guide/interfaces-and-generics/).

## Tuples

A tuple holds a few values of different types, without giving the group a name. It is handy to
return two results at once:

```kova
fn min_max(a: Int, b: Int) -> (Int, Int) {
    if a < b {
        return (a, b)
    }
    return (b, a)
}

fn main {
    let (low, high): (Int, Int) := min_max(9, 4)
    print(low)
    print(high)
    let pair: (String, Int) := ("Rex", 3)
    print(pair.0)
    var a: Int := 1
    var b: Int := 2
    (a, b) := (b, a)
    print("{a} {b}")
}
```

```text
4
9
Rex
2 1
```

`pair.0`, `pair.1` read the parts. `let (low, high): (Int, Int) := …` takes them apart, and
`(a, b) := (b, a)` swaps two variables: everything on the right is read before anything is
written. When the parts deserve names, make a [struct](/guide/structs/) instead.
