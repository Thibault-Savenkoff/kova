---
title: Fonctions
description: Déclarer et appeler des fonctions, les appels en parties, et les valeurs par défaut.
sidebar:
  order: 2
---

## Déclarer une fonction

```kova
fn square(x: Int) -> Int {
    return x * x
}

fn greet(name: String) {
    print("Hello, {name}!")
}

fn main {
    print(square(12))
    greet("Ada")
}
```

```text
144
Hello, Ada!
```

`fn`, le nom, les paramètres avec leurs types, puis `->` et le type du résultat, s'il y en a un.
Les fonctions se déclarent dans n'importe quel ordre : `main` peut appeler une fonction écrite
plus bas.

Une fonction qui donne un résultat doit le rendre avec `return` dans tous les cas, et le
compilateur le vérifie :

```kova
fn sign(n: Int) -> Int {
    if n > 0 {
        return 1
    }
}

fn main {
    print(sign(2))
}
```

```txt
erreur: `sign` peut arriver ici sans `return`
 --> main.kova:5:1
  |
5 | }
  | ^
  = aide : la fonction doit renvoyer un `Int` dans tous les cas
```

Les paramètres ne peuvent pas changer dans la fonction : ils sont ce que l'appelant a donné. Pour
changer une valeur de l'appelant, le paramètre dit `var`, et l'appel aussi (voir
[Possession](/fr/guide/ownership/)).

## Pas de `()` sans arguments

Une fonction sans paramètres se déclare et s'appelle sans parenthèses :

```kova
fn seven -> Int {
    return 7
}

fn main {
    print(seven)
    print(seven.to_float)
}
```

```text
7
7.0
```

Pareil pour les méthodes : `name.length`, `rex.sound`, `x.to_float`. L'idée vient d'Eiffel et de
Smalltalk : que `seven` soit une valeur rangée ou calculée est un détail de fabrication, et le
code qui l'utilise ne devrait pas changer quand ce détail change. `seven()` est une erreur, et
l'aide dit d'écrire `seven`. La seule exception est un type générique créé vide, qui garde ses
`()` : `List<Int>()`.

Pour qu'un nom seul se lise toujours de la même façon, aucune variable ne peut prendre le nom
d'une fonction sans paramètres.

## Les appels en parties

Quand une fonction prend plusieurs choses, son nom peut être coupé en **parties**, chacune
nommant ce qui la suit, pour que l'appel se lise comme une phrase :

```kova
fn between(x: Int, low: Int) upto(high: Int) -> Bool {
    return low <= x && x <= high
}

fn main {
    print(between(5, 1) upto(10))
    print(between(12, 1) upto(10))
}
```

```text
true
false
```

Le nom complet de la fonction est `between(…) upto(…)` ; chaque partie a ses paramètres. La
bibliothèque s'en sert partout où une simple liste d'arguments serait ambiguë :
`files.copy(a) to(b)`, `s.replace_all(pattern) with(text)`, `secure.encrypt(text) key(k)`. C'est
encore l'idée de Smalltalk (`dict at: 1 put: x`) : à l'appel, on voit quel argument est lequel,
sans aller lire la déclaration.

## Les valeurs par défaut

Un paramètre d'une partie peut avoir une valeur par défaut. Une partie dont tous les paramètres
en ont une peut alors être omise à l'appel :

```kova
fn connect(host: String) port(p: Int := 80) secure(on: Bool := false) -> String {
    return "{host}:{p} {on}"
}

fn main {
    print(connect("example.com"))
    print(connect("example.com") port(8080))
    print(connect("example.com") secure(true))
}
```

```text
example.com:80 false
example.com:8080 false
example.com:80 true
```

On n'omet que des parties entières, et la première partie est toujours écrite : un appel ne
dépend jamais de la position d'un argument absent.

## Plusieurs résultats

Une fonction rend une seule valeur, mais cette valeur peut être un
[tuple](/fr/guide/values/#les-tuples) ou une [struct](/fr/guide/structs/) :

```kova
fn divide(a: Int, b: Int) -> (Int, Int) {
    return (a / b, a % b)
}

fn main {
    let (q, r): (Int, Int) := divide(17, 5)
    print("17 = 5 × {q} + {r}")
}
```

```text
17 = 5 × 3 + 2
```

Une fonction qui peut échouer rend un `T or E`, et une qui peut n'avoir rien à donner rend un
`T?` : [Valeurs absentes et erreurs](/fr/guide/errors/) explique les deux.

## La récursion

Une fonction peut s'appeler elle-même :

```kova
fn fib(n: Int) -> Int {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}

fn main {
    print(fib(30))
}
```

```text
832040
```

## Ce qu'une fonction n'est pas

Les fonctions ne sont pas des valeurs en Kova : on ne les range pas dans des variables ni dans des
listes. Pour donner du code à une fonction, on écrit un [bloc](/fr/guide/blocks/), fait pour ça.
Chaque appel reste ainsi lisible : un nom suivi d'arguments est toujours cette fonction-là, jamais
une variable qui pourrait en contenir une autre.
