---
title: Map et Set
description: Map<K, V>, une valeur pour chaque clé, et Set<T>, des valeurs gardées une fois chacune.
sidebar:
  order: 5
---

Les deux trouvent une clé vite, quelle que soit leur taille (elles utilisent une table de
hachage), et les deux gardent l'ordre dans lequel les clés ont été ajoutées la première fois,
donc un programme les parcourt de la même façon à chaque exécution.

Leurs clés peuvent être de n'importe quel type qui se compare avec `=` : nombres, `Char`,
`String`, dates, tuples de ceux-ci, et types qui suivent
[`Equal`](/fr/guide/interfaces-and-generics/#les-interfaces-intégrées).

## `Map<K, V>`

| Écrit | Fait |
| --- | --- |
| `["a" := 1, "b" := 2]` | une map de ces entrées ; `[]` en est une vide là où le type est connu |
| `m.length -> Int` | le nombre d'entrées. |
| `m.is_empty -> Bool` | si la Map n'a aucune entrée. |
| `m[k] -> V?` | la valeur de `k`, `none` s'il n'y en a pas. |
| `m[k] := v` | donne à `k` la valeur `v`, en ajoutant la clé si besoin (`m` doit être un `var`). |
| `m.contains(k: K) -> Bool` | si `k` a une valeur. |
| `m.remove(k: K) -> V?` | retire l'entrée de `k` et donne sa valeur, `none` s'il n'y en avait pas. |
| `m.keys -> List<K>` | des copies des clés, dans l'ordre. |
| `m.values -> List<V>` | des copies des valeurs, dans le même ordre. |
| `for k, v in m { … }` | parcourt les entrées dans l'ordre. |
| `m.each { k: K, v: V => … }` | exécute le bloc sur chaque entrée, dans l'ordre. |
| `m.filter { k: K, v: V => … } -> Map<K, V>` | une nouvelle Map des entrées pour lesquelles le bloc dit `true`. |
| `m.map { k: K, v: V => … } -> List<U>` | une List de ce que le bloc donne pour chaque entrée. |
| `m.any { k: K, v: V => … } -> Bool` | si le bloc dit `true` pour au moins une entrée. |
| `m.all { k: K, v: V => … } -> Bool` | si le bloc dit `true` pour toutes les entrées. |
| `m.fold(start) { total: U, k: K, v: V => … } -> U` | parcourt les entrées en portant un résultat. |
| `a = b`, `a != b` | les mêmes clés avec des valeurs égales, dans n'importe quel ordre. |
| `a + b -> Map<K, V>` | une nouvelle Map des deux ; quand les deux ont une clé, la valeur de `b` gagne. |
| `a += b` | ajoute les entrées de `b` à `a`, de la même façon. |
| `m.clone -> Map<K, V>` | une copie. |

```kova
fn main {
    var stock: Map<String, Int> := ["apple" := 12, "pear" := 4]
    stock["plum"] := 30
    stock["pear"] := 5
    print(stock["pear"])
    print(stock["kiwi"])
    print(stock.remove("apple"))
    print(stock.keys)
    for fruit, n in stock {
        print("{fruit}: {n}")
    }
}
```

```text
5
none
12
["pear", "plum"]
pear: 5
plum: 30
```

`m[k]` est un `V?`, donc on ne peut pas calculer directement avec : `m[k] += 1` est refusé,
puisque la clé peut manquer. Lis-la, puis décide :

```kova
fn main {
    var counts: Map<Char, Int> := []
    for c in "banana".chars {
        match counts[c] {
            none => counts[c] := 1
            n => counts[c] := n + 1
        }
    }
    print(counts)
}
```

```text
['b' := 1, 'a' := 3, 'n' := 2]
```

Une `Map<String, T>` se lit aussi depuis des objets JSON et s'y écrit :
[JSON et CSV](/fr/library/json-and-csv/).

## `Set<T>`

| Écrit | Fait |
| --- | --- |
| `Set["a", "b"]` | un set de ces valeurs (une valeur écrite deux fois est gardée une fois) ; `Set[]` est vide |
| `s.length -> Int` | le nombre de valeurs. |
| `s.is_empty -> Bool` | si le Set n'a aucune valeur. |
| `s.add(v)` | ajoute `v`, sauf s'il y est déjà. |
| `s.contains(v: T) -> Bool` | si `v` y est. |
| `s.remove(v: T) -> Bool` | retire `v` ; `true` s'il y était. |
| `for v in s { … }` | parcourt les valeurs, dans l'ordre où elles ont été ajoutées. |
| `s.each { x: T => … }` | exécute le bloc sur chaque valeur, dans l'ordre. |
| `s.filter { x: T => … } -> Set<T>` | un nouveau Set des valeurs pour lesquelles le bloc dit `true`. |
| `s.map { x: T => … } -> List<U>` | une List de ce que le bloc donne pour chaque valeur. |
| `s.any { x: T => … } -> Bool` | si le bloc dit `true` pour au moins une valeur. |
| `s.all { x: T => … } -> Bool` | si le bloc dit `true` pour toutes les valeurs. |
| `s.fold(start) { total: U, x: T => … } -> U` | parcourt les valeurs en portant un résultat. |
| `a = b`, `a != b` | les mêmes valeurs, dans n'importe quel ordre. |
| `a + b -> Set<T>` | un nouveau Set des valeurs des deux. |
| `a += b` | ajoute les valeurs de `b` à `a`. |
| `s.clone -> Set<T>` | une copie. |

```kova
fn main {
    var tags: Set<String> := Set["kova", "docs"]
    tags.add("kova")
    tags.add("web")
    print(tags)
    print(tags.contains("docs"))
    print(tags.remove("docs"))
    print(tags.length)
}
```

```text
Set["kova", "docs", "web"]
true
true
2
```
