---
title: Run or build
description: What kova run and kova build do, how a function can differ between them, and the header of a program.
sidebar:
  order: 14
---

## Two ways, one result

Every program can be started two ways:

- `kova run app.kova` checks the program and runs it at once, in Kova's interpreter. Nothing is
  written to the disk; it starts in a few milliseconds. This is the mode for writing and trying.
- `kova build app.kova` checks the program and compiles it to native code, an executable that runs
  without Kova. This is the mode for speed and for giving the program to others.

Both print the same thing, digit for digit, on every system. Kova's own tests run every example
program both ways and compare the outputs.

Programs that call [C libraries](/guide/calling-c/) or open [windows](/guide/windows/) run as
native code even under `kova run`: Kova builds them in memory first, without being asked.

## A function that differs: `mode`

Sometimes a function should only do its work in one mode: a banner only the built program shows,
a slow check only worth running while writing, a call to something the interpreter cannot do.
`mode build` (or `mode run`) under the signature says so, and says what a call does **in the
other mode**:

```kova
fn banner
    mode build else skip
{
    print("=== built ===")
}

fn workers_count -> Int
    mode build else return 1
{
    return 8
}

fn main {
    banner
    print(workers_count)
}
```

```text
1
```

Built, the same program prints `=== built ===` and `8`. The choices after `else`:

| Written | In the other mode, a call |
| --- | --- |
| `else skip` | does nothing (only for a function that returns nothing) |
| `else return value` | gives this value (`else return none` for a `T?`) |
| `else "message"` | stops the program with this message, like a broken `require` |
| no `else` | runs the function as well, with the same results |

The last one is for speed: under `kova run`, a function marked `mode build` without `else` makes
Kova compile the whole program to native code before running it, which takes a few milliseconds.

In the other mode, the function's contracts are left out too, while its arguments are still
computed (a `move` argument is given away all the same). `kova test` counts as `kova run`.

Why at the function and not an `if` inside the code? Because it is then visible in one place, at
the top of the function, where every reader of a call can find it; and the compiler can check
that both modes give a value of the right type.

## The header

A program may start with a `header`, its settings:

```kova
header {
    mode := build
    lock := true
    lang := fr
}

fn main {
    print("Bonjour")
}
```

| Setting | Values | Says |
| --- | --- | --- |
| `mode` | `run` (default), `build` | what `kova app.kova`, without a command, does |
| `lock` | `false` (default), `true` | `true`: the other command is refused, for a program meant only to be built (or only run) |
| `lang` | `auto` (default), `en`, `fr` | the language of Kova's messages for this program |
| `name` | a text: `name := "Counter"` | the program's name: the executable `kova build` makes, and the name the Dock, the taskbar and the list of programs show (by default, the file's name) |
| `icon` | an image: `icon := "counter.svg"` | the program's icon: PNG, SVG, JPEG, `.ico` or `.icns`, square, from the program's folder ([below](#the-icon)) |

With `mode := build`, `kova app.kova` builds the program, and with `lock := true`, `kova run
app.kova` explains that this program is meant to be built.

## The icon

`icon := "counter.svg"` gives the program its icon. Kova reads the image when it builds the
program and puts it inside the executable: the image file is not needed afterwards.

- **Formats:** PNG, SVG (drawn at 512×512), JPEG, Windows' `.ico` and the Mac's `.icns`. A
  `.ico` keeps all its sizes on Windows, a `.icns` all its sizes on the Mac; on the other systems,
  their largest image is used.
- **Square:** an image that is not square is refused (an SVG is centered in a square instead).
  512×512 is the best size: the Dock shows it sharp on a Retina screen.
- **Where it shows:**
  - Windows: the executable's icon in Explorer, and the window's in the title bar and the taskbar;
  - Mac: the Dock, while a program with [windows](/guide/windows/) runs;
  - Linux: the window's icon, when the desktop reads it from the program (GNOME and KDE may still
    show a generic one: they look for a `.desktop` file, which Kova does not install).

```plaintext
error: the icon `logo.png` is 640×480: it must be square (512×512 is best)
```

## The executable

`kova build` writes the executable next to the program: `app` on Linux and macOS, `app.exe` on
Windows (`-o name` chooses another name). It needs no Kova, no runtime and no library to run, apart
from GTK 4 on Linux for windows and OpenSSL on Linux for `https://`, both found when the program
starts.

- `--target windows`, `linux`, `mac` (and their `-arm` variants) build for another system, from any
  system ([the command](/tools/command/#targets)).
- `--boost` leaves the [contracts](/guide/contracts-and-tests/) out.
- A built program picks the language of its messages when it starts, like Kova itself.
