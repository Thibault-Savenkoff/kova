---
title: Installing Kova
description: Install the kova command and the editor extension.
sidebar:
  order: 1
---

Kova is one command, `kova`, that runs, builds, tests and formats programs. It comes ready-made
for Linux, Windows and macOS: nothing else to install, no compiler, no linker, no Rust.

## 1. Install Kova

On Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.sh | sh
```

It puts `kova` in `~/.local/bin` (or the folder `KOVA_INSTALL` names). On Windows, in PowerShell:

```sh
irm https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.ps1 | iex
```

It puts `kova.exe` in `%LOCALAPPDATA%\Programs\kova` and adds it to your `PATH`. Then, in a new
terminal:

```sh
kova --version
```

The files are also on the [releases page](https://github.com/Thibault-Savenkoff/kova/releases),
one per system. Running the same command again installs the latest version.

:::note[Two things only some programs need on Linux]
A program that opens a window (`import ui`) needs GTK 4, and one that uses `https://` needs
OpenSSL. Both are already there on most desktops, and they are looked for only when the program
runs.
:::

## 2. Build for every system

Each `kova` builds for every system: `kova build app.kova --target windows` makes a Windows
executable from Linux or a Mac, and so on (`linux`, `linux-arm`, `windows`, `windows-arm`, `mac`,
`mac-arm`, `mac-intel`). On a Mac, `kova build` makes one executable for every Mac by default.

## 3. Building Kova itself (to change it)

Kova is written in Rust. With Rust installed by [rustup](https://rustup.rs), from a clone of the
repository:

```sh
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-pc-windows-msvc
cargo install --path . --locked
```

The `rustup target add` line gives Kova the pieces it needs for each system it builds for; without
it, this `kova` builds for its own machine only, and `kova build --target …` says what is missing.

## 4. The editor

The extension for VSCode and VSCodium colors the code, shows errors while you type, completes
names, formats on save, and adds the ▶ run, 🔨 build, 🧪 test and 🐞 debug buttons. From a clone
of the repository:

```sh
cd editors/vscode
npm install
npm run package
code --install-extension kova-*.vsix
```

(`codium` instead of `code` for VSCodium.) The extension starts `kova lsp`, the language server
inside the `kova` command: after installing a new Kova, run **Developer: Reload Window** so that
the editor starts the new one.

Next: [your first program](/start/first-program/).
