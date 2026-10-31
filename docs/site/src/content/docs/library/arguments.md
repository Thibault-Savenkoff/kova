---
title: arguments
description: Reading a program's command line.
sidebar:
  order: 18
---

```kova
import arguments
```

`system.args` gives the words after the program's name. `arguments.read` sorts them, so
`tool --port 8080 --verbose photos` does not need a loop of your own. Pass it `system.args`; the
example below uses a written list so that it can show its result.

```kova
import arguments

fn main {
    let given: List<String> := ["photos", "--port", "8080", "--verbose", "--name=Ada"]
    let a: arguments.Arguments := arguments.read(given) options(["port"])
    print(a.words)
    print(a.flag("verbose"))
    print(a.option("port") else "80")
    print(a.option("name") else "nobody")
}
```

```text
["photos"]
true
8080
Ada
```

`options([…])` names the options that take a value. It is optional, because a value can always be
written `--port=8080`.

| Written | Read as |
| --- | --- |
| `--port 8080`, `-p 8080` | an option, if `port` or `p` is named in `options([…])` |
| `--port=8080` | an option, always |
| `--verbose`, `-v` | a flag |
| `--` | everything after it is a word, even if it starts with `-` |
| anything else | a word |

| Member | What it gives |
| --- | --- |
| `a.flag(name) -> Bool` | whether this flag was written. |
| `a.option(name) -> String?` | the value of this option, `none` if it was not written. |
| `a.words -> List<String>` | the other words, in order. |
