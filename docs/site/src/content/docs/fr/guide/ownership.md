---
title: Possession
description: Comment Kova libère la mémoire sans ramasse-miettes, et ce que veulent dire emprunter, var et move.
sidebar:
  order: 7
---

Un programme garde en mémoire des textes, des listes et des structs, et cette mémoire doit être
rendue une fois qu'ils ne servent plus. Les langages le font de trois façons :

- **à la main** (C) : le programmeur libère chaque chose. En oublier une, et la mémoire fuit ; en
  libérer une deux fois, ou s'en servir après, et le programme casse d'une façon difficile à
  trouver ;
- **avec un ramasse-miettes** (Java, Python, Go) : le langage cherche de temps en temps ce qui
  n'est plus atteignable. C'est sûr, mais le programme fait des pauses, utilise plus de mémoire,
  et ne sait jamais exactement quand une chose s'en va ;
- **avec la possession** (Rust, Swift, et Kova) : le compilateur sait, d'après le code lui-même,
  le moment où chaque valeur cesse de servir, et la libère à cet endroit.

Kova prend la troisième voie, sous sa forme la plus simple. Cette page explique les quatre règles
qu'il suit ; une fois lues, les messages du compilateur disent le reste.

## Règle 1 : chaque valeur a un seul propriétaire

Le propriétaire d'une valeur est la variable qui la contient. Quand la variable disparaît (à la
fin de son bloc, de sa fonction, ou d'un tour de boucle), la valeur est libérée avec elle :

```kova
fn main {
    let greeting: String := "hello"
    if greeting.length > 3 {
        let shout: String := greeting.to_upper
        print(shout)
    }
    print(greeting)
}
```

```text
HELLO
hello
```

`shout` est libéré au `}` du `if`, `greeting` à la fin de `main`. Rien n'est écrit pour ça, et
rien n'attend un ramasse-miettes : l'endroit est dans le code compilé.

## Règle 2 : donner une valeur la déplace

Puisqu'il n'y a qu'un propriétaire, `let b := a` ne peut pas laisser deux noms sur le même texte.
La valeur va à `b`, et `a` est vide :

```kova
fn main {
    let a: String := "hello"
    let b: String := a
    print(a)
}
```

```txt
erreur: `a` a été déplacé (ligne 3)
 --> main.kova:4:11
  |
4 |     print(a)
  |           ^
  = aide : utilise `a.clone` à la ligne 3 si tu veux en garder une copie
```

C'est trouvé à la vérification du programme, avant qu'il tourne. Trois sorties, selon ce qui était
voulu :

- utiliser `b` à partir de là : le texte a simplement un nouveau nom ;
- garder les deux avec une copie, `let b: String := a.clone` ;
- ne pas le prendre du tout : la plupart du temps, une valeur a seulement besoin d'être **lue**,
  et lire ne déplace jamais (règle 3).

Une valeur se déplace quand elle est donnée à `let` ou `:=`, rendue avec `return`, mise dans une
struct, une liste ou un tuple, ou passée à un paramètre `move`. Tout le reste ne fait que la lire.

Les nombres, `Bool`, `Char` et les dates sont petits : ils sont copiés au lieu d'être déplacés, et
`let b: Int := a` laisse `a` tel quel. Une struct à toi peut être copiée de la même façon en la
déclarant `copy struct`, quand tous ses champs sont copiés aussi :

```kova
copy struct Point {
    x: Int
    y: Int
}

fn main {
    let a := Point { x := 1, y := 2 }
    let b: Point := a
    print(a)
    print(b)
}
```

```text
Point { x := 1, y := 2 }
Point { x := 1, y := 2 }
```

## Règle 3 : un appel emprunte, sauf s'il dit autre chose

Donner une valeur à une fonction ne la déplace pas. La fonction l'**emprunte** : elle peut la lire
pendant l'appel, et la valeur revient à son propriétaire après. Quand une fonction doit faire
plus, son paramètre le dit, et **l'appel le dit aussi**, pour qu'il suffise de lire une ligne pour
savoir ce qui arrive à chaque valeur :

| Paramètre | À l'appel | La fonction peut | Après |
| --- | --- | --- | --- |
| `s: String` | `shout(word)` | lire `s` | `word` est inchangé |
| `l: var List<Int>` | `add_one(var l)` | lire et changer `l` | `l` contient les changements |
| `l: move List<Int>` | `count(move l)` | garder `l`, le libérer | `l` ne peut plus servir |

```kova
fn shout(s: String) -> String {
    return s.to_upper
}

fn add_one(l: var List<Int>) {
    l.add(1)
}

fn count(l: move List<Int>) -> Int {
    return l.length
}

fn main {
    let word: String := "hi"
    print(shout(word))
    print(word)
    var l: List<Int> := [5]
    add_one(var l)
    print(l)
    print(count(move l))
}
```

```text
HI
hi
[5, 1]
2
```

Le marqueur est obligatoire même là où il pourrait être deviné. Une fonction qui change son
argument sans que l'appel le dise, c'est là que se cachent les bugs ; en Kova, ça ne peut pas
arriver :

```kova
fn add_one(l: var List<Int>) {
    l.add(1)
}

fn main {
    var l: List<Int> := [5]
    add_one(l)
}
```

