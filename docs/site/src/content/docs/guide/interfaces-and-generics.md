---
title: Interfaces and generics
description: What several types have in common, code written once for many types, and values of any type that follows an interface.
sidebar:
  order: 10
---

## Interfaces

An **interface** lists methods that several types promise to have. A type follows it by naming it
after `:`, and the compiler checks the promise:

```kova
interface Animal {
    fn sound(self) -> String

    fn twice(self) -> String {
        return "{self.sound} {self.sound}"
    }
}

struct Dog : Animal {
    fn sound(self) -> String {
        return "woof"
    }
}

struct Cat : Animal {
    fn sound(self) -> String {
        return "meow"
    }

    fn twice(self) -> String {
        return "purr"
    }
}

fn main {
    print(Dog {}.twice)
    print(Cat {}.twice)
}
```

```text
woof woof
purr
```

- A method without a body (`sound`) must be written by every type that follows the interface.
- A method with a body (`twice`) is a **default**: a type gets it for free, or writes its own.
- A type may follow several interfaces: `struct Version : Comparable, Display`.

Forgetting a method is found at once, with the method to write:

```kova
interface Animal {
    fn sound(self) -> String
}

struct Dog : Animal {
    name: String
}

fn main {
}
```

```txt
error: `Dog` follows `Animal` but has no method `sound`
 --> main.kova:5:14
  |
5 | struct Dog : Animal {
  |              ^^^^^^
  = help: add `fn sound(self) -> String { … }` (`Self` = this type)
```

Inside an interface, `Self` is "the type that follows it": `fn equals(self, other: Self) -> Bool`.

An interface may also carry [contracts](/guide/contracts-and-tests/): every type following it is
then bound by them.

## Built-in interfaces

Three interfaces are part of the language, and give a type its operators:

| Interface | Method | Gives the type |
| --- | --- | --- |
| `Equal` | `fn equals(self, other: Self) -> Bool` | `=` and `!=`; as a `Map` key or in a `Set` |
| `Comparable` | `fn less(self, other: Self) -> Bool` | `<`, `<=`, `>`, `>=`; `l.sorted` |
| `Display` | `fn to_string(self) -> String` | its text in `"{…}"` and in `print` |

For `Equal`, the method may be left out: the fields are then compared one by one.

```kova
struct Version : Comparable, Display {
    major: Int
    minor: Int

    fn less(self, other: Version) -> Bool {
        return self.major < other.major || (self.major = other.major && self.minor < other.minor)
    }

    fn to_string(self) -> String {
        return "v{self.major}.{self.minor}"
    }
}

fn main {
    let versions: List<Version> := [Version { major := 2, minor := 0 }, Version { major := 1, minor := 9 }, Version { major := 1, minor := 10 }]
    print(versions.sorted)
    print("latest: {versions.sorted[2]}")
}
```

```text
[v1.9, v1.10, v2.0]
latest: v2.0
```

Nothing is given without being asked: a struct is not comparable, nor shown in a text, until it
follows the interface. That way, `a < b` always means an order someone chose.

## Generic functions

A **generic** function is written once for many types. Its type parameters come between `<…>`
after its name, with what they must allow:

```kova
fn larger<T: Comparable>(a: T, b: T) -> T {
    if a < b {
        return b.clone
    }
    return a.clone
}

fn main {
    print(larger(3, 8))
    print(larger("pear", "apple"))
    print(larger(2.5, 1.0))
}
```

```text
8
pear
2.5
```

The types are found from the arguments. `T: Comparable` is required because the function uses
`<`; without it, the compiler refuses `a < b`, since some `T`s cannot be ordered:

```kova
fn larger<T>(a: T, b: T) -> T {
    if a < b {
        return b.clone
    }
    return a.clone
}

fn main {
}
```

```txt
error: `<` cannot order `T` values
 --> main.kova:2:8
  |
2 |     if a < b {
  |        ^^^^^
  = help: `T` must follow `Comparable`
```

The function is checked once, for every `T` at the same time, and compiled once per type it is
used with: generic code is as fast as code written for one type.

## Generic types

Structs and enums take type parameters the same way:

```kova
struct Stack<T> {
    items: List<T>

    fn push(var self, x: move T) {
        self.items.add(move x)
    }

    fn pop(var self) -> T? {
        if self.items.length = 0 {
            return none
        }
        return self.items.remove(self.items.length - 1)
    }
}

fn main {
    var s := Stack<String> { items := [] }
    s.push("a")
    s.push("b")
    print(s.pop)
    print(s.pop)
    print(s.pop)
}
```

```text
"b"
"a"
none
```

`Stack<String>` is a stack of texts; `List`, `Map` and `Set` are generic types of the same kind.
A generic type's static functions are called on the full type: `Stack<Int>.empty`,
`Tree<Int>.Leaf`.

## Adding methods to any type

`extend` adds methods to a type you did not write, whether it comes from the language, from a
library or from another file of your program. Inside, `self` is the value, as in a struct. Adding
`: Describe` makes the type follow an interface, so it can then be given to a generic function or
be held in an `any Describe`:

```kova
interface Describe {
    fn describe(self) -> String
}

extend String {
    fn shout(self) -> String := self.to_upper + "!"
}

extend Int : Describe {
    fn is_even(self) -> Bool := self % 2 = 0
    fn describe(self) -> String := "the number {self}"
}

fn say<T: Describe>(x: T) {
    print(x.describe)
}

fn main {
    print("hello".shout)
    print(4.is_even)
    say(7)
}
```

```text
HELLO!
true
the number 7
```

`extend` works on `String`, `Int`, `Float`, `List<Int>`, `Map<String, Int>`, on your own structs and
on those of another file. A generic struct is extended as a whole, `extend Stack`, and its methods
use its `T`. `extend List<T>` does not exist yet: choose the element type.

It only adds. A method that already exists cannot be replaced, otherwise the same `s.length` would
mean different things from one program to the next:

```kova
extend String {
    fn length(self) -> Int := 0
}

fn main {
    print("a".length)
}
```

```txt
error: `length` already exists on `String`: `extend` adds methods, it cannot change one
 --> main.kova:2:8
  |
2 |     fn length(self) -> Int := 0
  |        ^^^^^^
```

## `any`: values of different types together

A `List<Animal>` cannot exist: `Animal` is not a type, only a promise. To hold a dog and a cat in
the same list, write `any Animal`, a value of **any** type that follows `Animal`:

```kova
interface Animal {
    fn sound(self) -> String
}

struct Dog : Animal {
    name: String

    fn sound(self) -> String {
        return "woof"
    }

    fn fetch(self) -> String {
        return "{self.name} fetches"
    }
}

struct Cat : Animal {
    fn sound(self) -> String {
        return "meow"
    }
}

fn main {
    let zoo: List<any Animal> := [Dog { name := "Rex" }, Cat {}]
    for a in zoo {
        print(a.sound)
        if a is Dog {
            print(a.fetch)
        }
    }
}
```

```text
woof
Rex fetches
meow
```

- On an `any Animal`, only `Animal`'s methods can be called; each value runs its own.
- `a is Dog` tells whether it holds a `Dog`; inside `if a is Dog { … }`, `a` **is** that `Dog`,
  and its own methods and fields can be used.

Use generics when one call works on one type at a time (`larger(3, 8)`), and `any` when values of
different types must live together (`a list of animals`). Generics cost nothing at run time; `any`
looks up the right method at each call.
