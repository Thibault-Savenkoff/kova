---
title: Interfaces et génériques
description: Ce que plusieurs types ont en commun, du code écrit une fois pour plusieurs types, et des valeurs de n'importe quel type qui suit une interface.
sidebar:
  order: 10
---

## Les interfaces

Une **interface** liste des méthodes que plusieurs types promettent d'avoir. Un type la suit en la
nommant après `:`, et le compilateur vérifie la promesse :

```kova
interface Animal {
    fn sound(self) -> String

    fn twice(self) -> String {
        return "{self.sound} {self.sound}"
    }
}

struct Dog : Animal {
    fn sound(self) -> String {
        return "woof"
    }
}

struct Cat : Animal {
    fn sound(self) -> String {
        return "meow"
    }

    fn twice(self) -> String {
        return "purr"
    }
}

fn main {
    print(Dog {}.twice)
    print(Cat {}.twice)
}
```

```text
woof woof
purr
```

- Une méthode sans corps (`sound`) doit être écrite par chaque type qui suit l'interface.
- Une méthode avec un corps (`twice`) est une méthode **par défaut** : un type l'a gratuitement,
  ou écrit la sienne.
- Un type peut suivre plusieurs interfaces : `struct Version : Comparable, Display`.

Oublier une méthode se voit tout de suite, avec la méthode à écrire :

```kova
interface Animal {
    fn sound(self) -> String
}

struct Dog : Animal {
    name: String
}

fn main {
}
```

```txt
erreur: `Dog` suit `Animal` mais n'a pas de méthode `sound`
 --> main.kova:5:14
  |
5 | struct Dog : Animal {
  |              ^^^^^^
  = aide : ajoute `fn sound(self) -> String { … }` (`Self` = ce type)
```

Dans une interface, `Self` est « le type qui la suit » : `fn equals(self, other: Self) -> Bool`.

Une interface peut aussi porter des [contrats](/fr/guide/contracts-and-tests/) : chaque type qui la
suit y est alors tenu.

## Les interfaces intégrées

Trois interfaces font partie du langage, et donnent à un type ses opérateurs :

| Interface | Méthode | Donne au type |
| --- | --- | --- |
| `Equal` | `fn equals(self, other: Self) -> Bool` | `=` et `!=` ; d'être une clé de `Map` ou dans un `Set` |
| `Comparable` | `fn less(self, other: Self) -> Bool` | `<`, `<=`, `>`, `>=` ; `l.sorted` |
| `Display` | `fn to_string(self) -> String` | son texte dans `"{…}"` et dans `print` |

Pour `Equal`, la méthode peut être omise : les champs sont alors comparés un par un.

```kova
struct Version : Comparable, Display {
    major: Int
    minor: Int

    fn less(self, other: Version) -> Bool {
        return self.major < other.major || (self.major = other.major && self.minor < other.minor)
    }

    fn to_string(self) -> String {
        return "v{self.major}.{self.minor}"
    }
}

fn main {
    let versions: List<Version> := [Version { major := 2, minor := 0 }, Version { major := 1, minor := 9 }, Version { major := 1, minor := 10 }]
    print(versions.sorted)
    print("latest: {versions.sorted[2]}")
}
```

```text
[v1.9, v1.10, v2.0]
latest: v2.0
```

Rien n'est donné sans être demandé : une struct n'est pas comparable, ni affichable dans un texte,
tant qu'elle ne suit pas l'interface. Ainsi, `a < b` veut toujours dire un ordre que quelqu'un a
choisi.

## Les fonctions génériques

Une fonction **générique** s'écrit une fois pour plusieurs types. Ses paramètres de type viennent
entre `<…>` après son nom, avec ce qu'ils doivent permettre :

```kova
fn larger<T: Comparable>(a: T, b: T) -> T {
    if a < b {
        return b.clone
    }
    return a.clone
}

fn main {
    print(larger(3, 8))
    print(larger("pear", "apple"))
    print(larger(2.5, 1.0))
}
```

```text
8
pear
2.5
```

Les types sont trouvés à partir des arguments. `T: Comparable` est obligatoire parce que la
fonction utilise `<` ; sans lui, le compilateur refuse `a < b`, puisque certains `T` ne peuvent pas
être ordonnés :

```kova
fn larger<T>(a: T, b: T) -> T {
    if a < b {
        return b.clone
    }
    return a.clone
}

fn main {
}
```

```txt
erreur: `<` ne sait pas ordonner des valeurs `T`
 --> main.kova:2:8
  |
2 |     if a < b {
  |        ^^^^^
  = aide : `T` doit suivre `Comparable`
```

La fonction est vérifiée une fois, pour tous les `T` à la fois, et compilée une fois par type
avec lequel elle sert : le code générique est aussi rapide que du code écrit pour un seul type.

