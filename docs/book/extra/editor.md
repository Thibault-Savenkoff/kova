# The editor

The extension for VSCode and VSCodium lives in `editors/vscode`. It starts `kova lsp`, the language
server inside the compiler, so what the editor shows is what `kova` itself says.

## Install

```text
cd editors/vscode
npm install
npm run package          # makes kova-<version>.vsix
codium --install-extension kova-<version>.vsix
```

`kova` must be on the PATH (`cargo install --path . --locked`). If the editor cannot find it, set
`kova.path` in the settings to the executable. After a new `kova` is installed, run
**Developer: Reload Window** so that the editor starts the new server.

## What it does

| Feature | How |
| --- | --- |
| Colors and snippets | automatic for `.kova` files |
| Errors while you type | underlined, with the help text |
| Completion | after a `.`, after `Type.`, `module.`, inside `Type { … }`, after `import`, for `match` cases, for parameters of a call |
| Types on hover | hover a name; inlay hints show inferred types |
| Go to definition, rename | `F12`, `F2`, across imported files |
| Quick fix | "add the missing `match` cases" |
| Format | `kova fmt` on save, or the Format Document command |
| Tests | a ▶ in the margin of each `test`, results in the Testing view |
| Buttons | ▶ run (`kova.run`), 🔨 build and run (`kova.build`), 🧪 test (`kova.test`), 🐞 debug (`kova.debug`) |
| Debugger | breakpoints, step, variables (a given-away value reads `(given away)`), and the Debug Console evaluates any expression of the program while stopped |

## Settings

- `kova.path`: the `kova` executable to use (default: the one on the PATH). Inside a Flatpak
  VSCodium the extension reaches the host's `kova` through `flatpak-spawn --host`.

## Files

`editors/mime/install.sh` makes file managers (Dolphin and others) see `.kova` files as Kova source,
with its icon.
