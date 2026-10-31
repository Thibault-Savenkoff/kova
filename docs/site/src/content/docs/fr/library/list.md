---
title: List
description: List<T>, une liste ordonnée de valeurs, et toutes ses méthodes.
sidebar:
  order: 4
---

Une `List<T>` garde des valeurs de type `T` dans l'ordre, atteintes par leur position à partir
de 0. Elle grandit selon les besoins. Le chapitre [Listes, Maps et Sets](/fr/guide/collections/)
du guide montre comment s'en servir ; cette page liste tout ce qu'elles ont.

## Faire une liste

| Écrit | Donne |
| --- | --- |
| `[1, 2, 3]` | une liste de ces valeurs |
| `[]` | une liste vide, là où son type est connu (`let l: List<Int> := []`) |
| `List<Int>()` | une liste vide, n'importe où |
| `l[a..b]`, `l[a..=b]` | une nouvelle liste avec des copies des éléments de `a` à `b` (non compris / compris) |
| `l.clone` | une copie de la liste et de ses éléments |

## Lire et changer

| Écrit | Fait |
| --- | --- |
| `l.length -> Int` | le nombre d'éléments. |
| `l.is_empty -> Bool` | si la liste n'a aucun élément. |
| `l.first -> T?` | le premier élément, `none` si la liste est vide. |
| `l.last -> T?` | le dernier élément, `none` si la liste est vide. |
| `l.contains(x: T) -> Bool` | si `x` est dans la liste, comparé avec `=`. |
| `l.min -> T?` | le plus petit élément, `none` si la liste est vide. |
| `l.max -> T?` | le plus grand élément, `none` si la liste est vide. |
| `l.sum -> T` | le total d'une `List<Int>` ou `List<Float>`, `0` si la liste est vide. |
| `l.reversed -> List<T>` | une nouvelle liste, dans l'ordre inverse. |
| `l[i]` | l'élément à la position `i`. Après la fin, le programme s'arrête. |
| `l[i] := v` | remplace l'élément en `i` (`l` doit être un `var`). |
| `a = b`, `a != b` | même longueur et mêmes éléments dans le même ordre. |
| `a + b` | une nouvelle liste : les éléments de `a`, puis ceux de `b`, copiés. |
| `l += other` | ajoute une copie de chaque élément de `other` à la fin. |
| `l.add(v)` | met `v` à la fin. Une valeur qui possède de la mémoire y est déplacée : `l.add(move name)`. |
| `l.remove(i) -> T` | retire l'élément en `i` et le donne ; ceux d'après reculent d'une place. |

Une liste ne peut pas changer pendant qu'un `for` la parcourt ; `for x in var l` change chaque
élément sur place, `for x in move l` les retire un par un.

## Avec un bloc

| Méthode | Donne |
| --- | --- |
| `l.each { x: T => … }` | lance le bloc sur chaque élément, dans l'ordre. |
| `l.filter { x: T => … } -> List<T>` | des copies des éléments pour lesquels le bloc donne `true`. |
| `l.map { x: T => … } -> List<U>` | ce que le bloc donne pour chaque élément, dans l'ordre. |
| `l.any { x: T => … } -> Bool` | si le bloc donne `true` pour au moins un élément (il s'arrête au premier). |
| `l.all { x: T => … } -> Bool` | s'il donne `true` pour chaque élément (il s'arrête au premier `false`). |
| `l.fold(start) { acc: U, x: T => … } -> U` | `start`, puis la valeur du bloc portée d'élément en élément. |
| `l.sorted { a: T, b: T => … } -> List<T>` | une copie triée, le bloc disant si `a` vient avant `b`. |
| `l.map workers(n) { … }`, `l.each workers(n) { … }` | la même chose, sur `n` fils d'exécution ([Plusieurs choses à la fois](/fr/guide/concurrency/)). |

```kova
fn main {
    let words: List<String> := ["kiwi", "fig", "banana"]
    print(words.filter { w: String => w.length > 3 })
    print(words.map { w: String => w.length })
    print(words.fold(0) { total: Int, w: String => total + w.length })
    print(words.sorted { a: String, b: String => a.length < b.length })
}
```

```text
["kiwi", "banana"]
[4, 3, 6]
13
["fig", "kiwi", "banana"]
```

## Ordre et hasard

| Méthode | Donne |
| --- | --- |
| `l.sorted -> List<T>` | une copie triée, le plus petit d'abord, pour un `T` qui suit `Comparable` (nombres, textes, dates…). Les éléments égaux gardent leur ordre. |
| `l.shuffled -> List<T>` | une copie dans un ordre au hasard. |
| `l.chosen -> T?` | un élément tiré au hasard, `none` si la liste est vide. |

`shuffled` et `chosen` utilisent le même hasard que `random`, donc `KOVA_SEED` les fait se
répéter. Pour ce qui ne doit pas pouvoir être deviné, utilise [`secure`](/fr/library/secure/).

## Nombres et textes

| Méthode | Sur | Donne |
| --- | --- | --- |
| `l.mean -> Float` | `List<Int>`, `List<Float>` | la moyenne. Une liste vide arrête le programme. |
| `l.median -> Float` | `List<Int>`, `List<Float>` | la valeur du milieu une fois triée (la moyenne des deux du milieu pour une longueur paire). |
| `l.join(separator: String) -> String` | `List<String>` | les textes avec `separator` entre chacun. |

```kova
fn main {
    let marks: List<Int> := [12, 15, 9, 18]
    print(marks.mean)
    print(marks.median)
    print(["a", "b", "c"].join(", "))
}
```

```text
13.5
13.5
a, b, c
```

## JSON et CSV

`books.to_json` et `List<Book>.from_json(text)` écrivent et lisent une liste en JSON ;
`fruits.to_csv` et `List<Fruit>.from_csv(text)` en tableau : [JSON et CSV](/fr/library/json-and-csv/).

## Affichage

`print(l)` montre `[1, 2, 3]`. Une liste n'a pas de texte pour les gens, donc `"{l}"` est refusé :
construis le texte avec `join` ou une boucle.
