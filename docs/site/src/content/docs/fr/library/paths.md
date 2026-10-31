---
title: paths
description: Découper et assembler des chemins de fichiers.
sidebar:
  order: 16
---

```kova
import paths
```

Un chemin est un simple `String`. Ces fonctions le découpent et l'assemblent sans toucher au disque,
et elles comprennent `/` comme le `\` de Windows.

| Fonction | Ce qu'elle fait |
| --- | --- |
| `paths.join(first, second) -> String` | `second` à l'intérieur de `first`. Un `second` absolu l'emporte, comme si tu changeais de dossier. |
| `paths.folder(path) -> String` | la partie dossier, vide s'il n'y en a pas. |
| `paths.name(path) -> String` | la dernière partie, le nom du fichier. |
| `paths.stem(path) -> String` | le nom sans son extension. |
| `paths.extension(path) -> String` | l'extension sans son point, vide s'il n'y en a pas. |
| `paths.absolute(path) -> Bool` | s'il part de la racine d'un disque. |

```kova
import paths

fn main {
    let report: String := paths.join("data/2026", "report.final.csv")
    print(report)
    print(paths.folder(report))
    print(paths.name(report))
    print(paths.stem(report))
    print(paths.extension(report))
    print(paths.stem(".bashrc"))
}
```

```text
data/2026/report.final.csv
data/2026
report.final.csv
report.final
csv
.bashrc
```

Un nom qui commence par un point, comme `.bashrc`, n'a pas d'extension : le point fait partie de son
nom.
