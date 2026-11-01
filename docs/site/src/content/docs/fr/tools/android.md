---
title: Kova sur Android
description: kova build --target android produit un seul fichier .apk, sans rien installer.
sidebar:
  order: 3
---

Un programme Kova peut être une app Android. Kova n'a besoin ni des outils d'Android ni de Java :

```console
$ kova build counter.kova --target android
✓ counter.apk (Android, 391 Ko) en 1,4 s
```

`counter.apk` contient ton programme compilé pour les téléphones (arm64) et pour les émulateurs et
Chromebooks (x86-64), et il est signé. Il fonctionne à partir d'Android 8.0.

| Cible | Ce que contient le `.apk` |
|---|---|
| `android` | arm64 et x86-64 |
| `android-arm` | arm64 seul : plus petit, tous les téléphones récents |
| `android-intel` | x86-64 seul : émulateurs, certains Chromebooks |

## Le mettre sur un téléphone

Android n'installe les apps venues d'ailleurs que de son store si tu l'autorises :

1. Envoie `counter.apk` sur le téléphone : par câble, par mail, par un dossier partagé.
2. Ouvre le fichier sur le téléphone.
3. Android demande si cette source peut installer des apps : autorise-la, puis installe.

Si tu as l'outil `adb` d'Android, `adb install counter.apk` fait la même chose.

## Ce qu'un programme trouve sur un téléphone

| Dans le programme | Sur le téléphone |
|---|---|
| `print` | un écran de texte ; aussi `adb logcat -s kova` |
| `read_line` | un champ apparaît sous le texte |
| `import ui` | de vraies vues Android : boutons, champs, cases à cocher, curseurs, listes |
| `ui.canvas` | une vue dessinée par le programme ; `on_mouse` reçoit le doigt |
| `ui.gpu` | OpenGL ES 3 : les shaders écrits pour OpenGL 3.3 sont adaptés à la compilation |
| `view.menus` | le menu de la barre de titre |
| `ui.message`, `ui.confirm` | les dialogues d'Android |
| `ui.open_file` | le sélecteur de fichiers d'Android ; le programme en reçoit une copie |
| `import sound` | le téléphone joue le WAV |
| `web.get`, `web.post`, `web.send` | les connexions d'Android, donc HTTPS utilise ses certificats |
| `import files` | le dossier privé de l'app, qui est aussi le dossier de travail |

Le programme est du vrai code machine : les fils, `web.serve` et `net` marchent comme sur Linux. Sa
langue est celle du téléphone, sauf si le `header` en décide autrement. Les tailles que tu écris
sont indépendantes de la densité de l'écran : une vue a le même aspect partout.

`header { name := "Counter" }` nomme l'app et le fichier, et `icon := "counter.svg"` lui donne son
icône, comme sur les autres systèmes.

## Comment ça se comporte

Une app n'a pas de fin à elle : Android la fait tourner, et l'utilisateur la quitte avec le geste
retour.

Un programme sans fenêtre garde son texte à l'écran quand il se termine, jusqu'à ce que
l'utilisateur quitte. Un programme avec fenêtre se ferme quand `ui.run` rend la main ; s'il s'arrête
sur une erreur, l'erreur reste à l'écran. Quitter une fenêtre fait revenir `ui.run`, comme fermer
une fenêtre sur un ordinateur.

## Pourquoi il ne faut aucun outil

Un `.apk` est un fichier zip. Kova écrit lui-même tout ce qu'il contient : le programme sous forme
de bibliothèque, la description de l'app dans le format binaire d'Android, et la signature. La
partie Java d'une app, qui transforme une vue Kova en vues Android, est déjà compilée dans `kova`.

## Ta clé

Android refuse de mettre à jour une app signée par une autre clé. La première compilation
`--target android` crée ta clé, dans `~/.config/kova/android.key` (`%APPDATA%\kova\android.key`
sur Windows) :

- garde-la, sinon tes téléphones refuseront les mises à jour de tes apps tant qu'elles ne sont pas
  désinstallées ;
- ne la partage jamais et ne la mets pas dans un projet ;
- pour l'utiliser sur une autre machine, copie ce fichier, ou indique-le avec `KOVA_ANDROID_KEY`.

## Ce qui n'existe pas encore

- **Le Play Store** demande un autre format et des numéros de version : pour l'instant, les apps
  s'installent depuis le fichier. Une app est toujours en version 1.0, nommée `dev.kova.app.<nom>`.
- **`ui.image` et `d.image`** lisent les fichiers que le programme peut atteindre : rien n'est encore
  embarqué dans l'app.
- `ui.save_file` donne un chemin dans le dossier de l'app, que l'app Fichiers peut ouvrir.
  `ui.open_folder` répond `none`.
- `net` n'a pas de TLS ici : ses connexions sécurisées échouent, pas celles de `web.get`.
- `ui.place` et `ui.animate` sur la fenêtre ne font rien : une app remplit son écran.
- `extern` n'atteint que les bibliothèques qu'Android laisse charger aux apps.
