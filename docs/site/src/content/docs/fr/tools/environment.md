---
title: Langues et environnement
description: Kova en anglais ou en français, et les variables d'environnement qu'il lit.
sidebar:
  order: 3
---

## Kova parle anglais et français

Chaque message de Kova, des erreurs du compilateur aux erreurs d'un programme compilé pendant qu'il
tourne, existe en anglais et en français :

```sh
KOVA_LANG=fr kova run app.kova
```

```console
erreur: types différents : `String` attendu, `Int` donné
```

La langue est choisie, dans cet ordre, par :

1. `header { lang := fr }` en haut du programme ;
2. `KOVA_LANG` (`en`, `fr`) ;
3. la langue du système : `LC_ALL`, `LC_MESSAGES`, `LANG` ;
4. l'anglais.

Un exécutable compilé emporte toutes les langues et choisit la sienne au démarrage, donc le même
fichier parle français sur un ordinateur en français et anglais ailleurs.

Les messages se trouvent dans `locales/en.kova` et `locales/fr.kova`, écrits en Kova lui-même.
Lancer un fichier de traduction, `kova run locales/fr.kova`, ne lance rien : il liste les messages
qui manquent encore par rapport aux autres langues. Une nouvelle langue, c'est un nouveau fichier
là, plus une ligne dans la liste des langues du compilateur (`LANGUAGES` dans `src/locale.rs`).

## Variables d'environnement

| Variable | Effet |
| --- | --- |
| `KOVA_LANG` | la langue des messages de Kova : `en`, `fr` |
| `KOVA_SEED` | un nombre : `random`, `shuffled` et `chosen` répètent les mêmes tirages (jamais `secure`) |
| `KOVA_LOG` | le niveau de [`log`](/fr/library/log/) : `debug`, `info`, `warning`, `error` ; il l'emporte sur le programme |
| `NO_COLOR` | définie : pas de [couleurs](/fr/library/string/#les-couleurs), même dans un terminal |
| `FORCE_COLOR` | définie : des couleurs même quand la sortie n'est pas un terminal |
| `KOVA_UI_SCRIPT` | `1` : un programme [`ui`](/fr/library/ui/) n'ouvre pas de fenêtre, affiche ses vues et lit ses clics sur son entrée |
| `KOVA_UI_HIDDEN` | `1` : la fenêtre n'est jamais montrée (Windows, macOS) |
| `KOVA_LEAKS` | `1` : un texte jamais libéré arrête le programme avec le code 102 (utilisé par les tests de Kova) |
| `TZ` | le fuseau horaire de `Date.today` et `DateTime.now`, comme partout sous Linux et macOS |
