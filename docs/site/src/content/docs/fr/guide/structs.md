---
title: Structs et méthodes
description: Tes propres types, leurs champs, leurs méthodes et leurs fonctions statiques.
sidebar:
  order: 5
---

Une `struct` regroupe des valeurs nommées, ses **champs**, en un type à toi :

```kova
struct Dog {
    name: String
    age: Int
}

fn main {
    var rex := Dog { name := "Rex", age := 3 }
    rex.age += 1
    print(rex.name)
    print(rex)
}
```

```text
Rex
Dog { name := "Rex", age := 4 }
```

- `Dog { name := "Rex", age := 3 }` crée une valeur : chaque champ est donné, dans n'importe quel
  ordre. En oublier un est une erreur, car un champ ne contient jamais « rien » par accident (un
  champ qui peut être vide se déclare `T?`, voir [Valeurs absentes](/fr/guide/errors/)).
- `rex.age` lit un champ ; sur un `var`, `rex.age := 5` le change.
- Les noms de types commencent par une majuscule, les noms de champs par une minuscule.
- Comme le nom du type est écrit à droite, `let rex := Dog { … }` n'a pas besoin de `: Dog`.

### Écrire une valeur plus court

Un champ peut avoir sa propre valeur, utilisée quand le littéral le laisse de côté. Une variable qui
porte le nom du champ ne s'écrit qu'une fois. `...` prend les autres champs d'une autre valeur :

```kova
struct Dog {
    name: String
    age: Int := 1
}

fn main {
    let rex: Dog := Dog { name := "Rex" }
    print(rex)
    let name: String := "Fido"
    let fido: Dog := Dog { name, age := 3 }
    print(fido)
    let older: Dog := Dog { ...fido, age := 4 }
    print(older)
}
```

```text
Dog { name := "Rex", age := 1 }
Dog { name := "Fido", age := 3 }
Dog { name := "Fido", age := 4 }
```

`...fido` copie ses champs, donc `fido` reste utilisable. La valeur propre d'un champ ne lit aucune
variable.

## Les méthodes

