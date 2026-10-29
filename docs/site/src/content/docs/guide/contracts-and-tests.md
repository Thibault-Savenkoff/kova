---
title: Contracts and tests
description: require, ensure and check say what must be true; test blocks prove it.
sidebar:
  order: 12
---

The type of a parameter says what kind of value a function takes, but rarely all it needs: a
divisor must not be zero, a list must not be empty. Kova lets a function write it down, in its
signature, where every reader sees it. This is **design by contract**, as Eiffel introduced it.

## `require`: what a function needs

```kova
fn divide(a: Int, b: Int) -> Int
    require b != 0 else "you cannot divide by zero"
{
    return a / b
}

fn main {
    print(divide(10, 2))
    print(divide(5, 0))
}
```

```txt
error: you cannot divide by zero
  --> main.kova:2:13
  require b != 0
  values: b = 0
  called from main.kova:9:11
```

`require` comes after the signature, before the body. When it is false, the program stops, and the
message shows the condition, the values it read, and **who called** the function: a broken
`require` is the caller's mistake, so that is where to look. The text after `else` is optional; it
may hold values, `"{self.name}: negative years"`.

## `ensure`: what a function promises

`ensure` checks the result when the function returns. `result` is the value it gives:

```kova
fn average(l: List<Int>) -> Int
    require l.length > 0
    ensure result >= 0
{
    var total: Int := 0
    for x in l {
        total += x
    }
    return total / l.length
}

fn main {
    print(average([4, -9]))
}
```

```txt
error: contract violated
  --> main.kova:3:12
  ensure result >= 0
  values: result = -2
  called from main.kova:13:11
```

A broken `ensure` is the function's own mistake: here, it promised more than it does. Either the
promise is wrong (`ensure` should go, or `require` should ask for positive numbers) or the code is.

## `check`: what must be true here

`check` states something that must be true at that point of the code, anywhere:

```kova
fn main {
    let parts: List<String> := "2026-10-29".split("-")
    check parts.length = 3 else "a date has three parts"
    print(parts[0])
}
```

```text
2026
```

## Why write contracts

- **They document.** `require b != 0` says more, and more exactly, than a comment, and it cannot
  go out of date: it is checked.
- **They catch mistakes where they are made.** Without contracts, a wrong value travels through
  the program and breaks something far away. With them, it stops at the first function that
  received it, with the values that broke the rule.
- **They are free when you want them to be.** `kova build --boost` leaves every contract out of
  the executable, for the last bit of speed, once the tests pass. By default they stay in, since
  a program that stops with a clear message is better than one that goes on with a wrong value.

In an [interface](/guide/interfaces-and-generics/), contracts bind every type that follows it. A
type may promise more (an `ensure` of its own), but never ask for more (no `require` of its own
when the interface has one): code written for the interface must keep working with every type.

## Tests

A `test` block, next to the code it checks, holds `check`s:

```kova
fn square(x: Int) -> Int {
    return x * x
}

test "square of 3" {
    check square(3) = 9
}

test "square of a negative" {
    let n: Int := -4
    check square(n) = -16 else "a square is never negative"
}

fn main {
    print(square(5))
}
```

`kova test squares.kova` runs every test of the file:

```console
✅ square of 3
❌ square of a negative
   error: a square is never negative
     --> squares.kova:11:11
     check square(n) = -16
     values: n = -4

1 passed, 1 failed out of 2
```

- Each test runs on its own: one that fails or stops does not stop the others.
- `kova test squares.kova "square of 3"` runs only that one.
- `kova run` and `kova build` leave the tests out: they cost nothing in the program.
- In the editor, a ▶ in the margin runs a test, and ✅ or ❌ shows its result.

`kova test` ends with the exit code 1 when a test fails, so it can stop a build or a CI job.
