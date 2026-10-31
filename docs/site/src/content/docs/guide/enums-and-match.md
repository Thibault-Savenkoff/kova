---
title: Enums and match
description: Types whose value is one of several cases, and how match handles every one.
sidebar:
  order: 6
---

A struct holds **all** its fields at once. An `enum` holds **one** of its cases: a traffic light is
red, orange or green, never two at a time.

```kova
enum Light {
    Red
    Orange
    Green

    fn next(self) -> Light {
        return match self {
            .Red => Light.Green
            .Orange => Light.Red
            .Green => Light.Orange
        }
    }
}

fn main {
    let light := Light.Red
    print(light.next)
    print(light.next.next)
}
```

```text
Light.Green
Light.Orange
```

A case is written `Light.Red`; where the type is already known, as in a `match` on a `Light`, just
`.Red`. An enum may have methods
and static functions, like a struct.

## Cases that carry values

Each case may hold its own fields. A shape is a circle with a radius, a rectangle with two sides,
or a dot with nothing:

```kova
enum Shape {
    Circle(radius: Float)
    Rect(width: Float, height: Float)
    Dot
}

fn area(s: Shape) -> Float {
    return match s {
        .Circle(r) => 3.14159 * r * r
        .Rect(w, h) => w * h
        .Dot => 0.0
    }
}

fn main {
    let shapes: List<Shape> := [Shape.Circle(1.0), Shape.Rect(2.0, 3.0), Shape.Dot]
    for s in shapes {
        print(area(s))
    }
}
```

```text
3.14159
6.0
0.0
```

In a case, `Circle(r)` names the case's fields in their order: `r` is the radius, and exists only
inside that case. `_` stands for a field that is not needed: `Rect(w, _)`.

This is how Kova says "one of these, and here is what each one holds" in one type. Other languages
need a class per case and a type test, or a struct with fields that only mean something for some
values; the compiler then cannot tell which field is valid when.

## Writing less when the type is known

When the enum is already known from where the value goes, the case starts with a dot:

```kova
enum Light {
    Red
    Green
}

fn name(l: Light) -> String {
    return match l {
        .Red => "stop"
        .Green => "go"
    }
}

fn main {
    let l: Light := .Green
    print(name(l))
    print(name(.Red))
    print(l = .Green)
}
```

```text
go
stop
true
```

`.Green` works everywhere a `Light` is expected: a variable with its type, an argument, a `return`, a
field, a list of `Light`, the other side of `=`. Where nothing says which enum it is, such as
`print(.Red)`, Kova asks for the full name.

## Every case is handled

The compiler knows every case of an enum, so it checks that a `match` handles them all:

```kova
enum Shape {
    Circle(radius: Float)
    Rect(width: Float, height: Float)
    Dot
}

fn area(s: Shape) -> Float {
    return match s {
        .Circle(r) => 3.0 * r * r
        .Rect(w, h) => w * h
    }
}

fn main {
    print(area(Shape.Dot))
}
```

```txt
error: cases of `Shape` not handled: `.Dot`
 --> main.kova:8:18
  |
8 |     return match s {
  |                  ^
  = help: add them, or end with `_ => …` for all the others
```

The day a case is added to an enum, every `match` that forgets it is shown, instead of a program
that silently does nothing for it. In the editor, a quick fix writes the missing cases.

`_ => …` handles all the cases not written; use it when they really are all treated the same,
since it also hides the cases added later. Several cases with no fields can share a line:

```kova
fn main {
    let today := Date.year(2026) month(10) day(24)
    match today.weekday {
        .Saturday, .Sunday => print("weekend")
        _ => print("work")
    }
}
```

```text
weekend
```

## `match` gives a value

Like `if`, `match` gives the value of the case taken, as in `area` above. A case may also run a
whole block:

```kova
enum Command {
    Say(text: String)
    Quit
}

fn main {
    let commands: List<Command> := [Command.Say("hello"), Command.Quit, Command.Say("never")]
    for c in commands {
        match c {
            .Say(t) => print(t)
            .Quit => {
                print("bye")
                break
            }
        }
    }
}
```

```text
hello
bye
```

## Enums that hold themselves

A case may hold a value of its own enum. This is how trees and expressions are written:

```kova
enum Expr {
    Num(value: Int)
    Add(left: Expr, right: Expr)
    Mul(left: Expr, right: Expr)
}

fn eval(e: Expr) -> Int {
    return match e {
        .Num(v) => v
        .Add(a, b) => eval(a) + eval(b)
        .Mul(a, b) => eval(a) * eval(b)
    }
}

fn main {
    let e := Expr.Add(Expr.Num(2), Expr.Mul(Expr.Num(3), Expr.Num(4)))
    print(eval(e))
}
```

```text
14
```

Enums can also be generic, `enum Tree<T> { Leaf; Node(left: Tree<T>, value: T, right: Tree<T>) }`
([Generics](/guide/interfaces-and-generics/#generic-types)).

## Changing the fields of a case

The names of a case are read-only. To change the value in place, match it with `var`:

```kova
enum Counter {
    Off
    On(count: Int)
}

fn tick(c: var Counter) {
    match var c {
        .On(n) => n := n + 1
        .Off => {}
    }
}

fn main {
    var c := Counter.On(1)
    tick(var c)
    tick(var c)
    print(c)
}
```

```text
Counter.On(3)
```

Two enums are always there: `T?` (a value or `none`) and `T or E` (a result or an error), the
subject of the [next chapter but one](/guide/errors/).