## Les types génériques

Les structs et les enums prennent des paramètres de type de la même façon :

```kova
struct Stack<T> {
    items: List<T>

    fn push(var self, x: move T) {
        self.items.add(move x)
    }

    fn pop(var self) -> T? {
        if self.items.length = 0 {
            return none
        }
        return self.items.remove(self.items.length - 1)
    }
}

fn main {
    var s := Stack<String> { items := [] }
    s.push("a")
    s.push("b")
    print(s.pop)
    print(s.pop)
    print(s.pop)
}
```

```text
"b"
"a"
none
```

`Stack<String>` est une pile de textes ; `List`, `Map` et `Set` sont des types génériques du même
genre. Les fonctions statiques d'un type générique s'appellent sur le type complet :
`Stack<Int>.empty`, `Tree<Int>.Leaf`.

## Ajouter des méthodes à n'importe quel type

`extend` ajoute des méthodes à un type que tu n'as pas écrit, qu'il vienne du langage, d'une
bibliothèque ou d'un autre fichier de ton programme. À l'intérieur, `self` est la valeur, comme dans
une struct. Ajouter `: Describe` fait suivre une interface au type : il peut alors être donné à une
fonction générique ou tenu dans un `any Describe` :

```kova
interface Describe {
    fn describe(self) -> String
}

extend String {
    fn shout(self) -> String := self.to_upper + "!"
}

extend Int : Describe {
    fn is_even(self) -> Bool := self % 2 = 0
    fn describe(self) -> String := "the number {self}"
}

fn say<T: Describe>(x: T) {
    print(x.describe)
}

fn main {
    print("hello".shout)
    print(4.is_even)
    say(7)
}
```

```text
HELLO!
true
the number 7
```

`extend` marche sur `String`, `Int`, `Float`, `List<Int>`, `Map<String, Int>`, sur tes structs et sur
celles d'un autre fichier. Une struct générique s'étend en entier, `extend Stack`, et ses méthodes
utilisent son `T`.

Un type de la bibliothèque s'étend pour tous ses éléments. Dans `extend List<T>`, une majuscule seule
ou un nom avec une contrainte est un paramètre de type, et tout autre nom est un type :
`extend List<Dog>` vise les Lists de `Dog`, `extend Map<String, V>` les Maps à clés textes.

```kova
extend List<T> {
    fn second(self) -> T? := self.get(1)
}

extend List<T: Comparable> {
    fn smallest(self) -> T? := self.sorted.get(0)
}

fn main {
    print([3, 1, 2].second)
    print(["b", "a"].smallest)
}
```

```text
1
"a"
```

Il ne fait qu'ajouter. Une méthode qui existe déjà ne peut pas être remplacée, sinon le même
`s.length` voudrait dire autre chose d'un programme à l'autre :

```kova
extend String {
    fn length(self) -> Int := 0
}

fn main {
    print("a".length)
}
```

```txt
erreur: `length` existe déjà sur `String` : `extend` ajoute des méthodes, il ne peut pas en changer une
 --> main.kova:2:8
  |
2 |     fn length(self) -> Int := 0
  |        ^^^^^^
```

## `any` : des valeurs de types différents ensemble

Une `List<Animal>` ne peut pas exister : `Animal` n'est pas un type, seulement une promesse. Pour
mettre un chien et un chat dans la même liste, écris `any Animal`, une valeur de **n'importe quel**
type qui suit `Animal` :

```kova
interface Animal {
    fn sound(self) -> String
}

struct Dog : Animal {
    name: String

    fn sound(self) -> String {
        return "woof"
    }

    fn fetch(self) -> String {
        return "{self.name} fetches"
    }
}

struct Cat : Animal {
    fn sound(self) -> String {
        return "meow"
    }
}

fn main {
    let zoo: List<any Animal> := [Dog { name := "Rex" }, Cat {}]
    for a in zoo {
        print(a.sound)
        if a is Dog {
            print(a.fetch)
        }
    }
}
```

```text
woof
Rex fetches
meow
```

- Sur un `any Animal`, seules les méthodes d'`Animal` peuvent être appelées ; chaque valeur
  exécute la sienne.
- `a is Dog` dit s'il contient un `Dog` ; dans `if a is Dog { … }`, `a` **est** ce `Dog`, et ses
  propres méthodes et champs peuvent servir.

Utilise les génériques quand un appel travaille sur un seul type à la fois (`larger(3, 8)`), et
`any` quand des valeurs de types différents doivent vivre ensemble (une liste d'animaux). Les
génériques ne coûtent rien à l'exécution ; `any` cherche la bonne méthode à chaque appel.
