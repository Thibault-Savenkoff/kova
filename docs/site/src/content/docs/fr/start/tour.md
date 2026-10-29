---
title: Un tour de Kova
description: Tout le langage en dix minutes, un petit programme à la fois.
sidebar:
  order: 3
---

Cette page montre les grandes idées de Kova en quelques programmes. Chaque partie se termine par
un lien vers le chapitre du guide qui l'explique en entier.

## Les valeurs

```kova
fn main {
    let name: String := "Ada"
    var age: Int := 36
    age += 1
    print("{name} is {age}")
    print(age = 37)
}
```

```text
Ada is 37
true
```

`let` ne change jamais, `var` peut changer. `:=` donne une valeur, `=` compare : il n'y a pas de
`==`. Les types sont écrits. [Valeurs et types →](/fr/guide/values/)

## Les fonctions

```kova
fn area(width: Float) by(height: Float) -> Float {
    return width * height
}

fn greeting -> String {
    return "hello"
}

fn main {
    print(area(3.0) by(4.0))
    print(greeting.to_upper)
}
```

```text
12.0
HELLO
```

Un nom peut être coupé en parties, `area(…) by(…)`, pour qu'un appel se lise comme une phrase. Une
fonction sans paramètres n'a pas de `()`. [Fonctions →](/fr/guide/functions/)

## Structs et enums

```kova
struct Dog {
    name: String
    age: Int

    fn birthday(var self) {
        self.age += 1
    }
}

enum Size {
    Small
    Large(kilos: Int)
}

fn describe(s: Size) -> String {
    return match s {
        Small => "small"
        Large(k) => "large, {k} kg"
    }
}

fn main {
    var rex := Dog { name := "Rex", age := 3 }
    rex.birthday
    print(rex)
    print(describe(Size.Large(30)))
}
```

```text
Dog { name: "Rex", age: 4 }
large, 30 kg
```

Un `match` doit traiter chaque cas d'une enum : en oublier un est une erreur.
[Structs →](/fr/guide/structs/) [Enums →](/fr/guide/enums-and-match/)

## La possession

```kova
fn add_one(l: var List<Int>) {
    l.add(1)
}

fn main {
    var l: List<Int> := [5]
    add_one(var l)
    print(l)
    let m: List<Int> := l
    print(m)
}
```

```text
[5, 1]
[5, 1]
```

La mémoire est libérée sans ramasse-miettes : chaque valeur a un seul propriétaire. Un appel qui
change une valeur le dit avec `var` à l'appel ; `let m := l` déplace la liste dans `m`, et `l` ne
peut plus servir. [Possession →](/fr/guide/ownership/)

## Valeurs absentes et erreurs

```kova
fn half(n: Int) -> Int or String {
    if n % 2 != 0 {
        return err("{n} is odd")
    }
    return ok(n / 2)
}

fn main {
    match "42".to_int {
        none => print("not a number")
        n => print(half(n))
    }
    print(half(7))
}
```

```text
ok(21)
err("7 is odd")
```

Un `Int?` peut être `none` ; `T or E` est un résultat ou une erreur. Le compilateur s'assure que
les deux sont regardés. [Valeurs absentes et erreurs →](/fr/guide/errors/)

## Collections et blocs

```kova
fn main {
    let scores: List<Int> := [12, 5, 18, 7]
    print(scores.filter { x: Int => x > 10 })
    print(scores.sorted)
    let ages: Map<String, Int> := ["Ana" := 31, "Bo" := 25]
    print(ages["Bo"])
}
```

```text
[12, 18]
[5, 7, 12, 18]
25
```

[Listes, Maps et Sets →](/fr/guide/collections/) [Blocs →](/fr/guide/blocks/)

## Contrats et tests

```kova
fn divide(a: Int, b: Int) -> Int
    require b != 0 else "you cannot divide by zero"
{
    return a / b
}

test "divide rounds down" {
    check divide(7, 2) = 3
}

fn main {
    print(divide(9, 2))
}
```

```text
4
```

Une fonction dit ce dont elle a besoin ; `kova test` lance les tests.
[Contrats et tests →](/fr/guide/contracts-and-tests/)

## Et la bibliothèque

Fichiers, web et serveur web, JSON et CSV, dates, motifs, chiffrement, fenêtres natives et appels
au C viennent avec Kova : [la bibliothèque](/fr/library/overview/).
