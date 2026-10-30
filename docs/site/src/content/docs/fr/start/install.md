---
title: Installer Kova
description: Installer la commande kova et l'extension pour l'éditeur.
sidebar:
  order: 1
---

Kova est une seule commande, `kova`, qui lance, compile, teste et met en forme les programmes.
Elle existe toute prête pour Linux, Windows et macOS : rien d'autre à installer, ni compilateur,
ni éditeur de liens, ni Rust.

## 1. Installer Kova

Sous Linux et macOS :

```sh
curl -fsSL https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.sh | sh
```

Il met `kova` dans `~/.local/bin` (ou dans le dossier que nomme `KOVA_INSTALL`). Sous Windows,
dans PowerShell :

```sh
irm https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.ps1 | iex
```

Il met `kova.exe` dans `%LOCALAPPDATA%\Programs\kova` et l'ajoute à ton `PATH`. Ensuite, dans un
nouveau terminal :

```sh
kova --version
```

Pour **mettre Kova à jour**, relance la même commande : elle installe toujours la dernière
version. Pour le **désinstaller**, supprime le fichier `kova` (ou le dossier `kova` sous Windows).

### Ou télécharger à la main

Chaque version de Kova est publiée toute prête sur la
[page des versions](https://github.com/Thibault-Savenkoff/kova/releases). Prends le fichier de ton
système :

| Ton système | Le fichier |
| --- | --- |
| Linux sur un PC (Intel, AMD) | `kova-linux-x64.tar.gz` |
| Linux sur ARM (Raspberry Pi 4/5, serveurs ARM) | `kova-linux-arm64.tar.gz` |
| Windows sur un PC (Intel, AMD) | `kova-windows-x64.zip` |
| Windows sur ARM (Surface Pro X, Snapdragon) | `kova-windows-arm64.zip` |
| macOS, tous les Mac (Apple Silicon et Intel) | `kova-mac.tar.gz` |

Sous Linux, `uname -m` dit lequel prendre : `x86_64` pour x64, `aarch64` pour arm64.

Chaque fichier contient la commande `kova` (`kova.exe` sous Windows) et ses deux licences, rien
d'autre. Décompresse-le, puis mets `kova` dans un dossier de ton `PATH` :

```sh
tar -xzf kova-linux-x64.tar.gz
mv kova ~/.local/bin/
```

Sous Windows, fais un clic droit sur le `.zip`, choisis **Extraire tout**, puis mets `kova.exe`
dans un dossier de ton `PATH`.

:::caution[Sur Mac, après un téléchargement par le navigateur]
macOS bloque un programme téléchargé qui n'est pas signé par Apple (« impossible de vérifier le
développeur »). Retire ce blocage une fois :

```sh
xattr -d com.apple.quarantine kova
```

La commande `curl` ci-dessus n'a pas ce problème.
:::

:::note[Deux choses dont seuls certains programmes ont besoin sous Linux]
Un programme qui ouvre une fenêtre (`import ui`) a besoin de GTK 4, et un programme qui utilise
`https://` a besoin d'OpenSSL. Les deux sont déjà présents sur la plupart des bureaux, et ne sont
cherchés qu'au lancement du programme.
:::

## 2. Compiler pour tous les systèmes

Chaque `kova` compile pour tous les systèmes : `kova build app.kova --target windows` crée un
exécutable Windows depuis Linux ou un Mac, et ainsi de suite (`linux`, `linux-arm`, `windows`,
`windows-arm`, `mac`, `mac-arm`, `mac-intel`). Sur Mac, `kova build` fait par défaut un seul
exécutable pour tous les Mac.

## 3. Construire Kova lui-même (pour le modifier)

Kova est écrit en Rust. Avec Rust installé par [rustup](https://rustup.rs), depuis un clone du
dépôt :

```sh
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc aarch64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-pc-windows-msvc
cargo install --path . --locked
```

La ligne `rustup target add` donne à Kova les morceaux dont il a besoin pour chaque système visé ;
sans elle, ce `kova` ne compile que pour sa propre machine, et `kova build --target …` dit ce qui
manque.

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
