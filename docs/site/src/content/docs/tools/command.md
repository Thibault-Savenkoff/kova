---
title: The kova command
description: run, build, test, fmt, the targets, the options and the exit codes.
sidebar:
  order: 1
---

Everything goes through one command:

| Command | Does |
| --- | --- |
| `kova run app.kova [words…]` | checks the program and runs it at once; the words become its `system.args` |
| `kova build app.kova` | makes a native executable next to the file |
| `kova test app.kova ["name"]` | runs the program's `test` blocks, or only the one named |
| `kova fmt app.kova…` | lays the files out the standard way, in place |
| `kova app.kova` | does what the program's [header](/guide/run-or-build/#the-header) says, `run` by default |
| `kova lsp` | the language server, started by the editor |
| `kova debug` | the debugger, started by the editor |
| `kova --version` | Kova's version |

Every command reads the files the program imports by itself. There is no project file.

## `kova build`

| Option | Does |
| --- | --- |
| `-o name` | names the executable (default: the header's [`name`](/guide/run-or-build/#the-header), else the file's name; `.exe` added on Windows) |
| `--boost` | leaves the contracts (`require`, `ensure`, `check`) out, for speed |
| `--target system` | builds for another system (below) |

When it is done, it says what it made, for which system, and how long it took:

```console
✓ app (Linux, 29 KB) in 3 ms
```

The executable needs nothing installed to run, apart from GTK 4 on Linux for a program with
windows and OpenSSL on Linux for `https://`. Kova writes it entirely itself: no C compiler, no
linker, no SDK, on any system.

### Targets

From any system, `--target` builds for any other:

| Target | For |
| --- | --- |
| `linux` | Linux on x86-64 (Intel, AMD) |
| `linux-arm` | Linux on arm64 (Raspberry Pi 4 and later, ARM servers) |
| `windows` | Windows on x86-64 |
| `windows-arm` | Windows on arm64 |
| `mac` | every Mac, in one file (Apple chips and Intel); the default on a Mac |
| `mac-arm` | Macs with an Apple chip only |
| `mac-intel` | Intel Macs only |

Each target needs a small piece of Kova compiled for it, added to Rust before installing Kova
([Installing](/start/install/#3-build-for-other-systems-optional)); `kova build` says which one is
missing otherwise.

## `kova install`

`kova install counter.kova` builds the program and installs it for you, where your system looks
for programs. `kova uninstall counter.kova` removes everything it wrote. Nothing needs
administrator rights.

| System | The program | With [windows](/guide/windows/), also |
| --- | --- | --- |
| Linux | `~/.local/bin/<name>` | a `.desktop` file and the icon in `~/.local/share`: the program is in the applications menu, with its icon in GNOME, KDE and the others |
| Mac | `~/.local/bin/<name>` | instead, `~/Applications/<name>.app`: Launchpad, the Dock and the Finder show it with its icon |
| Windows | `%LOCALAPPDATA%\Programs\<name>\<name>.exe` | a shortcut in the Start menu |

`<name>` is the header's [`name`](/guide/run-or-build/#the-header), else the file's name.

## `kova test`

Runs each `test` block of the file on its own, and writes ✅ or ❌ with the message of the first
broken `check` ([Contracts and tests](/guide/contracts-and-tests/#tests)). `kova test` counts as
`kova run` for [`mode`](/guide/run-or-build/#a-function-that-differs-mode).

## `kova fmt`

Lays the code out the standard way: indentation and spaces. It never moves a line break, and
refuses to change a file if the result would not mean exactly the same program. The editor runs it
on save.

## Exit codes

| Code | Means |
| --- | --- |
| `0` | the program ended normally |
| `1` | Kova refused the program (an error in the code), or a test failed |
| `101` | the program stopped on an error while running: an index out of bounds, an overflow, a broken contract… |
| other | what the program gave to `system.exit(code)` |

A built executable ends with the same codes.
