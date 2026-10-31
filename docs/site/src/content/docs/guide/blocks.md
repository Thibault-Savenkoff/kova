---
title: Blocks
description: Pieces of code given to a function, which runs them.
sidebar:
  order: 11
---

A **block** is a piece of code given to a function, which runs it when it wants, as often as it
wants. It is written between braces, with its parameters before `=>`:

```kova
fn main {
    let scores: List<Int> := [12, 5, 18, 7]
    print(scores.filter { x: Int => x > 10 })
    print(scores.map { x: Int => x * 2 })
    print(scores.any { x: Int => x > 15 })
    print(scores.all { x: Int => x > 0 })
    print(scores.fold(0) { total: Int, x: Int => total + x })
}
```

```text
[12, 18]
[24, 10, 36, 14]
true
true
42
```

- `filter` keeps the elements for which the block gives `true`.
- `map` gives a list of what the block gives for each element.
- `any` and `all` tell whether the block gives `true` for one element, or for all of them.
- `fold(start)` carries a value from element to element: here, a running total.
- `each` runs the block on each element; `sorted { a: Int, b: Int => … }` sorts with the block as the order.

The last line of a block is its value. A block that gives nothing, as with `each`, may hold any
statements.

## Blocks see what is around them

A block can read the variables around it, and change them:

```kova
fn main {
    let limit: Int := 10
    var count: Int := 0
    [12, 5, 18, 7].each { x: Int =>
        if x > limit {
            count += 1
        }
    }
    print(count)
}
```

```text
2
```

## Writing a function that takes a block

A parameter of block type is written with the types of the block's parameters, and its result after
`->` if it gives one: `{Int}` takes an `Int` and gives nothing, `{Int -> Bool}` gives a `Bool`. The
function calls it like a function:

```kova
fn repeat(n: Int, body: {Int}) {
    for i in 1..=n {
        body(i)
    }
}

fn keep(l: List<Int>, wanted: {Int -> Bool}) -> List<Int> {
    var out: List<Int> := []
    for x in l {
        if wanted(x) {
            out.add(x)
        }
    }
    return out
}

fn main {
    repeat(3) { i: Int =>
        print("turn {i}")
    }
    print(keep([1, 8, 3, 9]) { x: Int => x > 5 })
}
```

```text
turn 1
turn 2
turn 3
[8, 9]
```

When the block is the last argument, it is written after the call's parentheses, on the same line:
`repeat(3) { … }` rather than `repeat(3, { … })`. Both are accepted. That is what makes
`web.serve(8080) { q: web.Request => … }` read like a statement of the language, while it is a
function of the library.

## Giving a function instead of a block

Where a block is expected, the name of a function does the same work, if its parameters are the
block's. That keeps a calculation with a name of its own, usable anywhere:

```kova
fn double(x: Int) -> Int := x * 2

fn main {
    let l: List<Int> := [1, 2, 3]
    print(l.map(double))
    print(l.map { x: Int => double(x) })
}
```

```text
[2, 4, 6]
[2, 4, 6]
```

The two lines are the same program. A function by its name is only given to a call, like a block: it
never goes into a variable.

## What a block cannot do

A block runs inside another function, at a moment chosen by that function. So:

- `return`, `break`, `continue` and `?` cannot be written in a block: they would leave a function
  or a loop that may no longer be running. The block's value is its last line;
- a block is only written as an argument. It is never stored in a variable or a struct, so it can
  never outlive the variables it uses:

```kova
fn main {
    let double := { x: Int => x * 2 }
}
```

```txt
error: a block `{ … }` is only written as a function's argument: `l.each { x: Int => print(x) }`
 --> main.kova:2:19
  |
2 |     let double := { x: Int => x * 2 }
  |                   ^
```

These two rules keep blocks simple to follow: a block is always run during the call it is given
to, and when the call ends, the block is gone. When code must be kept and chosen later, an
[enum](/guide/enums-and-match/) listing the possible actions, with a `match`, says the same thing
more clearly.

## Blocks on several threads

`l.map workers(4) { … }` and `l.each workers(4) { … }` share the elements between 4 threads:
[Doing several things at once](/guide/concurrency/).
