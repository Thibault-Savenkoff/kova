---
title: Map and Set
description: Map<K, V>, a value for each key, and Set<T>, values each kept once.
sidebar:
  order: 5
---

Both find a key quickly, whatever their size (they use a hash table), and both keep the order in
which keys were first added, so a program walks them the same way at every run.

Their keys can be any type that can be compared with `=`: numbers, `Char`, `String`, dates,
tuples of them, and types that follow [`Equal`](/guide/interfaces-and-generics/#built-in-interfaces).

## `Map<K, V>`

| Written | Does |
| --- | --- |
| `["a" := 1, "b" := 2]` | a map of these entries; `[]` is an empty one where the type is known |
| `m.length -> Int` | the number of entries. |
| `m.is_empty -> Bool` | whether the Map has no entry. |
| `m[k] -> V?` | the value of `k`, `none` if there is none. |
| `m[k] := v` | gives `k` the value `v`, adding the key if needed (`m` must be a `var`). |
| `m.contains(k: K) -> Bool` | whether `k` has a value. |
| `m.remove(k: K) -> V?` | takes the entry of `k` out and gives its value, `none` if there was none. |
| `m.keys -> List<K>` | copies of the keys, in order. |
| `m.values -> List<V>` | copies of the values, in the same order. |
| `for k, v in m { … }` | walks through the entries in order. |
| `m.clone -> Map<K, V>` | a copy. |

```kova
fn main {
    var stock: Map<String, Int> := ["apple" := 12, "pear" := 4]
    stock["plum"] := 30
    stock["pear"] := 5
    print(stock["pear"])
    print(stock["kiwi"])
    print(stock.remove("apple"))
    print(stock.keys)
    for fruit, n in stock {
        print("{fruit}: {n}")
    }
}
```

```text
5
none
12
["pear", "plum"]
pear: 5
plum: 30
```

`m[k]` is a `V?`, so it cannot be computed with directly: `m[k] += 1` is refused, since the key may
be missing. Read it, then decide:

```kova
fn main {
    var counts: Map<Char, Int> := []
    for c in "banana".chars {
        match counts[c] {
            none => counts[c] := 1
            n => counts[c] := n + 1
        }
    }
    print(counts)
}
```

```text
['b' := 1, 'a' := 3, 'n' := 2]
```

A `Map<String, T>` is also read from and written to JSON objects:
[JSON and CSV](/library/json-and-csv/).

## `Set<T>`

| Written | Does |
| --- | --- |
| `Set["a", "b"]` | a set of these values (a value written twice is kept once); `Set[]` is empty |
| `s.length -> Int` | the number of values. |
| `s.is_empty -> Bool` | whether the Set has no value. |
| `s.add(v)` | adds `v`, unless it is already there. |
| `s.contains(v: T) -> Bool` | whether `v` is there. |
| `s.remove(v: T) -> Bool` | takes `v` out; `true` if it was there. |
| `for v in s { … }` | walks through the values, in the order they were added. |
| `s.clone -> Set<T>` | a copy. |

```kova
fn main {
    var tags: Set<String> := Set["kova", "docs"]
    tags.add("kova")
    tags.add("web")
    print(tags)
    print(tags.contains("docs"))
    print(tags.remove("docs"))
    print(tags.length)
}
```

```text
Set["kova", "docs", "web"]
true
true
2
```
