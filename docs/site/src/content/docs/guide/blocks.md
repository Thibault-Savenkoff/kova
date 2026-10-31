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

The two lines are the same program. An enum's variant works the same way: `l.map(Event.Volume)` is
`l.map { v: Int => Event.Volume(v) }`, and `.Volume` is enough where the enum is known, as when a
parameter's type says `{Int -> Event}`. Only the plain case has this short form: as soon as the block
does more, write it.

## What a block cannot do

A block runs inside another function, at a moment chosen by that function. So:

- `return`, `break`, `continue` and `?` cannot be written in a block: they would leave a function
  or a loop that may no longer be running. The block's value is its last line.

## Keeping a function

A block given to a function is lent for that call, and it can change the variables around it. To
keep code for later, put it in a place that has a block type: a variable, a field, a list, a
function's result. Its type is written like a block parameter's, and the block is written the same
way. A function name works too:

```kova
fn double(x: Int) -> Int := x * 2

fn main {
    let twice: {Int -> Int} := double
    let square: {Int -> Int} := { x: Int => x * x }
    print(twice(4))
    print(square(4))
}
```

```text
8
16
```

A kept function takes its own copy of what it uses around it. That is why it can outlive the
function that made it. `adder` has finished when `add3` runs, and `n` is gone, but `add3` holds its
own `n`:

```kova
fn adder(n: Int) -> {Int -> Int} {
    return { x: Int => x + n }
}

fn main {
    let add3: {Int -> Int} := adder(3)
    print(add3(4))
    print(adder(10)(4))
}
```

```text
7
14
```

Since it holds a copy, it sees the value from the moment it was made, and later changes do not
reach it. It cannot change that copy either, otherwise calling `f(1)` twice could give two different
answers with nothing visible at the call:

```kova
fn main {
    var count: Int := 0
    let next: {-> Int} := {
        count += 1
        count
    }
}
```

```txt
error: a function kept in a variable cannot change `count`: it holds its own copy
 --> main.kova:4:9
  |
4 |         count += 1
  |         ^^^^^
  = help: use a `struct` with a `var self` method to keep a state that changes; between threads, a `threads.Shared`
```

A kept function is a value like a `String`: `let b: {Int -> Int} := a` moves it, `a.clone` copies it,
and two of them cannot be compared with `=`.

### Functions in fields and lists

A field holding a function is called like a method. This is how a button remembers what it does,
and how a list of steps is run one after the other:

```kova
struct Button {
    label: String
    on_click: {String -> String}
}

fn main {
    let button: Button := Button { label := "OK", on_click := { who: String => "{who} clicked" } }
    print(button.on_click("Ada"))

    let steps: List<{Int -> Int}> := [{ x: Int => x + 1 }, { x: Int => x * 10 }]
    print(steps[1](5))
    var value: Int := 1
    for step in steps {
        value := step(value)
    }
    print(value)
}
```

```text
Ada clicked
50
20
```

A function without parameters is called by writing its name alone, as everywhere in Kova. Where a
function is expected, the same name gives the function itself instead of calling it.

### Keeping a function that was given

A block parameter is only lent: it may use variables of the caller, so it cannot be kept.

```kova
fn keep(f: {Int -> Int}) -> {Int -> Int} {
    return f
}

fn main {
    print(keep({ x: Int => x })(1))
}
```

```txt
error: `f` is a block lent for this call: it cannot be kept
 --> main.kova:2:12
  |
2 |     return f
  |            ^
  = help: to keep it, write `f: move {…}` in the parameters
```

With `move`, the function belongs to the one who receives it. Combining two functions into a new
one works that way, and generics apply as usual:

```kova
fn then<A, B, C>(f: move {A -> B}, g: move {B -> C}) -> {A -> C} {
    return { x: A => g(f(x)) }
}

fn main {
    let size: {String -> String} := then({ s: String => s.length }, { n: Int => "{n} letters" })
    print(size("Kova"))
}
```

```text
4 letters
```

A kept function can also go wherever a block is expected, without `move`: `[1, 2].map(add3)` lends it
for the call.

## Blocks on several threads

`l.map workers(4) { … }` and `l.each workers(4) { … }` share the elements between 4 threads:
[Doing several things at once](/guide/concurrency/).
