---
title: files
description: Reading, writing, copying and finding files and folders.
sidebar:
  order: 8
---

```kova
import files
```

Everything that touches the disk may fail: the file is missing, the folder is not allowed, the
disk is full. So the functions of `files` give a `T or files.IoError`, and Kova makes sure the
program looks at it ([Missing values and errors](/guide/errors/)).

```kova
import files

fn main {
    let saved: Nothing or files.IoError := files.write("note.txt", "hello\n")
    match files.read("note.txt") {
        ok(text) => write(text)
        err(e) => print("cannot read: {e}")
    }
    print(files.read("missing.txt"))
    let removed: Nothing or files.IoError := files.remove("note.txt")
}
```

```text
hello
err(NotFound("missing.txt"))
```

Paths are written with `/` on every system (Windows takes them too), and are relative to the
folder the program is started from, unless they start at the root.

## Reading and writing

| Function | Does |
| --- | --- |
| `files.read(path) -> String or IoError` | the whole text of a file (`NotText` if it is not UTF-8 text). |
| `files.write(path, text) -> Nothing or IoError` | writes `text` into the file, replacing what was there, creating it if needed. |
| `files.append(path, text) -> Nothing or IoError` | adds `text` at the end of the file, creating it if needed. |
| `files.sha256(path) -> String or IoError` | the SHA-256 fingerprint of the file, any file, text or not. |

To read a file line by line, `files.read(path)?.split("\n")`; for a table, see
[CSV](/library/json-and-csv/#csv).

## Files and folders

| Function | Does |
| --- | --- |
| `files.exists(path) -> Bool` | whether there is a file or a folder there. |
| `files.is_folder(path) -> Bool` | whether it is a folder. |
| `files.size(path) -> Int or IoError` | the size in bytes. |
| `files.modified(path) -> DateTime or IoError` | when it was last changed, in local time. |
| `files.list(folder) -> List<String> or IoError` | the names in a folder, sorted. |
| `files.matching(pattern) -> List<String> or IoError` | the paths that fit a pattern, sorted (below). |
| `files.make_dir(path) -> Nothing or IoError` | makes a folder, and the folders above it if needed. |
| `files.remove(path) -> Nothing or IoError` | deletes a file. |
| `files.copy(path) to(destination) -> Nothing or IoError` | copies a file, replacing `destination` if it exists. |
| `files.move(path) to(destination) -> Nothing or IoError` | moves or renames a file, replacing `destination` if it exists. |

```kova
import files

fn main {
    let made: Nothing or files.IoError := files.write("a.txt", "hello")
    let copied: Nothing or files.IoError := files.copy("a.txt") to("b.txt")
    print(files.size("b.txt"))
    let moved: Nothing or files.IoError := files.move("b.txt") to("c.txt")
    print(files.matching("*.txt"))
    for f in ["a.txt", "c.txt"] {
        let gone: Nothing or files.IoError := files.remove(f)
    }
}
```

```text
ok(5)
ok(["a.txt", "c.txt"])
```

### Patterns

In `files.matching`, `*` stands for any characters of a name, `?` for one character, and `**` for
any number of folders: `"photos/**/*.jpg"` finds every `.jpg` under `photos`, however deep.

## `files.IoError`

Each case holds the path concerned:

| Case | Means |
| --- | --- |
| `NotFound(path)` | there is no such file or folder |
| `Denied(path)` | the system does not allow it |
| `IsDirectory(path)` | a folder where a file was expected |
| `NotText(path)` | the file is not UTF-8 text (`files.read`) |
| `Other(path)` | anything else (a full disk…) |

`IoError` follows `Display`, so `"{e}"` writes `NotFound("missing.txt")`.

`import files as fs` gives the module another name: `fs.read(…)`.
