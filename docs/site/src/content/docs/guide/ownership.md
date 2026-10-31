---
title: Ownership
description: How Kova frees memory without a garbage collector, and what borrow, var and move mean.
sidebar:
  order: 7
---

A program keeps texts, lists and structs in memory, and that memory must be given back once they
are no longer used. Languages do it in one of three ways:

- **by hand** (C): the programmer frees each thing. Forget one and memory leaks; free one twice,
  or use it after, and the program breaks in ways that are hard to find;
- **with a garbage collector** (Java, Python, Go): the language looks for what is no longer
  reachable, now and then. It is safe, but the program pauses, uses more memory, and never knows
  exactly when something goes;
- **with ownership** (Rust, Swift, and Kova): the compiler knows, from the code itself, the moment
  each value stops being used, and frees it there.

Kova takes the third way, in its simplest form. This page explains the four rules it follows; once
they are read, the compiler's messages say the rest.

## Rule 1: every value has one owner

The owner of a value is the variable that holds it. When the variable goes away (at the end of its
block, its function, or a loop's turn), the value is freed with it:

```kova
fn main {
    let greeting: String := "hello"
    if greeting.length > 3 {
        let shout: String := greeting.to_upper
        print(shout)
    }
    print(greeting)
}
```

```text
HELLO
hello
```

`shout` is freed at the `}` of the `if`, `greeting` at the end of `main`. Nothing is written for
it, and nothing waits for a collector: the place is in the compiled code.

## Rule 2: giving a value moves it

Since there is one owner, `let b := a` cannot leave two names on the same text. The value goes to
`b`, and `a` is empty:

```kova
fn main {
    let a: String := "hello"
    let b: String := a
    print(a)
}
```

```txt
error: `a` was moved (line 3)
 --> main.kova:4:11
  |
4 |     print(a)
  |           ^
  = help: use `a.clone` on line 3 if you want to keep a copy
```

This is found when the program is checked, before it runs. Three ways out, depending on what was
meant:

- use `b` from there on: the text simply has a new name;
- keep both with a copy, `let b: String := a.clone`;
- don't take it at all: most of the time a value only needs to be **read**, and reading never
  moves (rule 3).

A value moves when it is given to `let` or `:=`, returned, put inside a struct, a list or a tuple,
or handed to a `move` parameter. Everything else only reads it.

Numbers, `Bool`, `Char` and dates are small: they are copied instead of moved, and
`let b: Int := a` leaves `a` as it was. A struct of your own can be copied the same way by declaring
it `copy struct`, when all its fields are copied too:

```kova
copy struct Point {
    x: Int
    y: Int
}

fn main {
    let a := Point { x := 1, y := 2 }
    let b: Point := a
    print(a)
    print(b)
}
```

```text
Point { x := 1, y := 2 }
Point { x := 1, y := 2 }
```

## Rule 3: a call borrows, unless it says otherwise

Giving a value to a function does not move it. The function **borrows** it: it may read it during
the call, and the value is back with its owner afterwards. When a function must do more, its
parameter says so, and **the call says it too**, so that reading a line is enough to know what
happens to each value:

| Parameter | At the call | The function may | Afterwards |
| --- | --- | --- | --- |
| `s: String` | `shout(word)` | read `s` | `word` is unchanged |
| `l: var List<Int>` | `add_one(var l)` | read and change `l` | `l` holds the changes |
| `l: move List<Int>` | `count(move l)` | keep `l`, free it | `l` can no longer be used |

```kova
fn shout(s: String) -> String {
    return s.to_upper
}

fn add_one(l: var List<Int>) {
    l.add(1)
}

fn count(l: move List<Int>) -> Int {
    return l.length
}

fn main {
    let word: String := "hi"
    print(shout(word))
    print(word)
    var l: List<Int> := [5]
    add_one(var l)
    print(l)
    print(count(move l))
}
```

```text
HI
hi
[5, 1]
2
```

The marker is required even where it could be guessed. A function that changes its argument
without the call saying so is how bugs hide; in Kova it cannot happen:

```kova
fn add_one(l: var List<Int>) {
    l.add(1)
}

fn main {
    var l: List<Int> := [5]
    add_one(l)
}
```

```txt
error: `add_one` may change this argument: write `var l`
 --> main.kova:7:13
  |
7 |     add_one(l)
  |             ^
```

Only a `var` variable can be given as `var`, and a `let` never changes:

```kova
fn main {
    let l: List<Int> := [5]
    l.add(2)
}
```

```txt
error: `l` is declared with let
 --> main.kova:3:5
  |
3 |     l.add(2)
  |     ^
  = help: declare it with `var l` if it must change
```

Methods follow the same rule for the value before the dot. A method that changes it declares
`var self`, and `rex.birthday` then needs `rex` to be a `var`:

```kova
struct Dog {
    name: String
    age: Int

    fn birthday(var self) {
        self.age += 1
    }
}

fn main {
    var rex := Dog { name := "Rex", age := 3 }
    rex.birthday
    print(rex.age)
}
```

```text
4
```

## Rule 4: a borrow never leaves its block

A borrowed value can be read, passed on to another call, copied with `.clone`, but never kept: it
cannot be returned, stored in a struct or added to a list. That is what lets Kova skip the hardest
part of other languages with ownership: there is never a reference that outlives what it points
to, so there is nothing like Rust's lifetimes (`'a`) to write or to understand.

```kova
fn first(l: List<String>) -> String {
    return l[0]
}

fn main {
    let l: List<String> := ["a"]
    print(first(l))
}
```

```txt
error: `l[0]` is an element of a List: it cannot be moved out
 --> main.kova:2:12
  |
2 |     return l[0]
  |            ^^^^
  = help: give a copy with `l[0].clone`, or take it out with `l.remove(0)`
```

`first` only borrowed the list, so it cannot hand one of its texts away. It returns a copy instead
(`return l[0].clone`), or takes the whole list (`l: move List<String>`) and then may give its parts.

The same holds inside a value: a field or an element is part of its owner, and is read, changed or
copied where it is, never moved out alone.

## Changing a value someone is reading

Because each value has one owner and the markers are visible, Kova can also refuse a change while
the same value is being read, the classic source of crashes in C and of
"concurrent modification" errors elsewhere:

```kova
fn main {
    var names: List<String> := ["b", "a"]
    for n in names {
        names.add(n.clone)
    }
}
```

```txt
error: `names` is walked by a `for` loop: it cannot change or be moved inside it
 --> main.kova:4:9
  |
4 |         names.add(n.clone)
  |         ^^^^^
```

Likewise, one call cannot receive the same variable as `var` and as a plain argument:
`both(var l, l)` is refused, and the help suggests giving a copy, `l.clone`.

To change each element of a list, say so in the loop: `for s in var scores { s := s * 10 }`.
`for s in move scores` takes the elements one by one, and the list is used up.

## Why this design

Kova's ownership is the model Swift and Hylo call *mutable value semantics*: a value behaves as if
every variable had its own, and nothing is ever shared behind the programmer's back. Compared with:

- **a garbage collector**: memory is freed at a known place, the program never pauses, and the
  executable carries no collector (a "hello" is about 30 KB);
- **Rust's borrow checker**: references cannot escape their block, so there are no lifetimes and
  no struct holds a borrowed field. Programs that need shared, long-lived links between values
  (a graph, a cache) keep them as indexes or keys into a `List` or a `Map`, which is what Rust
  programmers often end up doing too;
- **C**: none of the four classic memory bugs (leak, double free, use after free, change while
  reading) can be written.

The cost is a few `.clone` calls where a value really must be in two places, and a `var` or `move`
written at the calls that change or take something. Both are visible on purpose: they are the
places to look at when a program is slow or does something unexpected.

## In short

1. One owner per value; it is freed when its owner goes away.
2. `let b := a`, `return a`, putting `a` in a struct or a list moves it; numbers and
   `copy struct`s are copied; `.clone` copies anything.
3. A call borrows. `f(var a)` lets `f` change `a`; `f(move a)` gives `a` away.
4. A borrow stays in its block, so there are no lifetimes.
