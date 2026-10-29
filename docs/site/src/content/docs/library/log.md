---
title: log
description: Dated lines on the error output, by level, chosen by whoever runs the program.
sidebar:
  order: 13
---

```kova
import log
```

`print` writes what the program **gives**; `log` writes what it **does**, for whoever watches it
run: one line on the error output, with the time and the level.

```kova
import log

fn main {
    log.info("started")
    log.debug("not shown")
    log.warning("disk almost full")
    print("the result")
}
```

```text
the result
```

On the error output:

```console
14:05:09  info     started
14:05:09  warning  disk almost full
```

Since the lines go to the error output, `./tool > result.txt` keeps them out of the file.

| Function | Writes |
| --- | --- |
| `log.debug(text)` | details for finding a bug |
| `log.info(text)` | what the program is doing |
| `log.warning(text)` | something wrong it can go on with |
| `log.error(text)` | something wrong it cannot |
| `log.level(log.Level.Debug)` | from there on, every line from that level up |

Without `log.level`, lines from `info` up are written. `log.Level` is `Debug`, `Info`, `Warning` or
`Error`.

Whoever runs the program chooses too, without changing it: `KOVA_LOG=debug ./tool` shows
everything, `KOVA_LOG=error ./tool` only errors. `KOVA_LOG` wins over `log.level`.
