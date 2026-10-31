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

## Threads of your own

`workers` shares out one piece of work. When the work has several different parts, or a part must
go on while the rest of the program does something else, start a thread yourself with
[`threads.spawn`](/library/threads/). It takes a block, runs it on a new thread at once, and gives
a `Task`: `wait` waits for the end of the block and gives its value.

```kova
import threads

fn slow_sum(n: Int) -> Int {
    var sum: Int := 0
    for i in 1..=n {
        sum += i
    }
    return sum
}

fn main {
    let first: threads.Task<Int> := threads.spawn { slow_sum(1000) }
    let second: threads.Task<Int> := threads.spawn { slow_sum(2000) }
    print(first.wait + second.wait)
}
```

```text
2501500
```

The block runs on its own copies of what it reads, like a [function kept in a
variable](/guide/blocks/#keeping-a-function), so it cannot change a variable around it. Its last
line is its value; a block without one gives a `threads.Task<Nothing>`, and `wait` then gives
nothing.

A program ends when all its threads have, even if nobody called `wait`.

## Passing values: channels

A `threads.Channel<T>` is a pipe between threads. `send` puts a value in at one end and `receive`
takes the oldest one out at the other, waiting if there is none. A `for` loop receives until the
channel is closed.

To build a team of workers, start them first, each looping over the channel of jobs. Then send the
jobs, and call `close` to say there are no more. Each worker's loop ends when the channel is empty,
and `wait` tells you they are all done.

```kova
import threads

fn main {
    let jobs := threads.Channel<Int>.new
    let results := threads.Channel<Int>.new
    var team: List<threads.Task<Nothing>> := []
    for i in 0..3 {
        team.add(threads.spawn {
            for n in jobs {
                results.send(n * n)
            }
        })
    }
    for n in 1..=5 {
        jobs.send(n)
    }
    jobs.close
    for member in move team {
        member.wait
    }
    results.close

    var total: Int := 0
    for square in results {
        total += square
    }
    print(total)
}
```

```text
55
```

Three things make this hold together:

- A copy of a channel is **the same pipe**. That is how the block above reaches it: its copy of
  `jobs` and yours are the two ends.
- `send` **gives** its value away, so a value is never in two threads at once.
- `close` is the signal. Without it the workers would wait for ever, and Kova would stop the program
  and say so (below).

`threads.Channel<T>.new` holds as many values as you send. If senders may run ahead of receivers,
`threads.Channel<T>.limited(10)` holds 10 at most, and `send` waits for room.

## Waiting for several channels: `threads.select`

`c.receive` waits for one channel. A thread that listens to several, or that must not wait for
ever, uses `threads.select`: it takes the first value that comes, whichever channel it comes from,
and runs that case.

```kova
import threads

fn main {
    let orders := threads.Channel<Int>.new
    let notes := threads.Channel<String>.new
    orders.send(12)
    notes.send("call back")
    notes.close
    orders.close
    while true {
        threads.select {
            order from orders => print("order {order}")
            note from notes => print("note: {note}")
            after(5.0) => print("nothing for five seconds")
            closed => break
        }
    }
}
```

```text
order 12
note: call back
```

Each case says what happens:

- `name from channel`: a value came, and `name` holds it. It is yours, as with `for x in c`. The
  channels may hold different types.
- `after(seconds)`: nothing came for that long. This case is optional.
- `closed`: every channel is closed and empty, so nothing will ever come. It is required, because
  a loop around `select` must end somewhere: most of the time you write `closed => break`.

A channel that is closed while the others are not is simply left out. When several channels have a
value, the first one written is taken first.

To wait for one channel for a limited time, `c.receive_within(2.5)` is shorter. It gives `none` when
nothing came in time, and also when the channel is closed; use `select` to tell the two apart.

## Sharing a value: `threads.Shared`

When threads must all see one value, a counter or a cache, put it in a `threads.Shared<T>`. A copy
of it is the same value, as with a channel. `change` lends the value to a block, with no other
thread able to touch it until the block ends:

```kova
import threads

fn main {
    let count := threads.Shared<Int>.new(0)
    var team: List<threads.Task<Nothing>> := []
    for i in 0..4 {
        team.add(threads.spawn {
            for k in 0..1000 {
                count.change { n: var Int => n += 1 }
            }
        })
    }
    for member in move team {
        member.wait
    }
    print(count.read)
}
```

```text
4000
```

`n: var Int` is written because the block changes what it borrows. `change` gives the value of the
block, if it has one, which lets a thread read the number it just made. `read` gives a copy of the
value.

Two lessons from other languages are built in. There is no lock to forget: the value can only be
reached through `change` and `read`. And the block cannot keep the value, since it is only lent.

Prefer a channel when one thread can own the work, and `Shared` for what really is common: a
`Shared` makes threads queue for it.

## When every thread waits

A thread that waits for a value nobody will send would hang the program in silence. Kova counts the
threads that sleep on a channel, a `Task` or a `Shared`. When all of them are asleep, nothing can
wake them, so the program stops with an error at the line that waited:

```kova
import threads

fn main {
    let c := threads.Channel<Int>.new
    print(c.receive)
}
```

```txt
error: every thread is waiting, and nothing can wake them
  --> main.kova:5:13
```

The usual cause is a channel never closed. A thread asleep in `system.sleep` or waiting for the
network is not counted: it will wake up by itself.

## What to know

- A program using `workers` or `threads` runs as native code in `kova run` and `kova test` too.
  `kova debug` takes one element, or one question, at a time with `workers`; the results are the
  same.
- `workers(1)` works one at a time; `workers(0)` stops the program, since nothing would run.
- `random` draws differently in each thread, so a `random` inside the block gives other numbers
  than one at a time.
- Kova has no `async`/`await`: a blocking call simply waits in its own thread, and the code reads
  in order, as it runs.
