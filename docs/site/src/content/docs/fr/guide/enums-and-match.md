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
            .Red => Light.Green
            .Orange => Light.Red
            .Green => Light.Orange
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

Un cas s'écrit `Light.Red` ; là où le type est déjà connu, comme dans un `match` sur un `Light`,
simplement `.Red`. Une enum peut avoir
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
        .Circle(r) => 3.14159 * r * r
        .Rect(w, h) => w * h
        .Dot => 0.0
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

## Écrire moins quand le type est connu

Quand l'enum est déjà connue d'après l'endroit où va la valeur, le cas commence par un point :

```kova
enum Light {
    Red
    Green
}

fn name(l: Light) -> String {
    return match l {
        .Red => "stop"
        .Green => "go"
    }
}

fn main {
    let l: Light := .Green
    print(name(l))
    print(name(.Red))
    print(l = .Green)
}
```

```text
go
stop
true
```

`.Green` marche partout où une `Light` est attendue : une variable avec son type, un argument, un
`return`, un champ, une liste de `Light`, l'autre côté de `=`. Là où rien ne dit de quelle enum il
s'agit, comme `print(.Red)`, Kova demande le nom complet.

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
        .Circle(r) => 3.0 * r * r
        .Rect(w, h) => w * h
    }
}

fn main {
    print(area(Shape.Dot))
}
```

```txt
erreur: cas de `Shape` non traités : `.Dot`
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
        .Saturday, .Sunday => print("weekend")
        _ => print("work")
    }
}
```

```text
weekend
```

Des cas avec champs peuvent aussi partager une ligne, quand chacun nomme les mêmes noms, du même
type : le cas trouvé leur donne ses champs.

```kova
enum Shape {
    Circle(radius: Float)
    Square(side: Float)
    Rect(width: Float, height: Float)
}

fn widest(s: Shape) -> Float {
    return match s {
        .Circle(x), .Square(x) => x
        .Rect(w, h) => if w > h { w } else { h }
    }
}

fn main {
    print(widest(Shape.Square(3.0)))
    print(widest(Shape.Rect(1.0, 4.0)))
}
```

```text
3.0
4.0
```

`.Circle(x), .Rect(x, _)` marche aussi : ce qui compte, ce sont les noms, pas la place des champs.
Un nom absent d'un cas (`.Circle(x), .Point`) est refusé, puisque `x` n'y aurait pas de valeur.

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
            .Say(t) => print(t)
            .Quit => {
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

## Des cas dans les cas

Un cas peut contenir un autre cas là où va une valeur : `err(.Lost)`, c'est « une erreur, et cette
erreur est `Lost` ». Ça évite un `match` dans un `match` :

```kova
enum Problem {
    Lost
    Late(minutes: Int)
}

fn fetch(k: Int) -> Int or Problem {
    if k = 0 {
        return err(.Lost)
    }
    if k = 1 {
        return err(.Late(5))
    }
    return ok(k * 10)
}

fn main {
    for k in 0..4 {
        match fetch(k) {
            ok(20) => print("vingt")
            ok(v) => print("reçu {v}")
            err(.Lost) => print("perdu")
            err(.Late(m)) => print("en retard de {m}")
        }
    }
}
```

```text
perdu
en retard de 5
vingt
reçu 30
```

Les cas d'un même variant sont vérifiés ensemble : chaque cas à l'intérieur doit donc être traité
lui aussi. Un `err(e) =>` plus bas prend les autres et nomme l'erreur entière ; un `_ =>` final les
prend aussi :

```kova
enum Problem {
    Lost
    Late(minutes: Int)
}

fn main {
    let r: Int or Problem := err(.Late(3))
    match r {
        ok(v) => print(v)
        err(.Lost) => print("perdu")
    }
}
```

```txt
erreur: cas de `Problem` non traités : `.Late`
  --> main.kova:10:9
   |
10 |         err(.Lost) => print("perdu")
   |         ^^^
   = aide : ajoute-les, ou termine par `_ => …` pour tous les autres
```

Un variant à plusieurs champs accepte des valeurs et des variants sans champs parmi ses noms, un
tuple aussi :

```kova
enum Light {
    Red
    Green
}

enum Event {
    Pair(light: Light, n: Int)
    Quit
}

fn main {
    let events: List<Event> := [.Pair(.Red, 1), .Pair(.Red, 3), .Pair(.Green, 2), .Quit]
    for e in events {
        match e {
            .Pair(.Red, 1) => print("rouge un")
            .Pair(.Red, n) => print("rouge {n}")
            .Pair(_, n) => print("vert {n}")
            .Quit => print("fin")
        }
    }
    match (Light.Green, 2) {
        (.Red, _) => print("rouge")
        (.Green, k) => print("vert {k}")
        _ => print("jamais")
    }
}
```

```text
rouge un
rouge 3
vert 2
fin
vert 2
```

Kova lit un niveau : `err(.Lost)` et `.Plain(.Circle(r))` marchent, `ok(.Pair(.Red, n))` non. Avec
plusieurs champs, un cas à l'intérieur est une valeur ou un variant sans champs
(`.Pair(.Late(m), 1)` est refusé : nomme le champ et fais un `match` dessus).

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
        .Num(v) => v
        .Add(a, b) => eval(a) + eval(b)
        .Mul(a, b) => eval(a) * eval(b)
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
        .On(n) => n := n + 1
        .Off => {}
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
Counter.On(3)
```

Deux enums sont toujours là : `T?` (une valeur ou `none`) et `T or E` (un résultat ou une
erreur), expliquées dans [Valeurs absentes et erreurs](/fr/guide/errors/).
