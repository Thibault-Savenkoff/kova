---
title: Kova sur iOS
description: kova build --target ios-simulator fait une app pour le simulateur d'iPhone et d'iPad de Xcode.
sidebar:
  order: 4
---

Un programme Kova peut être une app d'iPhone et d'iPad. Elle tourne dans le **simulateur** de Xcode, sur un Mac avec une puce Apple,
ou sur un **vrai iPhone** une fois signée avec ton propre certificat Apple (plus bas).

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

Une fenêtre défile quand elle dépasse l'écran, `ui.canvas` dessine avec Core Graphics et entend les
touchers, les dialogues sont des alertes et le son passe par `AVAudioPlayer`.

La fenêtre est dans une barre de navigation : son titre est celui de la fenêtre, et chaque menu est un
bouton de la barre qui ouvre ses éléments (les raccourcis sont pour un clavier, qu'un iPhone n'a pas).
`ui.tabs` est une vraie tab bar. Les programmes sont liés à iOS 26 : les contrôles du système et les deux
barres ont l'aspect Liquid Glass. Un canvas plus large que l'écran défile sur
le côté.

`ui.gpu` tourne sur OpenGL ES 3.0, que tous les iPhone ont : tes shaders gardent `#version 330 core`,
Kova leur donne l'en-tête qu'iOS veut. Apple n'améliore plus OpenGL ES, et le fait toujours tourner.

Pas encore là : les dialogues de fichier (`ui.open_file`, `ui.save_file`).

## Sur un vrai iPhone

Apple ne lance que des apps signées avec un certificat qu'il a fait pour toi. Un Apple ID gratuit
suffit.

### Sur un Mac : rien à donner à Kova

Ajoute ton Apple ID dans Xcode une fois (Réglages → Comptes), branche l'iPhone avec le Mode
développeur activé (Réglages → Confidentialité et sécurité), puis :

```console
$ kova build compteur.kova --target ios --run
```

Kova demande à Xcode le profil de `dev.kova.compteur` (il en fait un la première fois), signe l'app
avec ton certificat du trousseau, l'installe sur l'iPhone et la lance, en montrant ce que le
programme affiche. Lance-le depuis un Terminal sur le Mac : la première fois, macOS demande si
`codesign` peut utiliser ta clé (choisis Toujours autoriser). Sans `--run`, tu obtiens
`compteur.app` signée et tu l'installes toi-même.

L'iPhone peut aussi être sur le même Wi-Fi plutôt que sur un câble : dans Xcode, Window → Devices
and Simulators, sélectionne-le une fois et coche Connect via network.

### Sur n'importe quel ordinateur

Kova signe tout seul avec un profil et un fichier d'identité, sous Linux, Windows ou Mac. Ce que
veut Apple, tu le fais **une seule fois, sur un Mac** :

1. Branche l'iPhone, et active Réglages → Confidentialité et sécurité → Mode développeur.
2. Dans Xcode, ajoute ton Apple ID (Réglages → Comptes) et crée un projet iOS App vide dont
   l'identifiant est celui de ton programme : `dev.kova.` et son nom en minuscules (un programme
   avec `header { name := "essai" }` est `dev.kova.essai`). Choisis ton compte dans Signing &
   Capabilities et lance-le une fois sur l'iPhone. Xcode fait le certificat et le profil.
3. Sur l'iPhone, fais-toi confiance dans Réglages → Général → VPN et gestion de l'appareil.
4. Donne à Kova les deux choses que Xcode a faites. Le profil est le fichier `.mobileprovision`
   de `~/Library/Developer/Xcode/UserData/Provisioning Profiles`. L'identité est ta clé et ton
   certificat dans un fichier PEM, à partir de l'export (`.p12`) de l'Accès au trousseau :

```console
$ openssl pkcs12 -legacy -in identite.p12 -nodes -out ios.pem
```

Ensuite, sur n'importe quel ordinateur :

```console
$ kova build compteur.kova --target ios --profile app.mobileprovision --identity ios.pem
✓ compteur.app (iOS, 230 Ko) en 6 s
```

Installe-la depuis un Mac avec `xcrun devicectl device install app --device <nom> compteur.app`.

Kova vérifie que le profil autorise le nom du programme (`dev.kova.compteur`) et qu'il n'a pas
expiré. **Un profil gratuit dure 7 jours** : ensuite l'app ne s'ouvre plus et tu la refais avec un
nouveau profil. Un compte développeur payant (99 $ par an) dure un an, et son profil peut autoriser
tous les noms qui commencent par `dev.kova.`. Garde `ios.pem` privé : c'est la clé qui signe tes
apps.

## Essayer un programme sans le doigt

La ligne de commande du simulateur ne sait pas toucher l'écran. Pour les tests, `KOVA_UI_TAP=0,0,1`
fait appuyer l'app sur ses propres boutons, par numéro, un à chaque fois que la fenêtre est
dessinée :

```console
$ SIMCTL_CHILD_KOVA_UI_TAP=0,0,1 xcrun simctl launch booted dev.kova.compteur
```
