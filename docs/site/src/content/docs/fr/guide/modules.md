---
title: Modules
description: Des programmes de plusieurs fichiers, ce que chaque fichier montre aux autres, et les modules de Kova.
sidebar:
  order: 13
---

Un programme grandit au-delà d'un fichier en en **important** d'autres. Chaque fichier est un
module : son nom est celui du fichier, et il ne montre aux autres que ce qu'il choisit.

```kova
// main.kova
import geometry

fn main {
    let p := geometry.Point { x := 3.0, y := 4.0 }
    print(geometry.length(p))
}
```

```kova
// geometry.kova, next to main.kova
public:

struct Point {
public:
    x: Float
    y: Float
}

fn length(p: Point) -> Float {
    return (square(p.x) + square(p.y)).sqrt
}

private:

fn square(x: Float) -> Float {
    return x * x
}
```

```text
5.0
```

`kova run main.kova` trouve `geometry.kova` tout seul : il n'y a pas de fichier de projet, pas de
liste de fichiers à tenir à jour. Les noms d'un module s'écrivent toujours avec lui,
`geometry.Point`, `geometry.length`, pour qu'un lecteur sache toujours d'où vient un nom.

## Public et privé

Dans un fichier importé, tout est **privé** sauf ce qui est écrit sous `public:` : seul ce fichier
peut s'en servir. `private:` cache à nouveau ce qui suit. Les champs d'une struct suivent la même
règle à l'intérieur de la struct, pour qu'un type puisse montrer ses méthodes et garder ses champs
pour lui.

```kova
// main.kova
import geometry

fn main {
    print(geometry.square(2.0))
}
```

```kova
// geometry.kova
fn square(x: Float) -> Float {
    return x * x
}
```

```txt
erreur: `geometry.square` est privé dans geometry.kova
 --> main.kova:5:11
  |
5 |     print(geometry.square(2.0))
  |           ^^^^^^^^^^^^^^^
  = aide : pour l'utiliser depuis un autre fichier, écris-le sous `public:` dans geometry.kova
```

Privé par défaut veut dire que l'intérieur d'un module peut changer sans casser les fichiers qui
l'utilisent : seul ce qui est public est une promesse. Le compilateur refuse aussi une fonction
publique qui prend ou donne un type privé, puisque personne dehors ne pourrait s'en servir.

## D'autres noms, d'autres dossiers

- `import geometry as geo` donne au module un nom plus court : `geo.Point`.
- Un fichier dans un dossier s'écrit avec des points : `import shapes.circle` lit
  `shapes/circle.kova`, et l'appelle `circle` (`circle.Circle`).
- Chaque `import` part du dossier du fichier principal du programme, même dans
  `shapes/circle.kova` : `import shapes.point`, jamais un chemin qui dépend de l'endroit où se
  trouve le fichier qui importe.
- Deux fichiers ne peuvent pas s'importer l'un l'autre, directement ou par d'autres : le
  compilateur montre le cercle.

## Les modules de Kova

Une partie de la bibliothèque est dans des modules fournis avec Kova. Ils demandent un `import`,
mais aucun fichier :

| Module | Pour |
| --- | --- |
| [`files`](/fr/library/files/) | lire et écrire des fichiers, des dossiers |
| [`system`](/fr/library/system/) | variables d'environnement, lancer d'autres programmes |
| [`web`](/fr/library/web/) | interroger des sites, et [répondre](/fr/library/web-server/) en serveur |
| [`secure`](/fr/library/secure/) | hasard sûr, empreintes de mots de passe, chiffrement |
| [`log`](/fr/library/log/) | des lignes datées sur la sortie d'erreur, par niveau |
| [`ui`](/fr/library/ui/) | des fenêtres natives |

```kova
import files

fn main {
    print(files.exists("surely-not-here.txt"))
}
```

```text
false
```

Sans l'`import`, le compilateur dit lequel écrire. Ce que chaque programme a sans import (`print`,
`args`, `random`, textes, listes, dates, JSON…) est listé dans
[la bibliothèque](/fr/library/overview/).

## Tests et modules

Un module n'a pas de `main` : il ne se lance pas seul. `kova test geometry.kova` lance ses tests,
et l'éditeur le vérifie comme n'importe quel fichier.
