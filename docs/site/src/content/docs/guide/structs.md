---
title: Structs and methods
description: Types of your own, their fields, their methods and their static functions.
sidebar:
  order: 5
---

A `struct` groups named values, its **fields**, into a type of your own:

```kova
struct Dog {
    name: String
    age: Int
}

fn main {
    var rex := Dog { name := "Rex", age := 3 }
    rex.age += 1
    print(rex.name)
    print(rex)
}
```

```text
Rex
Dog { name := "Rex", age := 4 }
```

- `Dog { name := "Rex", age := 3 }` makes a value: every field is given, in any order. A missing
  one is an error, unless it has a value of its own, since a field never holds "nothing" by
  accident (a field that may be empty is declared `T?`, see [Missing values](/guide/errors/)).
- `rex.age` reads a field; on a `var`, `rex.age := 5` changes it.
- Type names start with a capital, field names with a small letter.
- Since the type's name is written on the right, `let rex := Dog { … }` needs no `: Dog`.

### Shorter ways to write a value

A field may have a value of its own, used when a literal leaves it out. A variable with the field's
name is written once. `...` takes the other fields from another value:

```kova
struct Dog {
    name: String
    age: Int := 1
}

fn main {
    let rex: Dog := Dog { name := "Rex" }
    print(rex)
    let name: String := "Fido"
    let fido: Dog := Dog { name, age := 3 }
    print(fido)
    let older: Dog := Dog { ...fido, age := 4 }
    print(older)
}
```

```text
Dog { name := "Rex", age := 1 }
Dog { name := "Fido", age := 3 }
Dog { name := "Fido", age := 4 }
```

`...fido` copies its fields, so `fido` stays usable. A field's own value reads no variable.

## Methods

Functions written inside the struct are its **methods**. They reach the value through `self`, and
say what they do with it, like any [parameter](/guide/ownership/#rule-3-a-call-borrows-unless-it-says-otherwise):

| Written | The method may | Called on |
| --- | --- | --- |
| `fn summary(self)` | read the value | any value |
| `fn deposit(var self, …)` | change the value | a `var` |
| `fn close(move self)` | take the value (it is used up) | a value that is not needed afterwards |

A function inside the struct without `self` is a **static function**, called on the type itself:
`Account.open("Ada")`. It is the usual way to give a type a constructor with a name that says what
it does.

```kova
struct Account {
    owner: String
    balance: Int

    fn open(owner: move String) -> Account {
        return Account { owner := owner, balance := 0 }
    }

    fn deposit(var self, amount: Int) {
        self.balance += amount
    }

    fn summary(self) -> String {
        return "{self.owner}: {self.balance}"
    }

    fn close(move self) -> String {
        return "closed, {self.balance} given back to {self.owner}"
    }
}

fn main {
    var account: Account := Account.open("Ada")
    account.deposit(50)
    print(account.summary)
    print(account.close)
}
```

```text
Ada: 50
closed, 50 given back to Ada
```

After `account.close`, `account` is gone: using it again is an error, as for any moved value.

`open` takes `owner: move String` because it keeps the text inside the new account. With a plain
`owner: String`, the text would only be borrowed, and a borrowed value cannot be stored:

```kova
struct Account {
    owner: String

    fn open(owner: String) -> Account {
        return Account { owner := owner }
    }
}

fn main {
    let a: Account := Account.open("Ada")
}
```

```txt
error: `owner` is borrowed here: it cannot be moved
 --> main.kova:5:35
  |
5 |         return Account { owner := owner }
  |                                   ^^^^^
  = help: give a copy: `owner.clone`
```

A method without parameters is called without `()`, like a field: `account.summary`. Whoever
uses the type does not need to know which one it is.

## Operators on your types

`+`, `-`, `*`, `/` and `%` work on your own types when the type has a method of that name. The
method takes `self` and one parameter, and `a + b` calls it on `a`, so the value on the left chooses
the method. `+=` and the others come with it: `total += a` is `total := total + a`.

```kova
struct Vector {
    x: Float
    y: Float

    fn +(self, other: Vector) -> Vector := Vector { x := self.x + other.x, y := self.y + other.y }
    fn *(self, k: Float) -> Vector := Vector { x := self.x * k, y := self.y * k }
}

fn main {
    let a := Vector { x := 1.0, y := 2.0 }
    let b := Vector { x := 3.0, y := 4.0 }
    print(a + b)
    print((a + b) * 2.0)

    var total := Vector { x := 0.0, y := 0.0 }
    total += a
    total += b
    print(total)
}
```

```text
Vector { x := 4.0, y := 6.0 }
Vector { x := 8.0, y := 12.0 }
Vector { x := 4.0, y := 6.0 }
```

`-v` calls `fn -(self)`, a method without a parameter that lives beside `fn -(self, other: T)`.

When the value on the left is a number, the method goes on the number's type with
[`extend`](/guide/interfaces-and-generics/#adding-methods-to-any-type). Kova calls it when the right
side has the type it takes; between two numbers, `*` stays the usual one.

```kova
struct Vector {
    x: Float
    y: Float

    fn -(self) -> Vector := Vector { x := -self.x, y := -self.y }
    fn *(self, k: Float) -> Vector := Vector { x := self.x * k, y := self.y * k }
}

extend Float {
    fn *(self, v: Vector) -> Vector := v * self
}

fn main {
    let v := Vector { x := 1.0, y := 2.0 }
    print(-v)
    print(v * 2.0)
    print(2.0 * v)
    print(2.0 * 3.0)
}
```

```text
Vector { x := -1.0, y := -2.0 }
Vector { x := 2.0, y := 4.0 }
Vector { x := 2.0, y := 4.0 }
6.0
```

The parameter need not be of the same type: `Vector * Float` scales a vector. One operator is one
method, and Kova has no overloading, so a type cannot have both `Vector * Float` and
`Vector * Vector`: give the second one a name, like `v.dot(w)`.

`=` and `<` are not written this way: they come from [`Equal` and `Comparable`](#comparing-and-showing).
An interface can ask for an operator, which lets a generic function use it:
`interface Addable { fn +(self, other: Self) -> Self }`.

## Comparing and showing

A struct can be compared with `=` and `<`, or shown inside a text, only when it says so, by
following the built-in [interfaces](/guide/interfaces-and-generics/#built-in-interfaces):

```kova
struct Point : Equal {
    x: Int
    y: Int
}

fn main {
    let a := Point { x := 1, y := 2 }
    print(a = Point { x := 1, y := 2 })
}
```

```text
true
```

`: Equal` compares the fields one by one; a struct may also write its own `equals`. Without it,
`a = b` is refused, and the help says to add `: Equal`. Kova does not decide for you what makes
two values of your type the same.

## Values that are copied

A struct is moved like a `String`. When it only holds numbers, `Bool`s, `Char`s and other copied
types, it may be declared `copy struct`: it is then copied wherever it is given, like an `Int`.

```kova
copy struct Size {
    width: Int
    height: Int
}

fn main {
    let a := Size { width := 3, height := 4 }
    let b: Size := a
    print(a.width * b.height)
}
```

```text
12
```

## Fields seen from other files

Inside one file, every field can be read. In a file that others [import](/guide/modules/), only
what is written under `public:` is seen from outside, fields included, so a type can keep its
inside to itself and offer methods instead.
