---
title: log
description: Des lignes datées sur la sortie d'erreur, par niveau, choisi par qui lance le programme.
sidebar:
  order: 13
---

```kova
import log
```

`print` écrit ce que le programme **donne** ; `log` écrit ce qu'il **fait**, pour qui le regarde
tourner : une ligne sur la sortie d'erreur, avec l'heure et le niveau.

```kova
import log

fn main {
    log.info("started")
    log.debug("not shown")
    log.warning("disk almost full")
    print("the result")
}
```

```text
the result
```

Sur la sortie d'erreur :

```console
14:05:09  info     started
14:05:09  warning  disk almost full
```

Comme les lignes vont sur la sortie d'erreur, `./tool > result.txt` les garde hors du fichier.

| Fonction | Écrit |
| --- | --- |
| `log.debug(text)` | des détails pour trouver un bug |
| `log.info(text)` | ce que fait le programme |
| `log.warning(text)` | quelque chose qui ne va pas mais avec quoi il peut continuer |
| `log.error(text)` | quelque chose qui ne va pas et avec quoi il ne peut pas |
| `log.level(log.Level.Debug)` | à partir de là, chaque ligne de ce niveau et au-dessus |

Sans `log.level`, les lignes à partir de `info` sont écrites. `log.Level` vaut `Debug`, `Info`,
`Warning` ou `Error`.

Qui lance le programme choisit aussi, sans le changer : `KOVA_LOG=debug ./tool` montre tout,
`KOVA_LOG=error ./tool` seulement les erreurs. `KOVA_LOG` l'emporte sur `log.level`.
