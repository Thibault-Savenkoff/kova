---
title: Values and types
description: Variables, the basic types, numbers, and why types are written.
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
compiler refuses to change a `let`, and says to declare it with `var` if that was meant.

## Types are written

Every name is introduced with its type: `name: String`, `age: Int`. The compiler checks every use
against it, and guesses almost nothing:

```kova
fn main {
    let x := 5
}
```

```txt
error: the type of `x` must be written
 --> main.kova:2:9
  |
2 |     let x := 5
  |         ^
  = help: write `let x: Int := …`
```

Why write what the compiler could find? Because code is read far more often than it is written,
and a type is the most useful thing to know about a name. Written at the declaration, it is there
for the reader, in a code review, in a diff, in a book, without an editor to ask.

The type may be left out only when its name is already written on the right, so nothing is
hidden: `let rex := Dog { name := "Rex" }`, `let today := Date.today`.

A name cannot be declared again while it is still visible, even in an inner block: two different
`total`s in the same function are a classic source of confusion, so Kova asks for another name.

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
