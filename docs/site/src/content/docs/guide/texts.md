---
title: Texts
description: Strings, values inside texts, raw texts and characters.
sidebar:
  order: 4
---

A `String` holds any text, in any script, and `{…}` puts a value inside it:

```kova
fn main {
    let name: String := "Ada"
    let year: Int := 1843
    print("{name} wrote the first program in {year}.")
    print("In {2026 - year} years, {name.length} letters.")
}
```

```text
Ada wrote the first program in 1843.
In 183 years, 3 letters.
```

Between the braces goes a name, a call, a calculation: any expression without quotes or braces
of its own. Its value must have a **text for people**: numbers, `Bool`, `Char`, `String`, dates,
and the types that follow [`Display`](/guide/interfaces-and-generics/#built-in-interfaces), which
write their own `to_string`. A list or a struct has no such text by itself, since there is no one
right way to show it to a user:

```kova
struct Dog {
    name: String
}

fn main {
    let rex := Dog { name := "Rex" }
    print("my dog: {rex}")
}
```

```txt
error: `rex` (`Dog`) has no text for people
 --> main.kova:7:21
  |
7 |     print("my dog: {rex}")
  |                     ^^^
  = help: write `struct Dog : Display` and its method `fn to_string(self) -> String`
```

`print(rex)` alone, on the other hand, shows any value as it is, fields and all: that is for the
programmer, while debugging.

```kova
struct Dog {
    name: String
}

fn main {
    print(Dog { name := "Rex" })
    print([1, 2, 3])
    print("12".to_int)
}
```

```text
Dog { name: "Rex" }
[1, 2, 3]
12
```

## Special characters

| Written | Gives |
| --- | --- |
| `\n` | a line break |
| `\t` | a tab |
| `\"` | `"` |
| `\\` | `\` |
| `\{` | `{` without starting a value |

## Raw texts

Between `"""`, a text is **raw**: braces, quotes and backslashes are kept as written. It is the
way to write JSON, HTML, Windows paths or [patterns](/library/string/#patterns):

```kova
fn main {
    print("""{"title": "Dune"}""")
    print("""C:\Users\ada""")
    let page: String := """
        <ul>
          <li>one</li>
        </ul>
        """
    print(page)
}
```

```text
{"title": "Dune"}
C:\Users\ada
<ul>
  <li>one</li>
</ul>
```

Over several lines, the text starts on the line after the opening `"""`. The closing `"""`, alone
on its line, says how much indentation to take off every line, so the text can be indented with
the code around it.

## Texts are values

A `String` owns its characters: giving it to another variable [moves](/guide/ownership/) it, and
`.clone` copies it. Its methods never change it in place: `s.to_upper` gives a new text and `s`
stays as it was.

```kova
fn main {
    let s: String := "  Hello, World  "
    let clean: String := s.trim.to_lower
    print(clean)
    print(clean.split(", "))
    print(clean.replace("world", "Kova"))
    print(clean.starts_with("hello"))
}
```

```text
hello, world
["hello", "world"]
hello, Kova
true
```

Positions count characters, not bytes: `"héllo".length` is 5 and `"héllo"[1..3]` is `"él"`.

## Characters

A `Char` is one character, between single quotes: `'a'`, `'é'`, `'\n'`. `s.chars` gives the
characters of a text, `c.to_int` a character's code and `n.to_char` the character of a code (a `Char?`: not every
number is one). `print` shows a `Char` inside another value with its quotes.

```kova
fn main {
    let codes: List<Int> := "Kova".chars.map { c: Char => c.to_int }
    print(codes)
    print(97.to_char)
}
```

```text
[75, 111, 118, 97]
'a'
```

Every method of texts, from `split` to patterns and base64, is in [String](/library/string/).
