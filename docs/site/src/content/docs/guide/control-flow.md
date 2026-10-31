---
title: Choices and loops
description: if, match on values, while and for.
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
    for i in 1..=3 {
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
- `0..=3` includes its end, which suits counting.
- A range whose end is below its start is empty: `3..1` runs nothing. The ends are computed once,
  before the first turn.
- `write` prints without going to the next line; `print` goes to the next line.

To go by another amount, or backwards, add `step` and a written number. A negative `step` needs a
start above the end:

```kova
fn main {
    for i in 0..10 step 3 {
        write(i)
    }
    print("")
    for i in 3..=1 step -1 {
        write(i)
    }
    print("")
}
```

```text
0369
321
```

With two names, `for` also gives each element's place, counted from 0:

```kova
fn main {
    for i, name in ["Ada", "Alan"] {
        print("{i}: {name}")
    }
}
```

```text
0: Ada
1: Alan
```

### Ranges as values, tuples taken apart

Outside a `for`, a range is a `List<Int>` of its numbers, so every list method works on it. In a
`for`, no list is made: `for i in 1..1000000000` costs nothing, while `(1..1000000000).length` would
really write those numbers in memory.

Parentheses around the name take a tuple apart at each turn, with `_` for a part you don't need.
They work in the two-name forms too: `for i, (name, _) in people`.

```kova
fn main {
    let squares: List<Int> := (1..=4).map { x: Int => x * x }
    print(squares)

    let people: List<(String, Int)> := [("Ada", 36), ("Alan", 41)]
    for (name, age) in people {
        print("{name}: {age}")
    }
}
```

```text
[1, 4, 9, 16]
Ada: 36
Alan: 41
```

These parts are copies. To change the elements, write `for x in var l` and use `x.0`.

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
