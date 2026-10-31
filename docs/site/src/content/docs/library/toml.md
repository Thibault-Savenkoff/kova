---
title: toml
description: Reading and writing TOML, the format of configuration files.
sidebar:
  order: 19
---

```kova
import toml
```

TOML is the format of configuration files: lines `name = value`, sections `[name]`, lists of
sections `[[name]]`.

`toml.to_json(text) -> String or toml.Error` turns it into JSON. You then read that JSON into a
struct of your program with [`from_json`](/library/json-and-csv/), which checks every field and its
type: there is one way to read data, for both formats.

```kova
import toml

struct Server {
public:
    host: String
    port: Int
}

struct Config {
public:
    name: String
    debug: Bool
    servers: List<Server>
}

fn main {
    let text: String := """
    name = "shop"
    debug = true

    [[servers]]
    host = "alpha"
    port = 8080
    """
    let json: String := toml.to_json(text)?
    let config: Config := Config.from_json(json)?
    print(config.servers[0].host)
    print(config.debug)
}
```

```text
alpha
true
```

A `toml.Error` holds `line` and `message`, and `e.to_string` writes both:

```kova
import toml

fn main {
    match toml.to_json("port = 80\nport = 81") {
        ok(json) => print(json)
        err(e) => print(e.to_string)
    }
}
```

```text
line 2: `port` is defined twice
```

Everything in TOML is read: all the kinds of texts, dotted names (`a.b = 1`), lists, `{ x = 1 }`
tables, dates, and the numbers `1_000` and `0xff`. A date stays a text in the JSON, which a `Date`
field reads. `inf` and `nan` are refused, since JSON cannot write them.

## Writing TOML

`toml.from_json(json) -> String or toml.Error` is the other direction: give it what `to_json` writes
for a struct of your program, and save the result. TOML has no `null`, so an absent field is left out.

```kova
import toml

struct Owner {
public:
    name: String
}

struct Settings {
public:
    port: Int
    tags: List<String>
    owner: Owner
}

fn main {
    let s: Settings := Settings { port := 80, tags := ["a"], owner := Owner { name := "Ada" } }
    print(toml.from_json(s.to_json)?)
}
```

```text
port = 80
tags = ["a"]

[owner]
name = "Ada"

```
