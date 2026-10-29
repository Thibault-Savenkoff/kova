---
title: Contrats et tests
description: require, ensure et check disent ce qui doit être vrai ; les blocs test le prouvent.
sidebar:
  order: 12
---

Le type d'un paramètre dit quel genre de valeur une fonction prend, mais rarement tout ce dont
elle a besoin : un diviseur ne doit pas être zéro, une liste ne doit pas être vide. Kova permet à
une fonction de l'écrire, dans sa signature, là où chaque lecteur le voit. C'est la
**programmation par contrat**, telle qu'Eiffel l'a introduite.

## `require` : ce dont une fonction a besoin

```kova
fn divide(a: Int, b: Int) -> Int
    require b != 0 else "you cannot divide by zero"
{
    return a / b
}

fn main {
    print(divide(10, 2))
    print(divide(5, 0))
}
```

```txt
erreur: you cannot divide by zero
  --> main.kova:2:13
  require b != 0
  valeurs : b = 0
  appelé depuis main.kova:9:11
```

`require` vient après la signature, avant le corps. Quand il est faux, le programme s'arrête, et le
message montre la condition, les valeurs qu'elle a lues, et **qui a appelé** la fonction : un
`require` non respecté est l'erreur de l'appelant, c'est donc là qu'il faut regarder. Le texte après
`else` est facultatif ; il peut contenir des valeurs, `"{self.name}: negative years"`.

## `ensure` : ce qu'une fonction promet

`ensure` vérifie le résultat quand la fonction rend la main. `result` est la valeur qu'elle donne :

```kova
fn average(l: List<Int>) -> Int
    require l.length > 0
    ensure result >= 0
{
    var total: Int := 0
    for x in l {
        total += x
    }
    return total / l.length
}

fn main {
    print(average([4, -9]))
}
```

```txt
erreur: contrat non respecté
  --> main.kova:3:12
  ensure result >= 0
  valeurs : result = -2
  appelé depuis main.kova:13:11
```

Un `ensure` non respecté est l'erreur de la fonction elle-même : ici, elle a promis plus qu'elle
ne fait. Soit la promesse est fausse (`ensure` doit partir, ou `require` doit demander des nombres
positifs), soit le code l'est.

## `check` : ce qui doit être vrai ici

`check` affirme une chose qui doit être vraie à cet endroit du code, n'importe où :

```kova
fn main {
    let parts: List<String> := "2026-10-29".split("-")
    check parts.length = 3 else "a date has three parts"
    print(parts[0])
}
```

```text
2026
```

## Pourquoi écrire des contrats

- **Ils documentent.** `require b != 0` en dit plus, et plus exactement, qu'un commentaire, et ne
  peut pas devenir faux sans qu'on le voie : il est vérifié.
- **Ils attrapent les erreurs là où elles sont faites.** Sans contrats, une mauvaise valeur
  voyage dans le programme et casse quelque chose loin de là. Avec eux, elle s'arrête à la première
  fonction qui l'a reçue, avec les valeurs qui ont enfreint la règle.
- **Ils sont gratuits quand on le veut.** `kova build --boost` retire tous les contrats de
  l'exécutable, pour le dernier gain de vitesse, une fois les tests passés. Par défaut ils restent,
  car un programme qui s'arrête avec un message clair vaut mieux qu'un programme qui continue avec
  une valeur fausse.

Dans une [interface](/fr/guide/interfaces-and-generics/), les contrats tiennent chaque type qui la
suit. Un type peut promettre plus (un `ensure` à lui), mais jamais demander plus (pas de `require`
à lui quand l'interface en a un) : le code écrit pour l'interface doit continuer de marcher avec
chaque type.

## Les tests

Un bloc `test`, à côté du code qu'il vérifie, contient des `check` :

```kova
fn square(x: Int) -> Int {
    return x * x
}

test "square of 3" {
    check square(3) = 9
}

test "square of a negative" {
    let n: Int := -4
    check square(n) = -16 else "a square is never negative"
}

fn main {
    print(square(5))
}
```

`kova test squares.kova` lance chaque test du fichier :

```console
✅ square of 3
❌ square of a negative
   erreur: a square is never negative
     --> squares.kova:11:11
     check square(n) = -16
     valeurs : n = -4

1 réussi(s), 1 raté(s) sur 2
```

- Chaque test tourne seul : un test qui échoue ou s'arrête n'arrête pas les autres.
- `kova test squares.kova "square of 3"` ne lance que celui-là.
- `kova run` et `kova build` laissent les tests de côté : ils ne coûtent rien au programme.
- Dans l'éditeur, un ▶ dans la marge lance un test, et ✅ ou ❌ montre son résultat.

`kova test` se termine avec le code de sortie 1 quand un test échoue, pour pouvoir arrêter une
compilation ou une CI.
