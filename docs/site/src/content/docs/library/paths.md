---
title: paths
description: Taking file paths apart and putting them together.
sidebar:
  order: 16
---

```kova
import paths
```

A path is a plain `String`. These functions cut it and join it without touching the disk, and they
understand both `/` and Windows' `\`.

| Function | What it does |
| --- | --- |
| `paths.join(first, second) -> String` | `second` inside `first`. An absolute `second` wins, as if you changed folder. |
| `paths.folder(path) -> String` | the folder part, empty if there is none. |
| `paths.name(path) -> String` | the last part, the file's name. |
| `paths.stem(path) -> String` | the name without its extension. |
| `paths.extension(path) -> String` | the extension without its dot, empty if there is none. |
| `paths.absolute(path) -> Bool` | whether it starts at the root of a disk. |

```kova
import paths

fn main {
    let report: String := paths.join("data/2026", "report.final.csv")
    print(report)
    print(paths.folder(report))
    print(paths.name(report))
    print(paths.stem(report))
    print(paths.extension(report))
    print(paths.stem(".bashrc"))
}
```

```text
data/2026/report.final.csv
data/2026
report.final.csv
report.final
csv
.bashrc
```

A name that starts with a dot, like `.bashrc`, has no extension: the dot is part of its name.
