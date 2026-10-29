---
title: Installer Kova
description: Installer la commande kova et l'extension pour l'éditeur.
sidebar:
  order: 1
---

Kova est une seule commande, `kova`, qui lance, compile, teste et met en forme les programmes.
Elle est écrite en Rust, et s'installe donc avec l'outil de Rust, `cargo`.

## 1. Installer Rust

Si `cargo --version` répond déjà, passe cette étape. Sinon, installe Rust avec
[rustup](https://rustup.rs) : sous Linux et macOS,

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

et sous Windows, télécharge et lance `rustup-init.exe` depuis le même site. Ouvre ensuite un
nouveau terminal, pour qu'il trouve `cargo`.

## 2. Installer Kova

```sh
cargo install --git https://github.com/Thibault-Savenkoff/kova --locked
```

Cela prend quelques minutes : cargo télécharge Kova et le compile. Vérifie ensuite :

```sh
kova --version
```

C'est tout ce dont `kova build` a besoin. Kova écrit les exécutables lui-même : il n'y a ni
compilateur C, ni éditeur de liens, ni autre outil à installer, sur aucun système.

:::note[Deux choses dont seuls certains programmes ont besoin sous Linux]
Un programme qui ouvre une fenêtre (`import ui`) a besoin de GTK 4, et un programme qui utilise
`https://` a besoin d'OpenSSL. Les deux sont déjà présents sur la plupart des bureaux, et ne sont
cherchés qu'au lancement du programme.
:::

## 3. Compiler pour d'autres systèmes (facultatif)

`kova build app.kova --target windows` crée un exécutable Windows depuis Linux ou un Mac, et ainsi
de suite pour chaque système. Kova a besoin d'une petite partie de lui-même compilée pour chaque
système visé ; ajoute ces systèmes à Rust **avant** d'installer Kova (ou réinstalle Kova après) :

```sh
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-pc-windows-msvc
```

Sans eux, `kova build --target …` dit lequel manque.

## 4. L'éditeur

L'extension pour VSCode et VSCodium colore le code, montre les erreurs pendant la frappe,
complète les noms, met en forme à l'enregistrement, et ajoute les boutons ▶ lancer, 🔨 compiler,
🧪 tester et 🐞 déboguer. Depuis une copie du dépôt :

```sh
cd editors/vscode
npm install
npm run package
code --install-extension kova-*.vsix
```

(`codium` au lieu de `code` pour VSCodium.) L'extension lance `kova lsp`, le serveur de langage
inclus dans la commande `kova` : après avoir installé un nouveau Kova, lance **Developer: Reload
Window** pour que l'éditeur démarre le nouveau.

Suite : [ton premier programme](/fr/start/first-program/).
