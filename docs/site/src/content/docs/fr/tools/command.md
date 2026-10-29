---
title: La commande kova
description: run, build, test, fmt, les cibles, les options et les codes de sortie.
sidebar:
  order: 1
---

Tout passe par une seule commande :

| Commande | Fait |
| --- | --- |
| `kova run app.kova [mots…]` | vérifie le programme et le lance tout de suite ; les mots deviennent ses `args` |
| `kova build app.kova` | fait un exécutable natif à côté du fichier |
| `kova test app.kova ["nom"]` | lance les blocs `test` du programme, ou seulement celui nommé |
| `kova fmt app.kova…` | met les fichiers en page de la façon standard, sur place |
| `kova app.kova` | fait ce que dit le [header](/fr/guide/run-or-build/#le-header) du programme, `run` par défaut |
| `kova lsp` | le serveur de langage, lancé par l'éditeur |
| `kova debug` | le débogueur, lancé par l'éditeur |
| `kova --version` | la version de Kova |

Chaque commande lit toute seule les fichiers que le programme importe. Il n'y a pas de fichier de
projet.

## `kova build`

| Option | Fait |
| --- | --- |
| `-o nom` | nomme l'exécutable (par défaut : le nom du fichier, avec `.exe` ajouté sous Windows) |
| `--boost` | laisse de côté les contrats (`require`, `ensure`, `check`), pour la vitesse |
| `--target système` | compile pour un autre système (ci-dessous) |

Une fois fini, il dit ce qu'il a fait, pour quel système, et en combien de temps :

```console
✓ app (Linux, 29 Ko) en 3 ms
```

L'exécutable n'a besoin de rien d'installé pour tourner, sauf GTK 4 sous Linux pour un programme à
fenêtres et OpenSSL sous Linux pour `https://`. Kova l'écrit entièrement lui-même : pas de
compilateur C, pas d'éditeur de liens, pas de SDK, sur aucun système.

### Les cibles

Depuis n'importe quel système, `--target` compile pour n'importe quel autre :

| Cible | Pour |
| --- | --- |
| `linux` | Linux sur x86-64 (Intel, AMD) |
| `linux-arm` | Linux sur arm64 (Raspberry Pi 4 et suivants, serveurs ARM) |
| `windows` | Windows sur x86-64 |
| `windows-arm` | Windows sur arm64 |
| `mac` | tous les Mac, dans un seul fichier (puces Apple et Intel) ; par défaut sur un Mac |
| `mac-arm` | seulement les Mac à puce Apple |
| `mac-intel` | seulement les Mac Intel |

Chaque cible a besoin d'un petit morceau de Kova compilé pour elle, ajouté à Rust avant d'installer
Kova ([Installation](/fr/start/install/#3-compiler-pour-dautres-systèmes-facultatif)) ; sinon
`kova build` dit lequel manque.

## `kova test`

Lance chaque bloc `test` du fichier séparément, et écrit ✅ ou ❌ avec le message du premier
`check` rompu ([Contrats et tests](/fr/guide/contracts-and-tests/#les-tests)). `kova test` compte
comme `kova run` pour [`mode`](/fr/guide/run-or-build/#une-fonction-qui-diffère--mode).

## `kova fmt`

Met le code en page de la façon standard : indentation et espaces. Il ne déplace jamais un saut de
ligne, et refuse de changer un fichier si le résultat ne voulait pas dire exactement le même
programme. L'éditeur le lance à l'enregistrement.

## Les codes de sortie

| Code | Veut dire |
| --- | --- |
| `0` | le programme s'est terminé normalement |
| `1` | Kova a refusé le programme (une erreur dans le code), ou un test a échoué |
| `101` | le programme s'est arrêté sur une erreur pendant qu'il tournait : un indice hors de la liste, un dépassement, un contrat rompu… |
| autre | ce que le programme a donné à `exit(code)` |

Un exécutable compilé se termine avec les mêmes codes.