Les fonctions écrites dans la struct sont ses **méthodes**. Elles atteignent la valeur par
`self`, et disent ce qu'elles en font, comme tout
[paramètre](/fr/guide/ownership/#règle-3--un-appel-emprunte-sauf-sil-dit-autre-chose) :

| Écrit | La méthode peut | S'appelle sur |
| --- | --- | --- |
| `fn summary(self)` | lire la valeur | n'importe quelle valeur |
| `fn deposit(var self, …)` | changer la valeur | un `var` |
| `fn close(move self)` | prendre la valeur (elle est consommée) | une valeur dont on n'a plus besoin |

Une fonction dans la struct sans `self` est une **fonction statique**, appelée sur le type
lui-même : `Account.open("Ada")`. C'est la façon habituelle de donner à un type un constructeur
dont le nom dit ce qu'il fait.

```kova
struct Account {
    owner: String
    balance: Int

    fn open(owner: move String) -> Account {
        return Account { owner := owner, balance := 0 }
    }

    fn deposit(var self, amount: Int) {
        self.balance += amount
    }

    fn summary(self) -> String {
        return "{self.owner}: {self.balance}"
    }

    fn close(move self) -> String {
        return "closed, {self.balance} given back to {self.owner}"
    }
}

fn main {
    var account: Account := Account.open("Ada")
    account.deposit(50)
    print(account.summary)
    print(account.close)
}
```

```text
Ada: 50
closed, 50 given back to Ada
```

Après `account.close`, `account` n'existe plus : s'en servir encore est une erreur, comme pour
toute valeur déplacée.

`open` prend `owner: move String` parce qu'il garde le texte dans le nouveau compte. Avec un
simple `owner: String`, le texte serait seulement emprunté, et une valeur empruntée ne peut pas
être rangée :

```kova
struct Account {
    owner: String

    fn open(owner: String) -> Account {
        return Account { owner := owner }
    }
}

fn main {
    let a: Account := Account.open("Ada")
}
```

```txt
erreur: `owner` est emprunté ici : il ne peut pas être déplacé
 --> main.kova:5:35
  |
5 |         return Account { owner := owner }
  |                                   ^^^^^
  = aide : donne une copie : `owner.clone`
```

Une méthode sans paramètres s'appelle sans `()`, comme un champ : `account.summary`. Qui utilise
le type n'a pas besoin de savoir lequel des deux c'est.

## Les opérateurs sur tes types

`+`, `-`, `*`, `/` et `%` marchent sur tes propres types quand le type a une méthode de ce nom. La
méthode prend `self` et un paramètre, et `a + b` l'appelle sur `a` : c'est donc la valeur de gauche
qui choisit la méthode. `+=` et les autres viennent avec : `total += a` est `total := total + a`.

```kova
struct Vector {
    x: Float
    y: Float

    fn +(self, other: Vector) -> Vector := Vector { x := self.x + other.x, y := self.y + other.y }
    fn *(self, k: Float) -> Vector := Vector { x := self.x * k, y := self.y * k }
}

fn main {
    let a := Vector { x := 1.0, y := 2.0 }
    let b := Vector { x := 3.0, y := 4.0 }
    print(a + b)
    print((a + b) * 2.0)

    var total := Vector { x := 0.0, y := 0.0 }
    total += a
    total += b
    print(total)
}
```

```text
Vector { x := 4.0, y := 6.0 }
Vector { x := 8.0, y := 12.0 }
Vector { x := 4.0, y := 6.0 }
```

`-v` appelle `fn -(self)`, une méthode sans paramètre qui vit à côté de `fn -(self, other: T)`.

Quand la valeur de gauche est un nombre, la méthode va sur le type du nombre avec
[`extend`](/fr/guide/interfaces-and-generics/#ajouter-des-méthodes-à-nimporte-quel-type). Kova
l'appelle quand le côté droit a le type qu'elle prend ; entre deux nombres, `*` reste le `*` habituel.

```kova
struct Vector {
    x: Float
    y: Float

    fn -(self) -> Vector := Vector { x := -self.x, y := -self.y }
    fn *(self, k: Float) -> Vector := Vector { x := self.x * k, y := self.y * k }
}

extend Float {
    fn *(self, v: Vector) -> Vector := v * self
}

fn main {
    let v := Vector { x := 1.0, y := 2.0 }
    print(-v)
    print(v * 2.0)
    print(2.0 * v)
    print(2.0 * 3.0)
}
```

```text
Vector { x := -1.0, y := -2.0 }
Vector { x := 2.0, y := 4.0 }
Vector { x := 2.0, y := 4.0 }
6.0
```

Le paramètre n'a pas besoin d'être du même type : ci-dessus, `Vector * Float` agrandit un vecteur.
Donc `v * 2.0` marche et `2.0 * v` non, puisque `Float` n'a pas cette méthode. Un opérateur est une
méthode, et Kova n'a pas de surcharge : un type ne peut pas avoir à la fois `Vector * Float` et
`Vector * Vector`. Donne un nom au second, comme `v.dot(w)`.

`=` et `<` ne s'écrivent pas comme ça : ils viennent de [`Equal` et `Comparable`](#comparer-et-afficher).
Une interface peut demander un opérateur, ce qui permet à une fonction générique de l'utiliser :
`interface Addable { fn +(self, other: Self) -> Self }`.

## Comparer et afficher

Une struct ne se compare avec `=` et `<`, ou ne s'affiche dans un texte, que si elle le dit, en
suivant les [interfaces](/fr/guide/interfaces-and-generics/#les-interfaces-intégrées) intégrées :

```kova
struct Point : Equal {
    x: Int
    y: Int
}

fn main {
    let a := Point { x := 1, y := 2 }
    print(a = Point { x := 1, y := 2 })
}
```

```text
true
```

`: Equal` compare les champs un par un ; une struct peut aussi écrire son propre `equals`. Sans
lui, `a = b` est refusé, et l'aide propose d'ajouter `: Equal`. Kova ne décide pas à ta place ce
qui rend deux valeurs de ton type identiques.

## Les valeurs copiées

Une struct se déplace comme un `String`. Quand elle ne contient que des nombres, des `Bool`, des
`Char` et d'autres types copiés, elle peut être déclarée `copy struct` : elle est alors copiée
partout où on la donne, comme un `Int`.

```kova
copy struct Size {
    width: Int
    height: Int
}

fn main {
    let a := Size { width := 3, height := 4 }
    let b: Size := a
    print(a.width * b.height)
}
```

```text
12
```

## Les champs vus des autres fichiers

Dans un fichier, chaque champ peut être lu. Dans un fichier que d'autres
[importent](/fr/guide/modules/), seul ce qui est écrit sous `public:` est visible de l'extérieur,
champs compris : un type peut ainsi garder son intérieur pour lui et offrir des méthodes à la
place.
