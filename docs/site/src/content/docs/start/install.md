---
title: Installing Kova
description: Install the kova command and the editor extension.
sidebar:
  order: 1
---

Kova is one command, `kova`, that runs, builds, tests and formats programs. It is written in Rust,
so it is installed with Rust's tool, `cargo`.

## 1. Install Rust

If `cargo --version` already answers, skip this step. Otherwise, install Rust with
[rustup](https://rustup.rs): on Linux and macOS,

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

and on Windows, download and run `rustup-init.exe` from the same site. Open a new terminal
afterwards, so that it finds `cargo`.

## 2. Install Kova

```sh
cargo install --git https://github.com/Thibault-Savenkoff/kova --locked
```

This takes a few minutes: cargo downloads Kova and compiles it. Then check it:

```sh
kova --version
```

That is all `kova build` needs. Kova writes executables itself: there is no C compiler, linker or
other tool to install, on any system.

:::note[Two things only some programs need on Linux]
A program that opens a window (`import ui`) needs GTK 4, and one that uses `https://` needs
OpenSSL. Both are already there on most desktops, and they are looked for only when the program
runs.
:::

## 3. Build for other systems (optional)

`kova build app.kova --target windows` makes a Windows executable from Linux or a Mac, and so on
for every system. Kova needs a small piece of itself compiled for each system it builds for; add
those systems to Rust **before** installing Kova (or install Kova again afterwards):

```sh
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-pc-windows-msvc
```

Without them, `kova build --target …` says which one is missing.

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
