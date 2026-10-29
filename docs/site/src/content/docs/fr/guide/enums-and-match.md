---
title: Enums et match
description: Des types dont la valeur est un cas parmi plusieurs, et comment match les traite tous.
sidebar:
  order: 6
---

Une struct contient **tous** ses champs à la fois. Une `enum` contient **un** de ses cas : un feu
est rouge, orange ou vert, jamais deux à la fois.

```kova
enum Light {
    Red
    Orange
    Green

    fn next(self) -> Light {
        return match self {
            Red => Light.Green
            Orange => Light.Red
            Green => Light.Orange
        }
    }
}

fn main {
    let light := Light.Red
    print(light.next)
    print(light.next.next)
}
```

```text
Light.Green
Light.Orange
```

Un cas s'écrit `Light.Red` ; dans un `match` sur un `Light`, simplement `Red`. Une enum peut avoir
des méthodes et des fonctions statiques, comme une struct.

## Des cas qui portent des valeurs

Chaque cas peut avoir ses propres champs. Une forme est un cercle avec un rayon, un rectangle
avec deux côtés, ou un point sans rien :

```kova
enum Shape {
    Circle(radius: Float)
    Rect(width: Float, height: Float)
    Dot
}

fn area(s: Shape) -> Float {
    return match s {
        Circle(r) => 3.14159 * r * r
        Rect(w, h) => w * h
        Dot => 0.0
    }
}

fn main {
    let shapes: List<Shape> := [Shape.Circle(1.0), Shape.Rect(2.0, 3.0), Shape.Dot]
    for s in shapes {
        print(area(s))
    }
}
```

```text
3.14159
6.0
0.0
```

Dans un cas, `Circle(r)` nomme les champs du cas dans leur ordre : `r` est le rayon, et n'existe
que dans ce cas. `_` remplace un champ dont on n'a pas besoin : `Rect(w, _)`.

C'est ainsi que Kova dit « un de ceux-ci, et voici ce que chacun contient » en un seul type.
D'autres langages ont besoin d'une classe par cas et d'un test de type, ou d'une struct dont
certains champs n'ont de sens que pour certaines valeurs ; le compilateur ne peut alors pas savoir
quel champ est valide quand.

## Chaque cas est traité

Le compilateur connaît chaque cas d'une enum, donc il vérifie qu'un `match` les traite tous :

```kova
enum Shape {
    Circle(radius: Float)
    Rect(width: Float, height: Float)
    Dot
}

fn area(s: Shape) -> Float {
    return match s {
        Circle(r) => 3.0 * r * r
        Rect(w, h) => w * h
    }
}

fn main {
    print(area(Shape.Dot))
}
```

```txt
erreur: cas de `Shape` non traités : `Dot`
 --> main.kova:8:18
  |
8 |     return match s {
  |                  ^
  = aide : ajoute-les, ou termine par `_ => …` pour tous les autres
```

Le jour où un cas est ajouté à une enum, chaque `match` qui l'oublie est signalé, au lieu d'un
programme qui ne fait silencieusement rien pour lui. Dans l'éditeur, une correction rapide écrit
les cas manquants.

`_ => …` traite tous les cas non écrits ; utilise-le quand ils sont vraiment tous traités pareil,
car il cache aussi les cas ajoutés plus tard. Plusieurs cas sans champs peuvent partager une
ligne :

```kova
fn main {
    let today := Date.year(2026) month(10) day(24)
    match today.weekday {
        Saturday, Sunday => print("weekend")
        _ => print("work")
    }
}
```

```text
weekend
```

## `match` donne une valeur

Comme `if`, `match` donne la valeur du cas pris, comme dans `area` plus haut. Un cas peut aussi
exécuter tout un bloc :

```kova
enum Command {
    Say(text: String)
    Quit
}

fn main {
    let commands: List<Command> := [Command.Say("hello"), Command.Quit, Command.Say("never")]
    for c in commands {
        match c {
            Say(t) => print(t)
            Quit => {
                print("bye")
                break
            }
        }
    }
}
```

```text
hello
bye
```

## Des enums qui se contiennent

Un cas peut contenir une valeur de sa propre enum. C'est ainsi qu'on écrit des arbres et des
expressions :

```kova
enum Expr {
    Num(value: Int)
    Add(left: Expr, right: Expr)
    Mul(left: Expr, right: Expr)
}

fn eval(e: Expr) -> Int {
    return match e {
        Num(v) => v
        Add(a, b) => eval(a) + eval(b)
        Mul(a, b) => eval(a) * eval(b)
    }
}

fn main {
    let e := Expr.Add(Expr.Num(2), Expr.Mul(Expr.Num(3), Expr.Num(4)))
    print(eval(e))
}
```

```text
14
```

Les enums peuvent aussi être génériques,
`enum Tree<T> { Leaf; Node(left: Tree<T>, value: T, right: Tree<T>) }`
([Génériques](/fr/guide/interfaces-and-generics/#les-types-génériques)).

## Changer les champs d'un cas

Les noms d'un cas sont en lecture seule. Pour changer la valeur sur place, fais le `match` avec
`var` :

```kova
enum Counter {
    Off
    On(count: Int)
}

fn tick(c: var Counter) {
    match var c {
        On(n) => n := n + 1
        Off => {}
    }
}

fn main {
    var c := Counter.On(1)
    tick(var c)
    tick(var c)
    print(c)
}
```

```text
Counter.On(count: 3)
```

Deux enums sont toujours là : `T?` (une valeur ou `none`) et `T or E` (un résultat ou une
erreur), expliquées dans [Valeurs absentes et erreurs](/fr/guide/errors/).
