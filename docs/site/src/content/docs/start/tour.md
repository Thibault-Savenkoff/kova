---
title: A tour of Kova
description: The whole language in ten minutes, one small program at a time.
sidebar:
  order: 3
---

This page shows the main ideas of Kova in a few programs. Each section ends with a link to the
chapter of the guide that explains it fully.

## Values

```kova
fn main {
    let name: String := "Ada"
    var age: Int := 36
    age += 1
    print("{name} is {age}")
    print(age = 37)
}
```

```text
Ada is 37
true
```

`let` never changes, `var` may. `:=` gives a value, `=` compares: there is no `==`. Types are
written. [Values and types →](/guide/values/)

## Functions

```kova
fn area(width: Float) by(height: Float) -> Float {
    return width * height
}

fn greeting -> String {
    return "hello"
}

fn main {
    print(area(3.0) by(4.0))
    print(greeting.to_upper)
}
```

```text
12.0
HELLO
```

A name can be cut into parts, `area(…) by(…)`, so a call reads like a sentence. A function without
parameters has no `()`. [Functions →](/guide/functions/)

## Structs and enums

```kova
struct Dog {
    name: String
    age: Int

    fn birthday(var self) {
        self.age += 1
    }
}

enum Size {
    Small
    Large(kilos: Int)
}

fn describe(s: Size) -> String {
    return match s {
        .Small => "small"
        .Large(k) => "large, {k} kg"
    }
}

fn main {
    var rex := Dog { name := "Rex", age := 3 }
    rex.birthday
    print(rex)
    print(describe(Size.Large(30)))
}
```

```text
Dog { name := "Rex", age := 4 }
large, 30 kg
```

A `match` must handle every case of an enum: forgetting one is an error.
[Structs →](/guide/structs/) [Enums →](/guide/enums-and-match/)

## Ownership

```kova
fn add_one(l: var List<Int>) {
    l.add(1)
}

fn main {
    var l: List<Int> := [5]
    add_one(var l)
    print(l)
    let m: List<Int> := l
    print(m)
}
```

```text
[5, 1]
[5, 1]
```

Memory is freed without a garbage collector: each value has one owner. A call that changes a
value says `var` at the call; `let m := l` moves the list to `m`, and `l` can no longer be used.
[Ownership →](/guide/ownership/)

## Missing values and errors

```kova
fn half(n: Int) -> Int or String {
    if n % 2 != 0 {
        return err("{n} is odd")
    }
    return ok(n / 2)
}

fn main {
    match "42".to_int {
        none => print("not a number")
        n => print(half(n))
    }
    print(half(7))
}
```

```text
ok(21)
err("7 is odd")
```

`Int?` may be `none`; `T or E` is a result or an error. The compiler makes sure both are looked at.
[Missing values and errors →](/guide/errors/)

## Collections and blocks

```kova
fn main {
    let scores: List<Int> := [12, 5, 18, 7]
    print(scores.filter { x: Int => x > 10 })
    print(scores.sorted)
    let ages: Map<String, Int> := ["Ana" := 31, "Bo" := 25]
    print(ages["Bo"])
}
```

```text
[12, 18]
[5, 7, 12, 18]
25
```

[Lists, Maps and Sets →](/guide/collections/) [Blocks →](/guide/blocks/)

## Contracts and tests

```kova
fn divide(a: Int, b: Int) -> Int
    require b != 0 else "you cannot divide by zero"
{
    return a / b
}

test "divide rounds down" {
    check divide(7, 2) = 3
}

fn main {
    print(divide(9, 2))
}
```

```text
4
```

A function says what it needs; `kova test` runs the tests.
[Contracts and tests →](/guide/contracts-and-tests/)

## And the library

Files, the web and a web server, JSON and CSV, dates, patterns, encryption, native windows and
calls to C come with Kova: [the library](/library/overview/).
