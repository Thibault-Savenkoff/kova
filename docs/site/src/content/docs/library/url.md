---
title: url
description: Writing texts into web addresses and reading them back.
sidebar:
  order: 17
---

```kova
import url
```

A web address may only hold a few characters: everything else is written with `%` and two hex
digits. A space becomes `%20`. Encode what you put in an address, and it cannot break it.

| Function | What it does |
| --- | --- |
| `url.encode(text) -> String` | the text, safe inside an address. Only letters, digits and `-_.~` stay as they are. |
| `url.decode(text) -> String?` | the text back; `+` is a space, as forms write it. `none` if the encoding is wrong. |
| `url.query(values: Map<String, String>) -> String` | `name=value&…`, sorted by name, each part encoded. |
| `url.parse_query(text) -> Map<String, String>` | the names and values of such a text. A part that cannot be read is left out. |

```kova
import url

fn main {
    let search: String := url.query(["q" := "kova & co", "page" := "2"])
    print("https://example.com/search?{search}")
    print(url.parse_query(search)["q"] else "?")
    print(url.decode("caf%C3%A9") else "?")
    print(url.decode("100%"))
}
```

```text
https://example.com/search?page=2&q=kova%20%26%20co
kova & co
café
none
```
