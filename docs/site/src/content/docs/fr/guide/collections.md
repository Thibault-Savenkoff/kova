---
title: Listes, Maps et Sets
description: Les trois collections, comment les remplir, les lire et les parcourir.
sidebar:
  order: 8
---

Kova a trois collections, chacune écrite avec des crochets :

| Type | Contient | S'écrit |
| --- | --- | --- |
| `List<T>` | des valeurs dans l'ordre, atteintes par leur position | `[12, 5, 18]` |
| `Map<K, V>` | une valeur pour chaque clé | `["Ana" := 31, "Bo" := 25]` |
| `Set<T>` | des valeurs, chacune au plus une fois | `Set["red", "blue"]` |

Le type entre `<…>` dit ce qu'elles contiennent, et tout ce qu'il y a dedans doit être de ce type.
Les trois possèdent ce qu'elles contiennent : y mettre une valeur l'y
[déplace](/fr/guide/ownership/), et la collection libère ses éléments quand elle disparaît.

## Les listes

```kova
fn main {
    var scores: List<Int> := [12, 5, 18]
    scores.add(7)
    print(scores)
    print(scores.length)
    print(scores[0])
    scores[1] := 6
    print(scores.remove(0))
    print(scores)
    print(scores.sorted)
    print(scores[1..3])
}
```

```text
[12, 5, 18, 7]
4
12
12
[6, 18, 7]
[6, 7, 18]
[18, 7]
```

- Les positions commencent à 0. `l[i]` lit, `l[i] := v` remplace.
- Une position après la fin arrête le programme (`indice hors de la liste`, code de sortie 101) au lieu
  de lire n'importe quelle mémoire.
- `l.add(v)` met `v` à la fin, `l.remove(i)` sort l'élément en `i` et le donne.
- `l[1..3]` est une nouvelle liste avec des copies des éléments 1 et 2 ; `l[1..=3]` inclut 3.
- `l.sorted` donne une copie triée ; `l.sorted { a: Int, b: Int => a > b }` trie selon ton ordre.
- Une liste vide s'écrit `[]`, là où son type est connu : `let empty: List<Int> := []`.

### Parcourir une liste

```kova
fn main {
    var prices: List<Int> := [10, 25, 40]
    for p in prices {
        print(p)
    }
    for p in var prices {
        p := p * 2
    }
    print(prices)
    var total: Int := 0
    for p in move prices {
        total += p
    }
    print(total)
}
```

```text
10
25
40
[20, 50, 80]
150
```

- `for p in prices` lit chaque élément ; la liste ne peut pas changer pendant la boucle.
- `for p in var prices` peut changer chaque élément sur place.
- `for p in move prices` sort chaque élément ; la liste est consommée après.

Les listes prennent aussi des [blocs](/fr/guide/blocks/) : `l.filter { x: Int => x > 10 }`,
`l.map { x: Int => x * 2 }`, `l.any`, `l.all`, `l.fold`, `l.each`. Toutes les méthodes sont dans
[List](/fr/library/list/).

## Les maps

Une `Map` retrouve une valeur à partir de sa clé, vite, quel que soit le nombre d'entrées :

```kova
fn main {
    var ages: Map<String, Int> := ["Ana" := 31, "Bo" := 25]
    ages["Cy"] := 40
    ages["Bo"] := 26
    print(ages["Bo"])
    print(ages["Zed"])
    print(ages.contains("Ana"))
    for name, age in ages {
        print("{name}: {age}")
    }
}
```

```text
26
none
true
Ana: 31
Bo: 26
Cy: 40
```

- `m[k] := v` ajoute la clé ou remplace sa valeur.
- `m[k]` est un `V?` : la clé peut manquer, et Kova te fait dire quoi faire dans ce cas (voir
  [Valeurs absentes](/fr/guide/errors/)), au lieu d'inventer une valeur ou de planter.
- Les clés sont gardées dans l'ordre où elles ont été ajoutées la première fois, donc un programme
  parcourt toujours une map de la même façon.
- `m.remove(k)` sort une entrée et donne sa valeur (`none` s'il n'y en avait pas) ; `m.keys` et
  `m.values` donnent des listes.

Compter est l'usage classique. Comme `counts[w]` peut manquer, un `match` dit quoi faire dans
chaque cas :

```kova
fn main {
    let text: String := "the cat and the dog and the bird"
    var counts: Map<String, Int> := []
    for w in text.words {
        match counts[w] {
            none => counts[w] := 1
            n => counts[w] := n + 1
        }
    }
    print(counts)
}
```

```text
["the" := 3, "cat" := 1, "and" := 2, "dog" := 1, "bird" := 1]
```

Les clés peuvent être de n'importe quel type comparable avec `=` : nombres, textes, dates, et tes
propres types qui suivent `Equal`.

## Les sets

Un `Set` garde chaque valeur une fois, et dit vite s'il en contient une :

```kova
fn main {
    var seen: Set<String> := Set["red", "blue"]
    seen.add("red")
    seen.add("green")
    print(seen)
    print(seen.contains("blue"))
    print(seen.length)
}
```

```text
Set["red", "blue", "green"]
true
3
```

`Set[…]` s'écrit avec son nom, pour que `[…]` désigne toujours une liste ou une map. Comme une map,
un set garde l'ordre dans lequel les valeurs ont été ajoutées la première fois.

## Comparer des collections

`=` compare deux listes élément par élément : même longueur, mêmes éléments, dans le même ordre. En
Kova, une liste est une valeur, donc c'est bien ce que veut dire « égales ». `+` crée une nouvelle
liste, `+=` ajoute à une liste :

```kova
fn main {
    let a: List<Int> := [1, 2]
    print(a = [1, 2])
    print(a = [2, 1])
    var b: List<Int> := a + [3]
    b += [4, 5]
    print(b)
}
```

```text
true
false
[1, 2, 3, 4, 5]
```

Les éléments doivent eux-mêmes pouvoir se comparer, comme les clés d'une map. Les maps et les sets
n'ont pas encore de `=`.
