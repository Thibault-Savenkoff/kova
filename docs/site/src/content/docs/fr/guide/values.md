---
title: Valeurs et types
description: Les variables, les types de base, les nombres, et pourquoi les types sont écrits.
sidebar:
  order: 1
---

## Les variables

Une variable est un nom pour une valeur. `let` en crée une qui ne change jamais, `var` une qui
peut changer :

```kova
fn main {
    let name: String := "Ada"
    var age: Int := 36
    age := age + 1
    age += 1
    print("{name} is {age}")
}
```

```text
Ada is 38
```

Deux signes, jamais confondus :

- `:=` **donne** une valeur : `age := 37`.
- `=` **compare** : `age = 37` vaut `true` ou `false`.

La plupart des langages utilisent `=` pour donner et `==` pour comparer, et un seul `=` oublié
transforme alors un test en changement (`if (x = 5)` en C). En Kova, cette erreur ne peut pas
s'écrire : `==` n'existe pas, et `:=` n'est pas permis là où une valeur est attendue.

`+=`, `-=`, `*=` et `/=` changent un `var` sur place : `age += 1` est `age := age + 1`.

Préfère `let` : une valeur qui ne peut pas changer, c'est une chose de moins à suivre en lisant le
code. Le compilateur refuse de changer un `let`, et propose de le déclarer avec `var` si c'était
voulu.

### Les constantes

Un `let` en haut d'un fichier est une constante, pour une valeur qui a un seul sens dans tout le
programme. Elle est faite de valeurs écrites et d'autres constantes, et son nom est libre : `limit`
ou `LIMIT`.

```kova
let LIMIT: Int := 3
let double: Int := LIMIT * 2

fn main {
    print(LIMIT)
    print(double)
}
```

```text
3
6
```

Sous `public:`, un autre fichier la lit avec `settings.limit`.

## Les types sont écrits

Chaque nom est introduit avec son type : `name: String`, `age: Int`. Le compilateur vérifie chaque
usage, et ne devine presque rien :

```kova
fn main {
    let x := 5
}
```

```txt
erreur: il faut écrire le type de `x`
 --> main.kova:2:9
  |
2 |     let x := 5
  |         ^
  = aide : écris `let x: Int := …`
```

Pourquoi écrire ce que le compilateur pourrait trouver ? Parce que le code est bien plus souvent
lu qu'écrit, et qu'un type est ce qu'il y a de plus utile à savoir sur un nom. Écrit à la
déclaration, il est là pour le lecteur, dans une relecture, un diff, un livre, sans éditeur à qui
demander.

Le type peut être omis seulement quand son nom est déjà écrit à droite, pour que rien ne soit
caché : `let rex := Dog { name := "Rex" }`, `let today := Date.today`.

Un nom ne peut pas être redéclaré tant qu'il est encore visible, même dans un bloc intérieur :
deux `total` différents dans la même fonction sont une source classique de confusion, alors Kova
demande un autre nom.

## Les types de base

| Type | Valeurs | S'écrit |
| --- | --- | --- |
| `Int` | nombres entiers, 64 bits (environ ±9,2 × 10¹⁸) | `42`, `-7`, `1_000_000`, `0xff` |
| `Float` | nombres à virgule, 64 bits | `3.14`, `2.0`, `1e-3` |
| `Bool` | `true`, `false` | |
| `Char` | un caractère Unicode | `'a'`, `'é'` |
| `String` | un texte | `"hello"`, `"""brut"""` ([String](/fr/library/string/)) |
| `BigInt` | nombres entiers sans limite | `123456789012345678901234567890` |

Les dates, les listes, les maps et tes propres types viennent dans les chapitres suivants.

## Les nombres

```kova
fn main {
    print(7 / 2)
    print(-7 / 2)
    print(-7 % 2)
    print(7.0 / 2.0)
    print(0.1 + 0.2)
    print(2.0.sqrt)
    print(3.7.to_int)
    print(2.5.round)
}
```

```text
3
-3
-1
3.5
0.30000000000000004
1.4142135623730951
3
3
```

- `/` entre `Int` donne un `Int`, coupé vers zéro ; `%` est le reste, du signe du nombre divisé.
- Les `Float` suivent les règles habituelles d'IEEE 754, donc `0.1 + 0.2` ne vaut pas exactement
  `0.3`, comme dans tous les langages : l'affichage ne le cache simplement pas.
- `Int` et `Float` ne se mélangent jamais tout seuls : `n.to_float` et `x.to_int` (coupé vers
  zéro), `x.round`, `x.floor`, `x.ceil` disent ce qu'on veut. Un nombre écrit dans le code devient
  un `Float` là où on en attend un (`let x: Float := 2`), mais une variable n'est jamais convertie
  en silence :

```kova
fn main {
    let a: Int := 7
    let b: Float := a
}
```

```txt
erreur: types différents : `Float` attendu, `Int` donné
 --> main.kova:3:21
  |
3 |     let b: Float := a
  |                     ^
  = aide : Kova ne convertit jamais une variable tout seul : utilise `.to_float` ou `.to_int`
```

### Pas de débordement silencieux

Un `Int` qui dépasse sa limite ne repart pas vers un nombre négatif, comme en C, Java ou Go : le
programme s'arrête et dit où. Pareil pour une division par zéro, `Int` ou `Float` :

```kova
fn main {
    let n: Int := 9223372036854775807
    print(n + 1)
}
```

```txt
erreur: dépassement d'entier
  --> main.kova:3:13
```

Un résultat faux qui passe inaperçu est pire qu'un arrêt. Quand les nombres sont vraiment grands,
`BigInt` n'a pas de limite :

```kova
fn main {
    let x: BigInt := 123456789012345678901234567890
    print(x * x)
}
```

```text
15241578753238836750495351562536198787501905199875019052100
```

### Les mêmes chiffres partout

`kova run`, `kova build`, Linux, Windows, macOS, Intel ou ARM : un programme affiche les mêmes
chiffres, `sin` et `pow` compris. Kova apporte ses propres maths au lieu d'utiliser celles de
chaque système, qui diffèrent sur le dernier chiffre. Un test écrit sur une machine passe sur
toutes.

Les méthodes des nombres (`abs`, `min`, `pow`, `sqrt`, `sin`…) sont listées dans
[Nombres](/fr/library/numbers/).

## La logique

`&&` (et), `||` (ou) et `!` (non) combinent des `Bool` ; le côté droit de `&&` et `||` n'est
calculé que si besoin. Les comparaisons sont `=`, `!=`, `<`, `<=`, `>`, `>=`, sur les nombres, les
`Char`, les textes, et tout type qui [les permet](/fr/guide/interfaces-and-generics/).

## Les tuples

Un tuple regroupe quelques valeurs de types différents, sans donner de nom au groupe. C'est
pratique pour rendre deux résultats à la fois :

```kova
fn min_max(a: Int, b: Int) -> (Int, Int) {
    if a < b {
        return (a, b)
    }
    return (b, a)
}

fn main {
    let (low, high): (Int, Int) := min_max(9, 4)
    print(low)
    print(high)
    let pair: (String, Int) := ("Rex", 3)
    print(pair.0)
    var a: Int := 1
    var b: Int := 2
    (a, b) := (b, a)
    print("{a} {b}")
}
```

```text
4
9
Rex
2 1
```

`pair.0`, `pair.1` lisent les parties. `let (low, high): (Int, Int) := …` les sépare, et
`(a, b) := (b, a)` échange deux variables : tout ce qui est à droite est lu avant que quoi que ce
soit soit écrit. Quand les parties méritent des noms, fais plutôt une [struct](/fr/guide/structs/).
