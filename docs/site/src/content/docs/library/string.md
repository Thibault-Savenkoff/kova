---
title: String
description: Texts in Kova, and every method they have.
sidebar:
  order: 2
---

A `String` is a text: any Unicode characters, as many as wanted. It is written between double
quotes, and it owns its characters, so it [moves](/guide/ownership/) like any value that owns
memory (`.clone` copies it).

```kova
fn main {
    let name: String := "Ada"
    let line: String := "{name} wrote the first program in {1843}"
    print(line)
    print(line.length)
    print(name + " Lovelace")
}
```

```text
Ada wrote the first program in 1843
35
Ada Lovelace
```

## Writing texts

- `{…}` inside a text puts a value there: a name, a call, a calculation (`"{a + b}"`). The value
  must have a text for people: numbers, `Bool`, `Char`, `String`, dates, and types that follow
  [`Display`](/guide/interfaces-and-generics/#built-in-interfaces) ([Texts](/guide/texts/)).
- `\n` is a line break, `\t` a tab, `\"` a quote, `\\` a backslash, and `\{` a brace that does not
  open a value.
- A **raw text**, between `"""`, keeps everything as written, braces and backslashes included:
  `"""{"title": "Dune"}"""`. Over several lines, it starts on the line after its `"""`, and the
  indentation of the closing `"""` is taken off every line:

```kova
fn main {
    let page: String := """
        <h1>{title}</h1>
          <p>C:\Users</p>
        """
    print(page)
}
```

```text
<h1>{title}</h1>
  <p>C:\Users</p>
```

- `+` joins two texts. It never turns a number into a text by itself: write `"{n}"` or
  `n.to_string`.
- `=`, `!=`, `<`, `<=`, `>`, `>=` compare texts, `<` in the order of their characters' codes
  (`"B" < "a"`).

Positions in a text count **characters**, from 0, not bytes: `"héllo".length` is 5, and `é` is one
character.

## Looking inside

| Method | Gives |
| --- | --- |
| `s.length -> Int` | the number of characters. |
| `s.contains(part: String) -> Bool` | whether `part` is somewhere in `s` (`""` is in every text). |
| `s.starts_with(part: String) -> Bool` | whether `s` begins with `part`. |
| `s.ends_with(part: String) -> Bool` | whether `s` ends with `part`. |
| `s.find(part: String) -> Int?` | the position of the first `part`, or `none` if there is none. |
| `s[a..b] -> String` | the characters from position `a` up to, not including, `b` (`s[a..=b]` includes it). Past the end, the program stops. |
| `s.chars -> List<Char>` | every character, in order. |

```kova
fn main {
    let s: String := "héllo world"
    print(s.contains("world"))
    print(s.find("o"))
    print(s.find("z"))
    print(s[0..5])
    print(s.chars[1])
}
```

```text
true
4
none
héllo
é
```

## Making a new text

A `String` never changes in place: these methods give a new text and leave `s` as it was.

| Method | Gives |
| --- | --- |
| `s.trim -> String` | `s` without spaces, tabs and line breaks at both ends. |
| `s.is_empty -> Bool` | whether `s` has no character. |
| `s.lines -> List<String>` | the lines of `s`, without their line breaks (`\n` or `\r\n`). |
| `s.reversed -> String` | the characters of `s`, last one first. |
| `s.to_upper -> String` | `s` in capitals, accented letters too (`"été"` → `"ÉTÉ"`). |
| `s.to_lower -> String` | `s` in small letters. |
| `s.replace(old: String) with(new: String) -> String` | `s` with every `old` replaced by `new`. `old` cannot be `""`. |
| `s.repeat(n: Int) -> String` | `s` written `n` times one after another. |
| `s.wrapped(width: Int) -> String` | `s` cut into lines of at most `width` characters, between words. |

```kova
fn main {
    print("  hello \n".trim)
    print("Été".to_upper)
    print("a-b-a".replace("a") with("x"))
    print("ab".repeat(3))
    print("Kova checks every pattern before the program runs.".wrapped(20))
}
```

```text
hello
ÉTÉ
x-b-x
ababab
Kova checks every
pattern before the
program runs.
```

## Cutting and joining

| Method | Gives |
| --- | --- |
| `s.split(separator: String) -> List<String>` | the pieces between each `separator`; two separators in a row give an empty piece. `separator` cannot be `""`. |
| `s.words -> List<String>` | the words, wherever the spaces, tabs and line breaks are, never an empty one. |
| `l.join(separator: String) -> String` | on a `List<String>`: its texts with `separator` between each. |

```kova
fn main {
    print("a,,b".split(","))
    print("  the quick\tfox ".words)
    print(["x", "y", "z"].join(" + "))
}
```

```text
["a", "", "b"]
["the", "quick", "fox"]
x + y + z
```

## Turning a text into a value

Each of these gives `none` when the text is not exactly a value of that kind, spaces included, so
the program decides what to do then:

| Method | Reads |
| --- | --- |
| `s.to_int -> Int?` | a whole number: `"42"`, `"-5"` (not `"+5"`, `" 5"` or `"1_000"`). |
| `s.to_float -> Float?` | a number with a dot: `"2.5"`, `"1e3"`, `"-0.1"`. |
| `s.to_bool -> Bool?` | exactly `"true"` or `"false"`. |
| `s.to_date -> Date?` | a date written `"2026-10-23"`, which must exist (`"2026-02-30"` is `none`). |
| `s.to_datetime -> DateTime?` | a date and a time written `"2026-10-23 14:05:09"`. |

```kova
fn main {
    match "42".to_int {
        none => print("not a number")
        n => print(n + 1)
    }
    print("forty".to_int)
    print("2.5".to_float)
    print("2026-10-23".to_date)
}
```

```text
43
none
2.5
2026-10-23
```

The other way, every value has `to_string`, the text `print` would write.

## Patterns

A pattern describes what a text looks like, to check it, find pieces of it or change them. Kova's
patterns are the usual regular expressions:

| Pattern | Matches |
| --- | --- |
| `\d`, `\w`, `\s` | a digit; a letter, digit or `_`; a space, tab or line break |
| `.` | any character |
| `[a-z]`, `[^,]` | one of these characters; any character but these |
| `x*`, `x+`, `x?`, `x{2,5}` | `x` repeated: any number of times, at least once, at most once, 2 to 5 times (`*?`, `+?`: as few as possible) |
| `a\|b` | `a` or `b` |
| `(…)` | a group, to repeat it or to read it with `groups` |
| `^`, `$`, `\b` | the start of the text, its end, the edge of a word |

| Method | Gives |
| --- | --- |
| `s.matches(pattern: String) -> Bool` | whether the **whole** text fits the pattern. |
| `s.all_matches(pattern: String) -> List<String>` | every piece of `s` that fits, from left to right. |
| `s.replace_matches(pattern: String) with(new: String) -> String` | `s` with every piece that fits replaced by `new`. |
| `s.groups(pattern: String) -> List<String>?` | the groups `(…)` of the first piece that fits, or `none` if none does. |

```kova
fn main {
    let line: String := "12 apples, 7 pears and 145 plums"
    print(line.all_matches("""\d+"""))
    print("ana@example.org".matches("[a-z.]+@[a-z.]+"))
    print("a   b \t c".replace_matches("""\s+""") with(" "))
    let parts: List<String>? := "on 2026-10-25".groups("""(\d+)-(\d+)-(\d+)""")
    if parts != none {
        print("{parts[2]}/{parts[1]}/{parts[0]}")
    }
}
```

```text
["12", "7", "145"]
true
a b c
25/10/2026
```

The pattern is written in the call, so the compiler reads it and refuses a broken one before the
program runs. Write it as a raw text when it holds a `\`: `"""\d+"""` rather than `"\\d+"`. Kova
runs patterns with its own engine, whose time grows with the length of the text and never
explodes, whatever the pattern.

## Fingerprints and base64

| Method | Gives |
| --- | --- |
| `s.sha256 -> String` | the SHA-256 fingerprint of the text, 64 hexadecimal digits. |
| `s.hmac_sha256(key: String) -> String` | the HMAC-SHA-256 signature of the text with a secret key. |
| `s.to_base64 -> String` | the text written in base64. |
| `s.to_base64url -> String` | the same, with the alphabet used in addresses and without `=`. |
| `String.from_base64(code: String) -> String?` | the text back from base64, `none` if `code` is not the base64 of a text. |
| `String.from_base64url(code: String) -> String?` | the same for `to_base64url`. |

```kova
fn main {
    print("hello".sha256)
    print("Kova".to_base64)
    print(String.from_base64("S292YQ=="))
}
```

```text
2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
S292YQ==
"Kova"
```

A fingerprint is not a way to keep passwords: use
[`secure.password_hash`](/library/secure/) for that.

## Colors

`s.red`, `s.green`, `s.yellow`, `s.blue`, `s.magenta`, `s.cyan`, `s.gray`, `s.bold` and
`s.underline` give the text in that color or style, and can follow one another:
`"error:".red.bold`. The colors are shown only on a terminal: written into a file or passed to
another program, the text stays plain. `NO_COLOR=1` makes it plain on a terminal too, and
`FORCE_COLOR=1` paints it everywhere.

## Every text also has

- `s.clone -> String`: a copy.
- `s.to_string -> String`: the text itself.
