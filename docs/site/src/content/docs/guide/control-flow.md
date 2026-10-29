---
title: Choices and loops
description: if, match on values, while, for and from … to.
sidebar:
  order: 3
---

## `if`

```kova
fn main {
    let temperature: Int := 23
    if temperature > 30 {
        print("hot")
    } else if temperature > 15 {
        print("mild")
    } else {
        print("cold")
    }
}
```

```text
mild
```

The condition is a `Bool`, without parentheses; the braces are always written, even for one line,
so adding a second line can never fall outside the `if` by mistake.

`if` also **gives a value**, which replaces the `? :` of other languages:

```kova
fn sign(n: Int) -> String {
    return if n < 0 { "negative" } else if n = 0 { "zero" } else { "positive" }
}

fn main {
    print(sign(-4))
    print(sign(0))
}
```

```text
negative
zero
```

An `if` that gives a value needs an `else`: the value must exist in every case.

## `match` on a value

When a value is compared with several possibilities, `match` lists them, one per line, each
followed by `=>` and what to do. Cases may be numbers, texts, characters, ranges, or several
of them separated by commas; `_` is anything else:

```kova
fn size(n: Int) -> String {
    return match n {
        0 => "none"
        1..=9 => "small"
        10, 20, 30 => "round"
        _ => "big"
    }
}

fn main {
    print(size(4))
    print(size(20))
    print(size(512))
    match "stop" {
        "go" => print("going")
        "stop" => print("stopping")
        _ => print("unknown")
    }
}
```

```text
small
round
big
stopping
```

A `match` must cover every value: without `_`, a `match` on an `Int` is refused, and two cases
that overlap (`1..=9` and `5`) are refused too, since the second one could never be reached.
`match` really shines with [enums](/guide/enums-and-match/), where the compiler knows every case.

## `while`

`while` repeats as long as its condition is `true`:

```kova
fn main {
    var n: Int := 1
    while n < 100 {
        n := n * 3
    }
    print(n)
}
```

```text
243
```

## `for` and ranges

`for` walks through a range of numbers, a [list](/guide/collections/), a map or a set:

```kova
fn main {
    for i in 0..3 {
        write(i)
    }
    print("")
    for i in 0..=3 {
        write(i)
    }
    print("")
    from i := 1 to 3 {
        write(i)
    }
    print("")
    for name in ["Ada", "Alan"] {
        print(name)
    }
}
```

```text
012
0123
123
Ada
Alan
```

- `0..3` stops **before** 3, which suits positions in a list (`0..l.length`).
- `0..=3` and `from i := 1 to 3` include their end, which suits counting.
- A range whose end is below its start is empty: `3..1` runs nothing. The ends are computed once,
  before the first turn.
- `write` prints without going to the next line; `print` goes to the next line.

## `break` and `continue`

`break` leaves the loop at once, `continue` goes to its next turn:

```kova
fn main {
    var total: Int := 0
    for i in 1..=10 {
        if i % 2 = 0 {
            continue
        }
        if i > 7 {
            break
        }
        total += i
    }
    print(total)
}
```

```text
16
```

(1 + 3 + 5 + 7.) `return` leaves the whole function, from inside any loop.
