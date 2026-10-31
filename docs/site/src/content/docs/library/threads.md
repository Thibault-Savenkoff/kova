---
title: threads
description: Threads, channels and values shared between threads.
sidebar:
  order: 15
---

```kova
import threads
```

[Several things at once](/guide/concurrency/) explains how to use them together. A program using
`threads` runs as native code, in `kova run` and `kova test` too.

## threads.spawn

| Function | Does |
| --- | --- |
| `threads.spawn { … }` | runs the block on a new thread, gives a `threads.Task<T>`, `T` being the block's value |
| `threads.spawn(f)` | the same with a function without parameters |

The block works on its own copies of what it reads and cannot change what is around it.

## threads.Task

| Method | Does |
| --- | --- |
| `t.wait` | waits for the end of the thread and gives its value; `t` is used up |

## threads.Channel

| Written | Does |
| --- | --- |
| `threads.Channel<T>.new` | a new channel, without a limit |
| `threads.Channel<T>.limited(n)` | a new channel holding `n` values at most (`n` of 1 or more) |
| `c.send(x)` | gives `x` to the channel; waits for room in a limited one; an error on a closed one |
| `c.receive` | waits and takes the oldest value: a `T?`, `none` once closed and empty |
| `c.close` | no more values: the ones inside can still be received |
| `for x in c { … }` | receives until the channel is closed and empty; each `x` is the loop's |

A copy of a channel (`c.clone`, or using it in a `spawn` block) is the same channel.

## threads.Shared

| Written | Does |
| --- | --- |
| `threads.Shared<T>.new(value)` | a value several threads share |
| `s.read` | a copy of the value |
| `s.change { x: var T => … }` | lends the value to the block, one thread at a time; gives the block's value, if any |

A copy of a `Shared` is the same value. A `change` inside a `change` of the same value waits for
itself: Kova stops the program if no other thread can run.
