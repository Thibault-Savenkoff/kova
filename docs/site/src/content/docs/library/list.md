---
title: List
description: List<T>, an ordered list of values, and every method it has.
sidebar:
  order: 4
---

A `List<T>` holds values of type `T` in order, reached by their position from 0. It grows as
needed. The guide's [Lists, Maps and Sets](/guide/collections/) shows how to use them; this page
lists everything they have.

## Making a list

| Written | Gives |
| --- | --- |
| `[1, 2, 3]` | a list of these values |
| `[]` | an empty list, where its type is known (`let l: List<Int> := []`) |
| `List<Int>()` | an empty list, anywhere |
| `l[a..b]`, `l[a..=b]` | a new list with copies of the elements from `a` to `b` (excluded / included) |
| `l.clone` | a copy of the list and of its elements |

## Reading and changing

| Written | Does |
| --- | --- |
| `l.length -> Int` | the number of elements. |
| `l.is_empty -> Bool` | whether the list has no element. |
| `l.first -> T?` | the first element, `none` if the list is empty. |
| `l.last -> T?` | the last element, `none` if the list is empty. |
| `l.contains(x: T) -> Bool` | whether `x` is in the list, compared with `=`. |
| `l.min -> T?` | the smallest element, `none` if the list is empty. |
| `l.max -> T?` | the biggest element, `none` if the list is empty. |
| `l.sum -> T` | the total of a `List<Int>` or `List<Float>`, `0` if the list is empty. |
| `l.get(index: Int) -> T?` | the element at `index`, `none` if there is none there; `l[index]` stops the program instead. |
| `l.take(n: Int) -> List<T>` | a new list of the first `n` elements, fewer if the list is shorter. |
| `l.skip(n: Int) -> List<T>` | a new list without the first `n` elements, empty if the list is shorter. |
| `l.find(x: T) -> Int?` | the position of the first `x`, compared with `=`; `none` if it is not there. |
| `l.reversed -> List<T>` | a new list, in the opposite order. |
| `l[i]` | the element at position `i`. Past the end, the program stops. |
| `l[i] := v` | replaces the element at `i` (`l` must be a `var`). |
| `a = b`, `a != b` | same length and same elements in the same order. |
| `a + b` | a new list: the elements of `a`, then those of `b`, copied. |
| `l += other` | adds a copy of each element of `other` at the end. |
| `l.add(v)` | puts `v` at the end. A value that owns memory is moved in: `l.add(move name)`. |
| `l.remove(i) -> T` | takes the element at `i` out and gives it; the ones after move back by one. |

A list cannot change while a `for` walks through it; `for x in var l` changes each element in
place, `for x in move l` takes them out one by one.

## With a block

| Method | Gives |
| --- | --- |
| `l.each { x: T => … }` | runs the block on each element, in order. |
| `l.filter { x: T => … } -> List<T>` | copies of the elements for which the block gives `true`. |
| `l.map { x: T => … } -> List<U>` | what the block gives for each element, in order. |
| `l.any { x: T => … } -> Bool` | whether the block gives `true` for at least one element (it stops at the first). |
| `l.all { x: T => … } -> Bool` | whether it gives `true` for every element (it stops at the first `false`). |
| `l.fold(start) { acc: U, x: T => … } -> U` | `start`, then the block's value carried from element to element. |
| `l.sorted { a: T, b: T => … } -> List<T>` | a sorted copy, the block telling whether `a` comes before `b`. |
| `l.map workers(n) { … }`, `l.each workers(n) { … }` | the same, on `n` threads ([Several things at once](/guide/concurrency/)). |

```kova
fn main {
    let words: List<String> := ["kiwi", "fig", "banana"]
    print(words.filter { w: String => w.length > 3 })
    print(words.map { w: String => w.length })
    print(words.fold(0) { total: Int, w: String => total + w.length })
    print(words.sorted { a: String, b: String => a.length < b.length })
}
```

```text
["kiwi", "banana"]
[4, 3, 6]
13
["fig", "kiwi", "banana"]
```

## Order and chance

| Method | Gives |
| --- | --- |
| `l.sorted -> List<T>` | a sorted copy, smallest first, for `T` that follows `Comparable` (numbers, texts, dates…). Equal elements keep their order. |
| `l.shuffled -> List<T>` | a copy in a random order. |
| `l.chosen -> T?` | one element drawn at random, `none` if the list is empty. |

`shuffled` and `chosen` use the same chance as `random`, so `KOVA_SEED` makes them repeat. For
what must not be guessed, use [`secure`](/library/secure/).

## Numbers and texts

| Method | On | Gives |
| --- | --- | --- |
| `l.mean -> Float?` | `List<Int>`, `List<Float>` | the average; `none` on an empty list. |
| `l.median -> Float?` | `List<Int>`, `List<Float>` | the middle value once sorted (the mean of the two middle ones for an even length). |
| `l.join(separator: String) -> String` | `List<String>` | the texts with `separator` between each. |

```kova
fn main {
    let marks: List<Int> := [12, 15, 9, 18]
    print(marks.mean)
    print(marks.median)
    print(["a", "b", "c"].join(", "))
}
```

```text
13.5
13.5
a, b, c
```

## Bytes

Bytes are a `List<Int>` of numbers from 0 to 255: there is no other type to learn. These methods
exist on every `List<Int>`, and stop the program with `a byte goes from 0 to 255` if a number
does not fit.

| Method | Gives |
| --- | --- |
| `data.to_text -> String?` | the text these bytes write in UTF-8; `none` if they do not. |
| `data.to_hex -> String` | two hexadecimal digits per byte. |
| `data.to_base64 -> String` | the bytes in base64. |
| `data.to_base64url -> String` | the same, with the alphabet used in addresses and without `=`. |
| `data.sha256 -> String` | the SHA-256 fingerprint, 64 hexadecimal digits. |
| `data.deflate -> List<Int>`, `data.inflate -> List<Int>?`, `data.crc32 -> Int` | packing and checking bytes: see [compress and zip](/library/compress/). |
| `List<Int>.from_hex(code: String) -> List<Int>?` | the bytes back, `none` if `code` is not hexadecimal. |
| `List<Int>.from_base64(code: String) -> List<Int>?` | the bytes back from base64. |
| `List<Int>.from_base64url(code: String) -> List<Int>?` | the same for `to_base64url`. |

`s.to_bytes` goes from a text to its bytes. To change one bit, use the
[bit operators](/library/numbers/); to read and write them, [binary files](/library/files/#binary-files).

```kova
fn main {
    let word: List<Int> := "Kova".to_bytes
    print(word)
    print(word.to_hex)
    print(word.to_text)
    print(List<Int>.from_hex("00ff10"))
    print([255].to_text)
}
```

```text
[75, 111, 118, 97]
4b6f7661
"Kova"
[0, 255, 16]
none
```

## JSON and CSV

`books.to_json` and `List<Book>.from_json(text)` write and read a list as JSON;
`fruits.to_csv` and `List<Fruit>.from_csv(text)` as a table: [JSON and CSV](/library/json-and-csv/).

## Display

`print(l)` shows `[1, 2, 3]`. A list has no text for people, so `"{l}"` is refused: build the text
with `join` or a loop.
