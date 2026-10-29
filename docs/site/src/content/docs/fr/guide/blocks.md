---
title: Blocs
description: Des morceaux de code donnés à une fonction, qui les exécute.
sidebar:
  order: 11
---

Un **bloc** est un morceau de code donné à une fonction, qui l'exécute quand elle veut, autant de
fois qu'elle veut. Il s'écrit entre accolades, avec ses paramètres avant `=>` :

```kova
fn main {
    let scores: List<Int> := [12, 5, 18, 7]
    print(scores.filter { x: Int => x > 10 })
    print(scores.map { x: Int => x * 2 })
    print(scores.any { x: Int => x > 15 })
    print(scores.all { x: Int => x > 0 })
    print(scores.fold(0) { total: Int, x: Int => total + x })
}
```

```text
[12, 18]
[24, 10, 36, 14]
true
true
42
```

- `filter` garde les éléments pour lesquels le bloc donne `true`.
- `map` donne une liste de ce que le bloc donne pour chaque élément.
- `any` et `all` disent si le bloc donne `true` pour un élément, ou pour tous.
- `fold(départ)` porte une valeur d'élément en élément : ici, un total qui grandit.
- `each` exécute le bloc sur chaque élément ; `sorted { a: Int, b: Int => … }` trie avec le bloc
  comme ordre.

La dernière ligne d'un bloc est sa valeur. Un bloc qui ne donne rien, comme avec `each`, peut
contenir n'importe quelles instructions.

## Les blocs voient ce qui les entoure

Un bloc peut lire les variables autour de lui, et les changer :

```kova
fn main {
    let limit: Int := 10
    var count: Int := 0
    [12, 5, 18, 7].each { x: Int =>
        if x > limit {
            count += 1
        }
    }
    print(count)
}
```

```text
2
```

## Écrire une fonction qui prend un bloc

Un paramètre de type bloc s'écrit avec les types des paramètres du bloc, et son résultat après
`->` s'il en donne un : `{Int}` prend un `Int` et ne donne rien, `{Int -> Bool}` donne un `Bool`.
La fonction l'appelle comme une fonction :

```kova
fn repeat(n: Int, body: {Int}) {
    from i := 1 to n {
        body(i)
    }
}

fn keep(l: List<Int>, wanted: {Int -> Bool}) -> List<Int> {
    var out: List<Int> := []
    for x in l {
        if wanted(x) {
            out.add(x)
        }
    }
    return out
}

fn main {
    repeat(3) { i: Int =>
        print("turn {i}")
    }
    print(keep([1, 8, 3, 9]) { x: Int => x > 5 })
}
```

```text
turn 1
turn 2
turn 3
[8, 9]
```

Quand le bloc est le dernier argument, il s'écrit après les parenthèses de l'appel, sur la même
ligne : `repeat(3) { … }` plutôt que `repeat(3, { … })`. Les deux sont acceptés. C'est ce qui fait
que `web.serve(8080) { q: web.Request => … }` se lit comme une instruction du langage, alors que
c'est une fonction de la bibliothèque.

## Ce qu'un bloc ne peut pas faire

Un bloc s'exécute à l'intérieur d'une autre fonction, à un moment choisi par elle. Donc :

- `return`, `break`, `continue` et `?` ne peuvent pas s'écrire dans un bloc : ils quitteraient une
  fonction ou une boucle qui ne tourne peut-être plus. La valeur du bloc est sa dernière ligne ;
- un bloc ne s'écrit qu'en argument. Il n'est jamais rangé dans une variable ou une struct, donc il
  ne peut jamais survivre aux variables qu'il utilise :

```kova
fn main {
    let double := { x: Int => x * 2 }
}
```

```txt
erreur: un bloc `{ … }` ne s'écrit qu'en argument d'une fonction : `l.each { x: Int => print(x) }`
 --> main.kova:2:19
  |
2 |     let double := { x: Int => x * 2 }
  |                   ^
```

Ces deux règles rendent les blocs simples à suivre : un bloc s'exécute toujours pendant l'appel
auquel on le donne, et quand l'appel se termine, le bloc disparaît. Quand du code doit être gardé
et choisi plus tard, une [enum](/fr/guide/enums-and-match/) qui liste les actions possibles, avec
un `match`, dit la même chose plus clairement.

## Des blocs sur plusieurs fils

`l.map workers(4) { … }` et `l.each workers(4) { … }` répartissent les éléments entre 4 fils
d'exécution : [Plusieurs choses à la fois](/fr/guide/concurrency/).
