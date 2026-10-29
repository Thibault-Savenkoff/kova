---
title: Overview
description: What the library holds, and what needs an import.
sidebar:
  order: 1
---

Kova comes with what most programs need, written for Kova and the same on every system. Nothing
is downloaded, and there is no package to choose between three that do almost the same.

## Always there

| Page | Holds |
| --- | --- |
| [String](/library/string/) | texts: searching, cutting, patterns, conversions, base64, colors |
| [Numbers, Bool and Char](/library/numbers/) | `Int`, `Float`, `BigInt`: operators, maths |
| [List](/library/list/) | ordered lists, blocks, sorting, statistics |
| [Map and Set](/library/map-and-set/) | values by key, values kept once |
| [Dates and time](/library/dates/) | `Date`, `DateTime`, `Weekday`, `now`, `sleep` |
| [JSON and CSV](/library/json-and-csv/) | reading and writing them into your own structs |
| [system, args and the terminal](/library/system/) | `print`, `read_line`, `args`, `exit` |
| [secure and chance](/library/secure/) | `random` |

## With an import

| Module | Page | Holds |
| --- | --- | --- |
| `import files` | [files](/library/files/) | reading and writing files and folders |
| `import system` | [system](/library/system/#import-system) | environment variables, running programs |
| `import web` | [web](/library/web/), [web server](/library/web-server/) | asking sites, answering as a server |
| `import secure` | [secure](/library/secure/#import-secure) | secure chance, passwords, encryption |
| `import log` | [log](/library/log/) | dated lines by level |
| `import ui` | [ui](/library/ui/) | native windows |

## Every value has

`x.clone`, a copy that the program then owns.

`print(x)` shows any value, fields and all. A text for people, `x.to_string` or `"{x}"` inside a
text, exists for numbers, `Bool`, `Char`, `String`, dates, and types that follow `Display`
([Texts](/guide/texts/)).

## Built-in interfaces

`Equal` (`=`), `Comparable` (`<`) and `Display` (`"{…}"`) are described in
[Interfaces and generics](/guide/interfaces-and-generics/#built-in-interfaces).
