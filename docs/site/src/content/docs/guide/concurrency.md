---
title: Several things at once
description: workers(n) shares work between threads, safely.
sidebar:
  order: 15
---

A program waits a lot: for a site to answer, for a disk, for a long computation. When pieces of
work do not depend on each other, doing them **at the same time**, on several threads, makes the
whole much faster. Kova offers this in one word, `workers(n)`, in the three places where it is
safe and useful.

## On a list

`l.map workers(n) { … }` gives the same list as `l.map { … }`, but shares the elements between `n`
threads:

```kova
import system

fn slow_square(n: Int) -> Int {
    system.sleep(0.2)
    return n * n
}

fn main {
    let numbers: List<Int> := [1, 2, 3, 4, 5, 6, 7, 8]
    let start: Float := system.clock
    let squares: List<Int> := numbers.map workers(8) { n: Int => slow_square(n) }
    print(squares)
    print("{system.clock - start} seconds")
}
```

Built, this takes 0.2 seconds instead of 1.6: the eight waits happen together. The results stay
in the list's order, whatever order the threads finish in.

`l.each workers(n) { … }` runs the block on each element the same way, for its effects (writing
files, asking sites). Its elements are taken in an order that changes from one run to the next, so
two `print`s may come out swapped.

## In a server

`web.serve(8080) workers(8) { q: web.Request => … }` answers up to 8 questions at the same time,
so a slow answer keeps no one else waiting ([the web server](/library/web-server/)).

## Why it is safe

Threads that change the same variable at the same time are the source of the worst bugs there are:
results that are wrong once in a thousand runs, never on the developer's machine. Kova rules them
out with one rule: **a `workers` block may read the variables around it, never change them.**

```kova
fn main {
    var count: Int := 0
    [1, 2, 3].each workers(2) { x: Int =>
        count += x
    }
}
```

```txt
error: `count` cannot change inside `workers(…)`
 --> main.kova:4:9
  |
4 |         count += x
  |         ^^^^^
  = help: several threads run this block at the same time: each would change its own copy
```

Each thread starts with its own copy of what the block reads, so there is nothing shared to lock,
and nothing to forget to lock. To combine results, let `map` gather them, then work on the list it
gives (`squares.fold(0) { … }`).

## What to know

- `kova run` takes one element, or one question, at a time: only the built program uses threads.
  The results are the same either way.
- `workers(1)` works one at a time; `workers(0)` stops the program, since nothing would run.
- `random` draws differently in each thread, so a `random` inside the block gives other numbers
  than one at a time.
- Kova has no `async`/`await`: a blocking call simply waits in its own thread, and the code reads
  in order, as it runs.
