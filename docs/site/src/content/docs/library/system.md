---
title: system, args and the terminal
description: The program's arguments, its input and output, environment variables, running other programs, exit codes.
sidebar:
  order: 9
---

## Always there

These need no import:

| Function | Does |
| --- | --- |
| `print(value)` | writes the value and a line break on the output. Any value: `print(rex)` shows its fields. |
| `write(value)` | the same, without the line break. |
| `read_line -> String?` | the next line of the input, without its line break; `none` at the end of the input. |
| `args -> List<String>` | the words given after the program's name: `kova run app.kova a b`, `./app a b`. |
| `exit(code: Int)` | ends the program at once with this exit code (0 means success). |

```kova
fn main {
    var total: Int := 0
    while true {
        let line: String? := read_line
        if line = none {
            break
        }
        match line.to_int {
            none => print("not a number: {line}")
            n => total += n
        }
    }
    print("total: {total}")
}
```

```sh
printf '4\n5\nsix\n' | kova run sum.kova
```

```console
not a number: six
total: 9
```

A program that stops on an error (a broken contract, an index out of bounds) writes it on the error
output and ends with the code 101.

For colors on the terminal, see [`String`](/library/string/#colors); for dated lines on the error
output, [`log`](/library/log/).

## `import system`

```kova
import system
```

| Function | Gives |
| --- | --- |
| `system.env(name: String) -> String?` | the value of an environment variable, `none` if it is not set. |
| `system.run(program: String) -> system.Finished or system.RunError` | runs a program without arguments, and waits for it. |
| `system.run(program) args(words: List<String>) -> system.Finished or system.RunError` | the same with arguments. |

```kova
import system

fn main {
    print(system.env("KOVA_NO_SUCH_NAME"))
    match system.run("kova-no-such-program") args(["--version"]) {
        ok(done) => print("{done.output} (code {done.code})")
        err(e) => print(e)
    }
}
```

```text
none
NotFound("kova-no-such-program")
```

- The program is looked for in the folders of `PATH` when its name holds no folder.
- Its input is empty; `system.Finished` holds `output` (what it wrote), `errors` (what it wrote on
  the error output) and `code` (its exit code, 128 + the signal's number if a signal stopped it).
- A program that ran and failed is still `ok`: look at `code`. `err` means it could not start:
  `NotFound(program)`, `Denied(program)` or `Other(program)`.
- The words go to the program as they are. No shell reads them, so a space, a `;` or a `$` in one
  is just a character: nothing from a user can turn into another command.
