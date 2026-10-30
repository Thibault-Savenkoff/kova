---
title: Choix et boucles
description: if, match sur une valeur, while, for et from … to.
sidebar:
  order: 3
---

## `if`

```kova
fn main {
    let temperature: Int := 23
    if temperature > 30 {
        print("hot")
    } else if temperature > 15 {
        print("mild")
    } else {
        print("cold")
    }
}
```

```text
mild
```

La condition est un `Bool`, sans parenthèses ; les accolades sont toujours écrites, même pour une
ligne, pour qu'une deuxième ligne ajoutée ne puisse jamais tomber hors du `if` par erreur.

`if` **donne aussi une valeur**, ce qui remplace le `? :` d'autres langages :

```kova
fn sign(n: Int) -> String {
    return if n < 0 { "negative" } else if n = 0 { "zero" } else { "positive" }
}

fn main {
    print(sign(-4))
    print(sign(0))
}
```

```text
negative
zero
```

Un `if` qui donne une valeur a besoin d'un `else` : la valeur doit exister dans tous les cas.

## `match` sur une valeur

Quand une valeur est comparée à plusieurs possibilités, `match` les liste, une par ligne, chacune
suivie de `=>` et de ce qu'il faut faire. Les cas peuvent être des nombres, des textes, des
caractères, des intervalles, ou plusieurs d'entre eux séparés par des virgules ; `_` est tout le
reste :

```kova
fn size(n: Int) -> String {
    return match n {
        0 => "none"
        1..=9 => "small"
        10, 20, 30 => "round"
        _ => "big"
    }
}

fn main {
    print(size(4))
    print(size(20))
    print(size(512))
    match "stop" {
        "go" => print("going")
        "stop" => print("stopping")
        _ => print("unknown")
    }
}
```

```text
small
round
big
stopping
```

Un `match` doit couvrir toutes les valeurs : sans `_`, un `match` sur un `Int` est refusé, et deux
cas qui se recouvrent (`1..=9` et `5`) aussi, puisque le second ne pourrait jamais être atteint.
`match` brille vraiment avec les [enums](/fr/guide/enums-and-match/), dont le compilateur connaît
chaque cas.

## `while`

`while` répète tant que sa condition est `true` :

```kova
fn main {
    var n: Int := 1
    while n < 100 {
        n := n * 3
    }
    print(n)
}
```

```text
243
```

## `for` et les intervalles

`for` parcourt un intervalle de nombres, une [liste](/fr/guide/collections/), une map ou un set :

```kova
fn main {
    for i in 0..3 {
        write(i)
    }
    print("")
    for i in 0..=3 {
        write(i)
    }
    print("")
    from i := 1 to 3 {
        write(i)
    }
    print("")
    for name in ["Ada", "Alan"] {
        print(name)
    }
}
```

```text
012
0123
123
Ada
Alan
```

- `0..3` s'arrête **avant** 3, ce qui convient aux positions dans une liste (`0..l.length`).
- `0..=3` et `from i := 1 to 3` incluent leur fin, ce qui convient pour compter.
- Un intervalle dont la fin est sous le début est vide : `3..1` n'exécute rien. Les bornes sont
  calculées une fois, avant le premier tour.
- `write` affiche sans passer à la ligne ; `print` passe à la ligne.

Avec deux noms, `for` donne aussi la place de chaque élément, comptée depuis 0 :

```kova
fn main {
    for i, name in ["Ada", "Alan"] {
        print("{i}: {name}")
    }
}
```

```text
0: Ada
1: Alan
```

## `break` et `continue`

`break` quitte la boucle tout de suite, `continue` passe au tour suivant :

```kova
fn main {
    var total: Int := 0
    for i in 1..=10 {
        if i % 2 = 0 {
            continue
        }
        if i > 7 {
            break
        }
        total += i
    }
    print(total)
}
```

```text
16
```

(1 + 3 + 5 + 7.) `return` quitte toute la fonction, depuis n'importe quelle boucle.
