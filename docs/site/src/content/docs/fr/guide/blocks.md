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
    for i in 1..=n {
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

## Donner une fonction à la place d'un bloc

Là où un bloc est attendu, le nom d'une fonction fait le même travail, si ses paramètres sont ceux du
bloc. Un calcul garde ainsi un nom à lui, utilisable partout :

```kova
fn double(x: Int) -> Int := x * 2

fn main {
    let l: List<Int> := [1, 2, 3]
    print(l.map(double))
    print(l.map { x: Int => double(x) })
}
```

```text
[2, 4, 6]
[2, 4, 6]
```

Les deux lignes sont le même programme. Une fonction donnée par son nom ne va qu'à un appel, comme un
bloc : elle n'entre jamais dans une variable.

## Ce qu'un bloc ne peut pas faire

Un bloc s'exécute à l'intérieur d'une autre fonction, à un moment choisi par elle. Donc :

- `return`, `break`, `continue` et `?` ne peuvent pas s'écrire dans un bloc : ils quitteraient une
  fonction ou une boucle qui ne tourne peut-être plus. La valeur du bloc est sa dernière ligne.

## Garder une fonction

Un bloc donné à une fonction est prêté pour cet appel, et il peut changer les variables autour de
lui. Pour garder du code pour plus tard, mets-le dans un endroit qui a un type de bloc : une
variable, un champ, une liste, le résultat d'une fonction. Son type s'écrit comme celui d'un
paramètre bloc, et le bloc s'écrit de la même façon. Un nom de fonction marche aussi :

```kova
fn double(x: Int) -> Int := x * 2

fn main {
    let twice: {Int -> Int} := double
    let square: {Int -> Int} := { x: Int => x * x }
    print(twice(4))
    print(square(4))
}
```

```text
8
16
```

Une fonction gardée prend sa propre copie de ce qu'elle utilise autour d'elle. C'est pour ça qu'elle
peut vivre plus longtemps que la fonction qui l'a créée. `adder` est terminée quand `add3` tourne,
et `n` n'existe plus, mais `add3` tient son propre `n` :

```kova
fn adder(n: Int) -> {Int -> Int} {
    return { x: Int => x + n }
}

fn main {
    let add3: {Int -> Int} := adder(3)
    print(add3(4))
    print(adder(10)(4))
}
```

```text
7
14
```

Comme elle tient une copie, elle voit la valeur du moment où elle a été créée, et les changements
suivants ne l'atteignent pas. Elle ne peut pas non plus changer cette copie, sinon appeler `f(1)`
deux fois pourrait donner deux réponses différentes sans que rien ne se voie à l'appel :

```kova
fn main {
    var count: Int := 0
    let next: {-> Int} := {
        count += 1
        count
    }
}
```

```txt
erreur: une fonction gardée dans une variable ne peut pas changer `count` : elle en tient sa propre copie
 --> main.kova:4:9
  |
4 |         count += 1
  |         ^^^^^
  = aide : utilisez une `struct` avec une méthode `var self` pour garder un état qui change ; entre threads, un `threads.Shared`
```

Une fonction gardée est une valeur comme une `String` : `let b: {Int -> Int} := a` la déplace,
`a.clone` la copie, et on ne peut pas en comparer deux avec `=`.

### Des fonctions dans les champs et les listes

Un champ qui tient une fonction s'appelle comme une méthode. C'est comme ça qu'un bouton se souvient
de ce qu'il fait, et qu'une liste d'étapes s'exécute l'une après l'autre :

```kova
struct Button {
    label: String
    on_click: {String -> String}
}

fn main {
    let button: Button := Button { label := "OK", on_click := { who: String => "{who} clicked" } }
    print(button.on_click("Ada"))

    let steps: List<{Int -> Int}> := [{ x: Int => x + 1 }, { x: Int => x * 10 }]
    print(steps[1](5))
    var value: Int := 1
    for step in steps {
        value := step(value)
    }
    print(value)
}
```

```text
Ada clicked
50
20
```

Une fonction sans paramètres s'appelle en écrivant son nom seul, comme partout en Kova. Là où une
fonction est attendue, ce même nom donne la fonction elle-même au lieu de l'appeler.

### Garder une fonction reçue

Un paramètre bloc est seulement prêté : il peut utiliser des variables de l'appelant, donc il ne
peut pas être gardé.

```kova
fn keep(f: {Int -> Int}) -> {Int -> Int} {
    return f
}

fn main {
    print(keep({ x: Int => x })(1))
}
```

```txt
erreur: `f` est un bloc prêté pour cet appel : il ne peut pas être gardé
 --> main.kova:2:12
  |
2 |     return f
  |            ^
  = aide : pour le garder, écrivez `f: move {…}` dans les paramètres
```

Avec `move`, la fonction appartient à celui qui la reçoit. Combiner deux fonctions en une nouvelle
marche comme ça, et les génériques s'appliquent comme d'habitude :

```kova
fn then<A, B, C>(f: move {A -> B}, g: move {B -> C}) -> {A -> C} {
    return { x: A => g(f(x)) }
}

fn main {
    let size: {String -> String} := then({ s: String => s.length }, { n: Int => "{n} letters" })
    print(size("Kova"))
}
```

```text
4 letters
```

Une fonction gardée peut aussi aller partout où un bloc est attendu, sans `move` : `[1, 2].map(add3)`
la prête pour l'appel.

## Des blocs sur plusieurs fils

`l.map workers(4) { … }` et `l.each workers(4) { … }` répartissent les éléments entre 4 fils
d'exécution : [Plusieurs choses à la fois](/fr/guide/concurrency/).
