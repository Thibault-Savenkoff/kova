---
title: Functions
description: Declaring and calling functions, calls in parts, and default values.
sidebar:
  order: 2
---

## Declaring a function

```kova
fn square(x: Int) -> Int {
    return x * x
}

fn greet(name: String) {
    print("Hello, {name}!")
}

fn main {
    print(square(12))
    greet("Ada")
}
```

```text
144
Hello, Ada!
```

`fn`, the name, the parameters with their types, then `->` and the type of the result, if there is
one. Functions can be declared in any order: `main` may call one written below it.

A function that gives a result must `return` it on every path, and the compiler checks it:

```kova
fn sign(n: Int) -> Int {
    if n > 0 {
        return 1
    }
}

fn main {
    print(sign(2))
}
```

```txt
error: `sign` can get here without a `return`
 --> main.kova:5:1
  |
5 | }
  | ^
  = help: the function must return `Int` in every case
```

Parameters cannot change inside the function: they are what the caller gave. To change a value
of the caller, the parameter says `var`, and so does the call (see
[Ownership](/guide/ownership/)).

## No `()` without arguments

A function without parameters is declared and called without parentheses:

```kova
fn seven -> Int {
    return 7
}

fn main {
    print(seven)
    print(seven.to_float)
}
```

```text
7
7.0
```

The same goes for methods: `name.length`, `rex.sound`, `x.to_float`. This comes from Eiffel and
Smalltalk: whether `seven` is a stored value or a computed one is a detail of how it is made, and
the code that uses it should not have to change when that detail does. `seven()` is an error, and
the help says to write `seven`. The one exception is a generic type built empty, which keeps its
`()`: `List<Int>()`.

So that a bare name can always be read the same way, no variable may take the name of a function
without parameters.

## Calls in parts

When a function takes several things, its name can be cut into **parts**, each naming what follows
it, so that the call reads like a sentence:

```kova
fn between(x: Int, low: Int) upto(high: Int) -> Bool {
    return low <= x && x <= high
}

fn main {
    print(between(5, 1) upto(10))
    print(between(12, 1) upto(10))
}
```

```text
true
false
```

The function's full name is `between(…) upto(…)`; each part has its parameters. The library uses
this everywhere a plain list of arguments would be ambiguous: `files.copy(a) to(b)`,
`s.replace_all(pattern) with(text)`, `secure.encrypt(text) key(k)`. Again this is Smalltalk's
idea (`dict at: 1 put: x`): at the call, it is clear which argument is which, without looking at
the declaration.

## Default values

A parameter of a part may have a default value. A part whose parameters all have one may then be
left out of the call:

```kova
fn connect(host: String) port(p: Int := 80) secure(on: Bool := false) -> String {
    return "{host}:{p} {on}"
}

fn main {
    print(connect("example.com"))
    print(connect("example.com") port(8080))
    print(connect("example.com") secure(true))
}
```

```text
example.com:80 false
example.com:8080 false
example.com:80 true
```

Only whole parts are left out, and the first part is always written: a call never depends on the
position of an argument that is not there.

## Several results

A function returns one value, but that value may be a [tuple](/guide/values/#tuples) or a
[struct](/guide/structs/):

```kova
fn divide(a: Int, b: Int) -> (Int, Int) {
    return (a / b, a % b)
}

fn main {
    let (q, r): (Int, Int) := divide(17, 5)
    print("17 = 5 × {q} + {r}")
}
```

```text
17 = 5 × 3 + 2
```

A function that may fail returns `T or E`, and one that may have nothing to give returns `T?`:
[Missing values and errors](/guide/errors/) explains both.

## Recursion

A function may call itself:

```kova
fn fib(n: Int) -> Int {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}

fn main {
    print(fib(30))
}
```

```text
832040
```

## What a function is not

Functions are not values in Kova: they are not stored in variables or in lists. To give some code
to a function, write a [block](/guide/blocks/), which is made for that. This keeps every call
readable: a name followed by arguments is always that function, never a variable that may hold
another.
