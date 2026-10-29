---
title: Editor and debugger
description: The VSCode and VSCodium extension, and the step-by-step debugger.
sidebar:
  order: 2
---

The extension for VSCode and VSCodium lives in `editors/vscode`. It starts `kova lsp`, the language
server inside the `kova` command, so what the editor says is exactly what `kova` says.

## Installing

From a clone of the repository:

```sh
cd editors/vscode
npm install
npm run package
codium --install-extension kova-*.vsix
```

(`code` instead of `codium` for VSCode.) `kova` must be on the `PATH`; otherwise set `kova.path` in
the settings to the executable. After installing a new `kova`, run **Developer: Reload Window** so
that the editor starts the new server.

## What it does

| Feature | How |
| --- | --- |
| Colors and snippets | for every `.kova` file |
| Errors while you type | underlined, with Kova's message and help |
| Completion | after a `.`, after `Type.` and `module.`, inside `Type { … }`, after `import`, for the parts of a call |
| Types on hover | hover a name to see its type and its declaration |
| Go to definition, rename | `F12`, `F2`, across imported files |
| Quick fix | adds the cases a `match` is missing |
| Format on save | `kova fmt`, or the **Format Document** command |
| Tests | a ▶ in the margin of each `test`, results in the Testing view |
| Buttons | ▶ run, 🔨 build and run, 🧪 test, 🐞 debug, at the top of the editor |

## The debugger

🐞, or **Run and Debug**, starts the program under `kova debug`:

- **breakpoints**: click in the margin, the program stops before that line;
- **step** over a line, into a call, out of a function;
- **variables** of each function on the call stack, with their current values; a value given
  away with `move` reads `(given away)`;
- the **Debug Console** evaluates any expression of the program while it is stopped:
  `rex.describe`, `scores.filter { x: Int => x > 10 }`. Hovering a name shows its value too.

The debugger runs the program in Kova's interpreter. A program that calls C (`extern`) or opens
windows (`ui`) cannot be stepped through: it stops at the first such call.

## Other editors

`kova lsp` speaks the Language Server Protocol and `kova debug` the Debug Adapter Protocol, so any
editor that supports them (Neovim, Helix, Zed, Emacs…) can use them; only the VSCode extension is
ready-made today.

## File managers

`editors/mime/install.sh` makes Dolphin and other file managers on Linux see `.kova` files as Kova
source, with Kova's icon.
