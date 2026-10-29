---
title: Lists, Maps and Sets
description: The three collections, how to fill them, read them and walk through them.
sidebar:
  order: 8
---

Kova has three collections, each written with square brackets:

| Type | Holds | Written |
| --- | --- | --- |
| `List<T>` | values in order, reached by position | `[12, 5, 18]` |
| `Map<K, V>` | a value for each key | `["Ana" := 31, "Bo" := 25]` |
| `Set<T>` | values, each at most once | `Set["red", "blue"]` |

The type between `<…>` says what they hold, and everything inside must be of that type. All three
own what they hold: putting a value in [moves](/guide/ownership/) it there, and the collection
frees its elements when it goes away.

## Lists

```kova
fn main {
    var scores: List<Int> := [12, 5, 18]
    scores.add(7)
    print(scores)
    print(scores.length)
    print(scores[0])
    scores[1] := 6
    print(scores.remove(0))
    print(scores)
    print(scores.sorted)
    print(scores[1..3])
}
```

```text
[12, 5, 18, 7]
4
12
12
[6, 18, 7]
[6, 7, 18]
[18, 7]
```

- Positions start at 0. `l[i]` reads, `l[i] := v` replaces.
- A position past the end stops the program (`index out of bounds`, exit code 101) instead of
  reading whatever memory is there.
- `l.add(v)` puts `v` at the end, `l.remove(i)` takes the element at `i` out and gives it.
- `l[1..3]` is a new list with copies of the elements 1 and 2; `l[1..=3]` includes 3.
- `l.sorted` gives a sorted copy; `l.sorted { a: Int, b: Int => a > b }` sorts by your own order.
- An empty list is written `[]`, where its type is known: `let empty: List<Int> := []`.

### Walking through a list

```kova
fn main {
    var prices: List<Int> := [10, 25, 40]
    for p in prices {
        print(p)
    }
    for p in var prices {
        p := p * 2
    }
    print(prices)
    var total: Int := 0
    for p in move prices {
        total += p
    }
    print(total)
}
```

```text
10
25
40
[20, 50, 80]
150
```

- `for p in prices` reads each element; the list cannot change during the loop.
- `for p in var prices` may change each element in place.
- `for p in move prices` takes each element out; the list is used up afterwards.

Lists also take [blocks](/guide/blocks/): `l.filter { x: Int => x > 10 }`,
`l.map { x: Int => x * 2 }`, `l.any`, `l.all`, `l.fold`, `l.each`. Every method is in
[List](/library/list/).

## Maps

A `Map` finds a value from its key, quickly, however many entries it holds:

```kova
fn main {
    var ages: Map<String, Int> := ["Ana" := 31, "Bo" := 25]
    ages["Cy"] := 40
    ages["Bo"] := 26
    print(ages["Bo"])
    print(ages["Zed"])
    print(ages.contains("Ana"))
    for name, age in ages {
        print("{name}: {age}")
    }
}
```

```text
26
none
true
Ana: 31
Bo: 26
Cy: 40
```

- `m[k] := v` adds the key or replaces its value.
- `m[k]` is a `V?`: the key may be missing, and Kova makes you say what to do then (see
  [Missing values](/guide/errors/)), instead of inventing a value or crashing.
- Keys are kept in the order they were first added, so a program always walks a map the same way.
- `m.remove(k)` takes an entry out and gives its value (`none` if there was none);
  `m.keys` and `m.values` give lists.

Counting is the classic use. Since `counts[w]` may be missing, a `match` says what to do in each
case:

```kova
fn main {
    let text: String := "the cat and the dog and the bird"
    var counts: Map<String, Int> := []
    for w in text.words {
        match counts[w] {
            none => counts[w] := 1
            n => counts[w] := n + 1
        }
    }
    print(counts)
}
```

```text
["the" := 3, "cat" := 1, "and" := 2, "dog" := 1, "bird" := 1]
```

Keys may be any type that can be compared with `=`: numbers, texts, dates, and your own types
that follow `Equal`.

## Sets

A `Set` keeps each value once, and tells quickly whether it holds one:

```kova
fn main {
    var seen: Set<String> := Set["red", "blue"]
    seen.add("red")
    seen.add("green")
    print(seen)
    print(seen.contains("blue"))
    print(seen.length)
}
```

```text
Set["red", "blue", "green"]
true
3
```

`Set[…]` is written with its name, so that `[…]` always means a list or a map. Like a map, a set
keeps the order values were first added.

## Comparing collections

`=` is not defined between lists: two lists may be equal element by element, or be the same
list, and Kova does not choose for you. Compare what you mean (`a.length = b.length`, a loop),
or put the list in a struct that follows `Equal`.
