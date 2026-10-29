---
title: Languages and environment
description: Kova in English or French, and the environment variables it reads.
sidebar:
  order: 3
---

## Kova speaks English and French

Every message of Kova, from the compiler's errors to a built program's runtime errors, exists in
English and in French:

```sh
KOVA_LANG=fr kova run app.kova
```

```console
erreur: types différents : `String` attendu, `Int` donné
```

The language is chosen, in this order, by:

1. `header { lang := fr }` at the top of the program;
2. `KOVA_LANG` (`en`, `fr`);
3. the system's language: `LC_ALL`, `LC_MESSAGES`, `LANG`;
4. English.

A built executable carries every language and picks its own when it starts, so the same file
speaks French on a French computer and English elsewhere.

The messages live in `locales/en.kova` and `locales/fr.kova`, written in Kova itself. To add a
language, copy `en.kova`, translate it, and run `kova run xx.kova`: instead of running anything, it
lists the messages still missing.

## Environment variables

| Variable | Effect |
| --- | --- |
| `KOVA_LANG` | the language of Kova's messages: `en`, `fr` |
| `KOVA_SEED` | a number: `random`, `shuffled` and `chosen` repeat the same draws (never `secure`) |
| `KOVA_LOG` | the level of [`log`](/library/log/): `debug`, `info`, `warning`, `error`; it wins over the program |
| `NO_COLOR` | set: no [colors](/library/string/#colors), even on a terminal |
| `FORCE_COLOR` | set: colors even when the output is not a terminal |
| `KOVA_UI_SCRIPT` | `1`: a [`ui`](/library/ui/) program opens no window, prints its views and reads its clicks from its input |
| `KOVA_UI_HIDDEN` | `1`: the window is never shown (Windows, macOS) |
| `KOVA_LEAKS` | `1`: a text never freed stops the program with the code 102 (used by Kova's own tests) |
| `TZ` | the time zone of `Date.today` and `DateTime.now`, as everywhere on Linux and macOS |
