---
title: Vue d'ensemble
description: Ce que contient la bibliothèque, et ce qui demande un import.
sidebar:
  order: 1
---

Kova vient avec ce dont la plupart des programmes ont besoin, écrit pour Kova et identique sur
chaque système. Rien n'est téléchargé, et il n'y a pas de paquet à choisir parmi trois qui font
presque la même chose.

## Toujours là

| Page | Contient |
| --- | --- |
| [String](/fr/library/string/) | les textes : chercher, couper, motifs, conversions, base64, couleurs |
| [Nombres, Bool et Char](/fr/library/numbers/) | `Int`, `Float`, `BigInt` : opérateurs, maths |
| [List](/fr/library/list/) | listes ordonnées, blocs, tri, statistiques |
| [Map et Set](/fr/library/map-and-set/) | valeurs par clé, valeurs gardées une fois |
| [Dates et heures](/fr/library/dates/) | `Date`, `DateTime`, `Weekday` |
| [JSON et CSV](/fr/library/json-and-csv/) | les lire et les écrire dans tes propres structs |
| [system, args et le terminal](/fr/library/system/) | `print`, `read_line`, `system.args`, `system.exit` |
| [secure et le hasard](/fr/library/secure/) | `random` |

## Avec un import

| Module | Page | Contient |
| --- | --- | --- |
| `import files` | [files](/fr/library/files/) | lire et écrire des fichiers et des dossiers |
| `import system` | [system](/fr/library/system/#import-system) | variables d'environnement, lancer des programmes |
| `import web` | [web](/fr/library/web/), [serveur web](/fr/library/web-server/) | interroger des sites, répondre en serveur |
| `import secure` | [secure](/fr/library/secure/#import-secure) | hasard sûr, mots de passe, chiffrement |
| `import log` | [log](/fr/library/log/) | des lignes datées par niveau |
| `import threads` | [threads](/fr/library/threads/) | des fils, des canaux, des valeurs partagées |
| `import paths` | [paths](/fr/library/paths/) | découper et assembler des chemins de fichiers |
| `import url` | [url](/fr/library/url/) | encoder des textes pour les adresses web |
| `import arguments` | [arguments](/fr/library/arguments/) | lire la ligne de commande |
| `import toml` | [toml](/fr/library/toml/) | lire des fichiers de configuration |
| `import xml` | [xml](/fr/library/xml/) | lire et écrire du XML |
| `import compress`, `import zip` | [compress et zip](/fr/library/compress/) | gzip et archives zip |
| `import ui` | [ui](/fr/library/ui/) | des fenêtres natives |

## Chaque valeur a

`x.clone`, une copie que le programme possède ensuite.

`print(x)` montre n'importe quelle valeur, champs compris. Un texte pour les gens, `x.to_string` ou
`"{x}"` dans un texte, existe pour les nombres, `Bool`, `Char`, `String`, les dates, et les types
qui suivent `Display` ([Textes](/fr/guide/texts/)).

## Les interfaces intégrées

`Equal` (`=`), `Comparable` (`<`) et `Display` (`"{…}"`) sont décrites dans
[Interfaces et génériques](/fr/guide/interfaces-and-generics/#les-interfaces-intégrées).