```txt
erreur: `add_one` peut modifier cet argument : écris `var l`
 --> main.kova:7:13
  |
7 |     add_one(l)
  |             ^
```

Seule une variable `var` peut être donnée en `var`, et un `let` ne change jamais :

```kova
fn main {
    let l: List<Int> := [5]
    l.add(2)
}
```

```txt
erreur: `l` est déclarée avec let
 --> main.kova:3:5
  |
3 |     l.add(2)
  |     ^
  = aide : déclare-la avec `var l` si elle doit changer
```

Les méthodes suivent la même règle pour la valeur avant le point. Une méthode qui la change
déclare `var self`, et `rex.birthday` demande alors que `rex` soit un `var` :

```kova
struct Dog {
    name: String
    age: Int

    fn birthday(var self) {
        self.age += 1
    }
}

fn main {
    var rex := Dog { name := "Rex", age := 3 }
    rex.birthday
    print(rex.age)
}
```

```text
4
```

## Règle 4 : un emprunt ne quitte jamais son bloc

Une valeur empruntée peut être lue, passée à un autre appel, copiée avec `.clone`, mais jamais
gardée : elle ne peut pas être rendue, rangée dans une struct ni ajoutée à une liste. C'est ce qui
permet à Kova d'éviter la partie la plus difficile des autres langages à possession : il n'y a
jamais de référence qui vit plus longtemps que ce qu'elle désigne, donc rien comme les durées de
vie de Rust (`'a`) à écrire ni à comprendre.

```kova
fn first(l: List<String>) -> String {
    return l[0]
}

fn main {
    let l: List<String> := ["a"]
    print(first(l))
}
```

```txt
erreur: `l[0]` est un élément d'une List : on ne peut pas l'en sortir
 --> main.kova:2:12
  |
2 |     return l[0]
  |            ^^^^
  = aide : donne une copie avec `l[0].clone`, ou sors-le avec `l.remove(0)`
```

`first` a seulement emprunté la liste, donc elle ne peut pas en donner un des textes. Elle rend
une copie à la place (`return l[0].clone`), ou prend toute la liste (`l: move List<String>`) et
peut alors donner ses parties.

C'est pareil à l'intérieur d'une valeur : un champ ou un élément fait partie de son propriétaire,
et se lit, se change ou se copie sur place, jamais déplacé seul.

## Changer une valeur que quelqu'un lit

Comme chaque valeur a un seul propriétaire et que les marqueurs sont visibles, Kova peut aussi
refuser un changement pendant que la même valeur est lue, la source classique de plantages en C
et d'erreurs de « modification concurrente » ailleurs :

```kova
fn main {
    var names: List<String> := ["b", "a"]
    for n in names {
        names.add(n.clone)
    }
}
```

```txt
erreur: `names` est parcouru par une boucle `for` : il ne peut pas changer ni être déplacé dedans
 --> main.kova:4:9
  |
4 |         names.add(n.clone)
  |         ^^^^^
```

De même, un appel ne peut pas recevoir la même variable en `var` et en argument simple :
`both(var l, l)` est refusé, et l'aide propose de donner une copie, `l.clone`.

Pour changer chaque élément d'une liste, dis-le dans la boucle : `for s in var scores { s := s * 10 }`.
`for s in move scores` prend les éléments un par un, et la liste est consommée.

## Pourquoi ce choix

La possession de Kova est le modèle que Swift et Hylo appellent *mutable value semantics* : une
valeur se comporte comme si chaque variable avait la sienne, et rien n'est jamais partagé dans le
dos du programmeur. Comparé à :

- **un ramasse-miettes** : la mémoire est libérée à un endroit connu, le programme ne fait jamais
  de pause, et l'exécutable n'emporte pas de ramasse-miettes (un « hello » fait environ 30 Ko) ;
- **le vérificateur d'emprunts de Rust** : les références ne peuvent pas sortir de leur bloc, donc
  pas de durées de vie, et aucune struct ne contient de champ emprunté. Les programmes qui ont
  besoin de liens partagés et durables entre valeurs (un graphe, un cache) les gardent comme des
  positions ou des clés dans une `List` ou une `Map`, ce que les programmeurs Rust finissent
  souvent par faire aussi ;
- **le C** : aucun des quatre bugs mémoire classiques (fuite, double libération, usage après
  libération, changement pendant une lecture) ne peut s'écrire.

Le prix, c'est quelques `.clone` là où une valeur doit vraiment être à deux endroits, et un `var`
ou un `move` écrit aux appels qui changent ou prennent quelque chose. Les deux sont visibles exprès :
ce sont les endroits à regarder quand un programme est lent ou fait quelque chose d'inattendu.

## En bref

1. Un propriétaire par valeur ; elle est libérée quand son propriétaire disparaît.
2. `let b := a`, `return a`, mettre `a` dans une struct ou une liste le déplace ; les nombres et
   les `copy struct` sont copiés ; `.clone` copie n'importe quoi.
3. Un appel emprunte. `f(var a)` permet à `f` de changer `a` ; `f(move a)` donne `a`.
4. Un emprunt reste dans son bloc, donc il n'y a pas de durées de vie.
