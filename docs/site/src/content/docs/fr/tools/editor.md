---
title: Éditeur et débogueur
description: L'extension pour VSCode et VSCodium, et le débogueur pas à pas.
sidebar:
  order: 2
---

L'extension pour VSCode et VSCodium se trouve dans `editors/vscode`. Elle lance `kova lsp`, le
serveur de langage qui est dans la commande `kova`, donc ce que dit l'éditeur est exactement ce que
dit `kova`.

## L'installer

Depuis une copie du dépôt :

```sh
cd editors/vscode
npm install
npm run package
codium --install-extension kova-*.vsix
```

(`code` au lieu de `codium` pour VSCode.) `kova` doit être dans le `PATH` ; sinon, mets dans le
réglage `kova.path` le chemin de l'exécutable. Après avoir installé un nouveau `kova`, lance
**Developer: Reload Window** pour que l'éditeur démarre le nouveau serveur.

## Ce qu'elle fait

| Fonction | Comment |
| --- | --- |
| Couleurs et extraits | pour chaque fichier `.kova` |
| Erreurs pendant la frappe | soulignées, avec le message et l'aide de Kova |
| Complétion | après un `.`, après `Type.` et `module.`, dans `Type { … }`, après `import`, pour les parties d'un appel |
| Types au survol | survole un nom pour voir son type et sa déclaration |
| Aller à la définition, renommer | `F12`, `F2`, à travers les fichiers importés |
| Correction rapide | ajoute les cas qui manquent à un `match` |
| Mise en page à l'enregistrement | `kova fmt`, ou la commande **Format Document** |
| Tests | un ▶ dans la marge de chaque `test`, les résultats dans la vue Testing |
| Boutons | ▶ lancer, 🔨 compiler et lancer, 🧪 tester, 🐞 déboguer, en haut de l'éditeur |

## Le débogueur

🐞, ou **Run and Debug**, lance le programme sous `kova debug` :

- **points d'arrêt** : clique dans la marge, le programme s'arrête avant cette ligne ;
- **avancer** d'une ligne, entrer dans un appel, sortir d'une fonction ;
- les **variables** de chaque fonction de la pile d'appels, avec leurs valeurs du moment ; une
  valeur donnée avec `move` s'affiche `(given away)` ;
- la **Debug Console** calcule n'importe quelle expression du programme pendant qu'il est arrêté :
  `rex.describe`, `scores.filter { x: Int => x > 10 }`. Survoler un nom montre aussi sa valeur.

Le débogueur lance le programme dans l'interpréteur de Kova. Un programme qui appelle du C
(`extern`) ou ouvre des fenêtres (`ui`) ne peut pas être suivi pas à pas : il s'arrête au premier
appel de ce genre.

## Les autres éditeurs

`kova lsp` parle le Language Server Protocol et `kova debug` le Debug Adapter Protocol, donc
n'importe quel éditeur qui les connaît (Neovim, Helix, Zed, Emacs…) peut s'en servir ; seule
l'extension VSCode est prête aujourd'hui.

## Les gestionnaires de fichiers

`editors/mime/install.sh` fait voir aux gestionnaires de fichiers de Linux, Dolphin et les autres,
les fichiers `.kova` comme du code Kova, avec l'icône de Kova.
