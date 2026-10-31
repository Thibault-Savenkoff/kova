---
title: Numbers, Bool and Char
description: Int, Float, BigInt, Bool and Char, their operators and their methods.
sidebar:
  order: 3
---

## Operators

| Operator | On | Gives |
| --- | --- | --- |
| `+ - *` | `Int`, `Float`, `BigInt` | the sum, difference, product |
| `/` | `Int`, `BigInt` | the quotient, cut towards zero (`7 / 2` is `3`, `-7 / 2` is `-3`) |
| `/` | `Float` | the exact quotient (`7.0 / 2.0` is `3.5`) |
| `%` | `Int`, `BigInt` | the remainder, with the sign of the number divided |
| `-x` | `Int`, `Float`, `BigInt` | the opposite |
| `= != < <= > >=` | numbers, `Char`, `Bool` (`=`, `!=`) | a `Bool` |
| `&& \|\| !` | `Bool` | and, or, not; the right side is computed only when needed |

Both sides of an operator have the same type: `n.to_float * 1.5`, never `n * 1.5`.

A result too large for an `Int` stops the program (`integer overflow`), and so does a division by
zero, for `Int`, `BigInt` and `Float` alike. The program never goes on with a wrong value.

:::caution[A minus before a number]
A method binds more tightly than `-`: `-5.abs` is `-(5.abs)`, so `-5`. Write `(-5).abs` for `5`, and
`(-2.5).floor` for `-3`.
:::

## `Int`

A whole number of 64 bits, from −9,223,372,036,854,775,808 to 9,223,372,036,854,775,807. Written
`42`, `-7`, `1_000_000` (the `_` are ignored), `0xff` (hexadecimal).

| Method | Gives |
| --- | --- |
| `n.abs -> Int` | the value without its sign. |
| `n.min(other: Int) -> Int` | the smaller of the two. |
| `n.max(other: Int) -> Int` | the larger of the two. |
| `n.pow(e: Int) -> Int` | `n` to the power `e`; `e` must not be negative. |
| `n.to_float -> Float` | the same number as a `Float`. |
| `n.to_bigint -> BigInt` | the same number as a `BigInt`. |
| `n.to_char -> Char?` | the character whose Unicode code is `n` (`65.to_char` is `'A'`), `none` if there is none. |
| `n.to_string -> String` | its digits. |

```kova
fn main {
    print((-5).abs)
    print(3.min(8))
    print(2.pow(10))
    print(7.to_float / 2.0)
    print(65.to_char)
}
```

```text
5
3
1024
3.5
'A'
```

## `Float`

A number with a fraction, 64 bits (IEEE 754 `double`), about 15 to 17 significant digits. Written
`3.14`, `2.0`, `1e-3`. A whole number written in the code becomes a `Float` where one is expected:
`let x: Float := 2`.

| Method | Gives |
| --- | --- |
| `x.abs -> Float` | the value without its sign. |
| `x.min(other: Float) -> Float`, `x.max(other: Float) -> Float` | the smaller, the larger. |
| `x.floor -> Int` | the largest whole number below or equal (`(-2.5).floor` is `-3`). |
| `x.ceil -> Int` | the smallest whole number above or equal. |
| `x.round -> Int` | the nearest whole number, halves away from zero (`2.5.round` is `3`). |
| `x.round(digits: Int) -> Float` | the value kept to `digits` decimals (`3.14159.round(2)` is `3.14`). |
| `x.fixed(digits: Int) -> String` | a text with exactly `digits` decimals (`2.5.fixed(2)` is `2.50`): the way to show prices and measures. |
| `x.to_int -> Int` | the whole part, cut towards zero (`(-2.7).to_int` is `-2`). |
| `x.sqrt -> Float` | the square root (`NaN` for a negative number). |
| `x.pow(e: Int) -> Float`, `x.pow(e: Float) -> Float` | `x` to the power `e`. |
| `x.exp -> Float`, `x.ln -> Float`, `x.log10 -> Float` | e to the power `x`, the natural logarithm, the base 10 logarithm. |
| `x.sin`, `x.cos`, `x.tan -> Float` | trigonometry, `x` in radians. |
| `x.asin`, `x.acos`, `x.atan -> Float` | the inverse functions, in radians. |
| `Float.pi -> Float` | π, 3.141592653589793. |
| `x.to_string -> String` | its shortest exact text: `0.1` stays `0.1`. |

`x.floor`, `x.ceil`, `x.round` and `x.to_int` stop the program when the result does not fit in an
`Int`.

```kova
fn main {
    let r: Float := 2.0
    print(Float.pi * r.pow(2))
    print(2.0.sqrt)
    print((Float.pi / 2.0).sin)
    print(1.0.exp)
    print(3.7.round)
}
```

```text
12.566370614359172
1.4142135623730951
1.0
2.718281828459045
4
```

Every one of these gives the same digits on every system, built or run: Kova computes them
itself, correctly rounded where it matters (`exp`, `ln`, `log10`, `pow`), instead of asking the
system, whose libraries differ in the last digit.

## `BigInt`

A whole number without a limit, for what does not fit in 64 bits: factorials, cryptography,
exact counts. A number too large for an `Int`, written in the code, is a `BigInt`; a smaller one
becomes a `BigInt` where one is expected.

| Method | Gives |
| --- | --- |
| `b.to_int -> Int?` | the same number as an `Int`, `none` if it does not fit. |
| `b.to_string -> String` | its digits. |

The operators `+ - * / %`, the comparisons and `-b` work on `BigInt`s, and `n.to_bigint` turns an `Int`
into one.

```kova
fn main {
    var f: BigInt := 1
    for i in 1..=30 {
        f := f * i.to_bigint
    }
    print(f)
    print(f.to_int)
}
```

```text
265252859812191058636308480000000
none
```

## `Bool`

`true` or `false`. `&&`, `||` and `!` combine them; `if` and `while` take them. `b.to_string` is
`"true"` or `"false"`.

## `Char`

One Unicode character, written between single quotes: `'a'`, `'é'`, `'\n'`. A `String` is made of
them (`s.chars`).

| Method | Gives |
| --- | --- |
| `c.to_int -> Int` | its Unicode code (`'A'.to_int` is `65`). |
| `c.to_string -> String` | a text of this one character. |

Characters compare by their codes: `'a' < 'b'`.

## Every number also has

- `x.clone`: a copy (for numbers, the value itself);
- `x.to_string`: the text `print` writes.
