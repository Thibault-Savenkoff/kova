---
title: Nombres, Bool et Char
description: Int, Float, BigInt, Bool et Char, leurs opérateurs et leurs méthodes.
sidebar:
  order: 3
---

## Les opérateurs

| Opérateur | Sur | Donne |
| --- | --- | --- |
| `+ - *` | `Int`, `Float`, `BigInt` | la somme, la différence, le produit |
| `/` | `Int`, `BigInt` | le quotient, coupé vers zéro (`7 / 2` vaut `3`, `-7 / 2` vaut `-3`) |
| `/` | `Float` | le quotient exact (`7.0 / 2.0` vaut `3.5`) |
| `%` | `Int`, `BigInt` | le reste, du signe du nombre divisé |
| `-x` | `Int`, `Float`, `BigInt` | l'opposé |
| `= != < <= > >=` | nombres, `Char`, `Bool` (`=`, `!=`) | un `Bool` |
| `&& \|\| !` | `Bool` | et, ou, non ; le côté droit n'est calculé que si besoin |

Les deux côtés d'un opérateur ont le même type : `n.to_float * 1.5`, jamais `n * 1.5`.

Un résultat trop grand pour un `Int` arrête le programme (`dépassement d'entier`), et une division
par zéro aussi, pour `Int`, `BigInt` et `Float`. Le programme ne continue jamais avec une valeur
fausse.

:::caution[Un moins devant un nombre]
Une méthode s'attache plus fort que `-` : `-5.abs` est `-(5.abs)`, donc `-5`. Écris `(-5).abs` pour
`5`, et `(-2.5).floor` pour `-3`.
:::

## `Int`

Un nombre entier de 64 bits, de −9 223 372 036 854 775 808 à 9 223 372 036 854 775 807. S'écrit
`42`, `-7`, `1_000_000` (les `_` sont ignorés), `0xff` (hexadécimal).

| Méthode | Donne |
| --- | --- |
| `n.abs -> Int` | la valeur sans son signe. |
| `n.min(other: Int) -> Int` | le plus petit des deux. |
| `n.max(other: Int) -> Int` | le plus grand des deux. |
| `n.pow(e: Int) -> Int` | `n` à la puissance `e` ; `e` ne doit pas être négatif. |
| `n.to_float -> Float` | le même nombre en `Float`. |
| `n.to_big -> BigInt` | le même nombre en `BigInt`. |
| `n.to_char -> Char?` | le caractère dont le code Unicode est `n` (`65.to_char` est `'A'`), `none` s'il n'y en a pas. |
| `n.to_string -> String` | ses chiffres. |

```kova
fn main {
    print((-5).abs)
    print(3.min(8))
    print(2.pow(10))
    print(7.to_float / 2.0)
    print(65.to_char)
}
```

```text
5
3
1024
3.5
'A'
```

## `Float`

Un nombre à virgule, 64 bits (`double` IEEE 754), environ 15 à 17 chiffres significatifs. S'écrit
`3.14`, `2.0`, `1e-3`. Un nombre entier écrit dans le code devient un `Float` là où on en attend
un : `let x: Float := 2`.

| Méthode | Donne |
| --- | --- |
| `x.abs -> Float` | la valeur sans son signe. |
| `x.min(other: Float) -> Float`, `x.max(other: Float) -> Float` | le plus petit, le plus grand. |
| `x.floor -> Int` | le plus grand entier inférieur ou égal (`(-2.5).floor` vaut `-3`). |
| `x.ceil -> Int` | le plus petit entier supérieur ou égal. |
| `x.round -> Int` | l'entier le plus proche, les demis loin de zéro (`2.5.round` vaut `3`). |
| `x.to_int -> Int` | la partie entière, coupée vers zéro (`(-2.7).to_int` vaut `-2`). |
| `x.sqrt -> Float` | la racine carrée (`NaN` pour un nombre négatif). |
| `x.pow(e: Int) -> Float`, `x.pow(e: Float) -> Float` | `x` à la puissance `e`. |
| `x.exp -> Float`, `x.ln -> Float`, `x.log10 -> Float` | e puissance `x`, le logarithme népérien, le logarithme décimal. |
| `x.sin`, `x.cos`, `x.tan -> Float` | la trigonométrie, `x` en radians. |
| `x.asin`, `x.acos`, `x.atan -> Float` | les fonctions réciproques, en radians. |
| `Float.pi -> Float` | π, 3.141592653589793. |
| `x.to_string -> String` | son texte exact le plus court : `0.1` reste `0.1`. |

`x.floor`, `x.ceil`, `x.round` et `x.to_int` arrêtent le programme quand le résultat ne tient pas
dans un `Int`.

```kova
fn main {
    let r: Float := 2.0
    print(Float.pi * r.pow(2))
    print(2.0.sqrt)
    print((Float.pi / 2.0).sin)
    print(1.0.exp)
    print(3.7.round)
}
```

```text
12.566370614359172
1.4142135623730951
1.0
2.718281828459045
4
```

Chacune donne les mêmes chiffres sur chaque système, compilée ou lancée : Kova les calcule
lui-même, correctement arrondies là où ça compte (`exp`, `ln`, `log10`, `pow`), au lieu de les
demander au système, dont les bibliothèques diffèrent sur le dernier chiffre.

## `BigInt`

Un nombre entier sans limite, pour ce qui ne tient pas en 64 bits : factorielles, cryptographie,
comptes exacts. Un nombre trop grand pour un `Int`, écrit dans le code, est un `BigInt` ; un plus
petit devient un `BigInt` là où on en attend un.

| Méthode | Donne |
| --- | --- |
| `b.to_int -> Int?` | le même nombre en `Int`, `none` s'il ne tient pas. |
| `b.to_string -> String` | ses chiffres. |

Les opérateurs `+ - * / %`, les comparaisons et `-b` marchent sur les `BigInt`, et `n.to_big`
transforme un `Int` en `BigInt`.

```kova
fn main {
    var f: BigInt := 1
    from i := 1 to 30 {
        f := f * i.to_big
    }
    print(f)
    print(f.to_int)
}
```

```text
265252859812191058636308480000000
none
```

## `Bool`

`true` ou `false`. `&&`, `||` et `!` les combinent ; `if` et `while` les prennent. `b.to_string`
vaut `"true"` ou `"false"`.

## `Char`

Un caractère Unicode, écrit entre apostrophes : `'a'`, `'é'`, `'\n'`. Un `String` en est fait
(`s.chars`).

| Méthode | Donne |
| --- | --- |
| `c.to_int -> Int` | son code Unicode (`'A'.to_int` vaut `65`). |
| `c.to_string -> String` | un texte de ce seul caractère. |

Les caractères se comparent par leurs codes : `'a' < 'b'`.

## Chaque nombre a aussi

- `x.clone` : une copie (pour les nombres, la valeur elle-même) ;
- `x.to_string` : le texte que `print` écrit.
