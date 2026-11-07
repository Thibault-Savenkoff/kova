---
title: Kova sur iOS
description: kova build --target ios-simulator fait une app pour le simulateur d'iPhone et d'iPad de Xcode.
sidebar:
  order: 4
---

Un programme Kova peut être une app d'iPhone et d'iPad. Pour l'instant elle tourne dans le
**simulateur** de Xcode, sur un Mac avec une puce Apple ; un vrai iPhone viendra plus tard (Apple
ne lance que des apps signées avec son propre certificat).

```console
$ kova build compteur.kova --target ios-simulator
✓ compteur.app (iOS Simulator, 180 Ko) en 4,5 s
```

`compteur.app` est un dossier : ton programme et le petit fichier qui le nomme. Kova n'a besoin ni
du compilateur de Xcode ni de Swift pour le faire. Sur le Mac, avec Xcode installé :

```console
$ xcrun simctl boot "iPhone 17"
$ xcrun simctl install booted compteur.app
$ xcrun simctl launch --console-pty booted dev.kova.compteur
```

Le dernier mot est le nom de l'app en minuscules, après `dev.kova.`. `--console-pty` montre ce que
le programme affiche.

## Ce qu'un programme trouve sur un iPhone

| Dans le programme | Sur l'iPhone |
|---|---|
| `print` | la console de `simctl launch --console-pty` |
| `import ui` | de vraies vues UIKit : textes, boutons, champs, interrupteurs, curseurs, choix, images, listes |
| une colonne ou une ligne de fenêtre | une vue en pile, avec ses marges et ses styles |

Pas encore là : `ui.canvas`, `ui.gpu`, les menus, les dialogues et le son. Un canvas affiche une
ligne de texte à sa place.

## Essayer un programme sans le doigt

La ligne de commande du simulateur ne sait pas toucher l'écran. Pour les tests, `KOVA_UI_TAP=0,0,1`
fait appuyer l'app sur ses propres boutons, par numéro, un à chaque fois que la fenêtre est
dessinée :

```console
$ SIMCTL_CHILD_KOVA_UI_TAP=0,0,1 xcrun simctl launch booted dev.kova.compteur
```
