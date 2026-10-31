---
title: Textes
description: Les String, les valeurs dans les textes, les textes bruts et les caractères.
sidebar:
  order: 4
---

Un `String` contient n'importe quel texte, dans n'importe quelle écriture, et `{…}` y met une
valeur :

```kova
fn main {
    let name: String := "Ada"
    let year: Int := 1843
    print("{name} wrote the first program in {year}.")
    print("In {2026 - year} years, {name.length} letters.")
}
```

```text
Ada wrote the first program in 1843.
In 183 years, 3 letters.
```

Entre les accolades va un nom, un appel, un calcul : toute expression sans guillemets ni
accolades à elle. Sa valeur doit avoir un **texte pour les gens** : les nombres, `Bool`, `Char`,
`String`, les dates, et les types qui suivent
[`Display`](/fr/guide/interfaces-and-generics/#les-interfaces-intégrées), qui écrivent leur propre
`to_string`. Une liste ou une struct n'a pas de tel texte par elle-même, puisqu'il n'y a pas une
seule bonne façon de la montrer à un utilisateur :

```kova
struct Dog {
    name: String
}

fn main {
    let rex := Dog { name := "Rex" }
    print("my dog: {rex}")
}
```

```txt
erreur: `rex` (`Dog`) n'a pas de texte pour les gens
 --> main.kova:7:21
  |
7 |     print("my dog: {rex}")
  |                     ^^^
  = aide : écris `struct Dog : Display` et sa méthode `fn to_string(self) -> String`
```

`print(rex)` seul, en revanche, montre n'importe quelle valeur telle qu'elle est, champs compris :
c'est pour le programmeur, pendant le débogage.

```kova
struct Dog {
    name: String
}

fn main {
    print(Dog { name := "Rex" })
    print([1, 2, 3])
    print("12".to_int)
}
```

```text
Dog { name := "Rex" }
[1, 2, 3]
12
```

## Les caractères spéciaux

| Écrit | Donne |
| --- | --- |
| `\n` | un saut de ligne |
| `\t` | une tabulation |
| `\"` | `"` |
| `\\` | `\` |
| `\{` | `{` sans commencer une valeur |

## Les textes bruts

Entre `"""`, un texte est **brut** : accolades, guillemets et barres obliques inverses restent tels
qu'écrits. C'est la façon d'écrire du JSON, du HTML, des chemins Windows ou des
[motifs](/fr/library/string/#les-motifs) :

```kova
fn main {
    print("""{"title": "Dune"}""")
    print("""C:\Users\ada""")
    let page: String := """
        <ul>
          <li>one</li>
        </ul>
        """
    print(page)
}
```

```text
{"title": "Dune"}
C:\Users\ada
<ul>
  <li>one</li>
</ul>
```

Sur plusieurs lignes, le texte commence à la ligne qui suit le `"""` ouvrant. Le `"""` fermant,
seul sur sa ligne, dit combien d'indentation retirer de chaque ligne, pour que le texte puisse
être indenté avec le code qui l'entoure.

## Les textes sont des valeurs

Un `String` possède ses caractères : le donner à une autre variable le
[déplace](/fr/guide/ownership/), et `.clone` le copie. Ses méthodes ne le changent jamais sur
place : `s.to_upper` donne un nouveau texte et `s` reste tel quel.

```kova
fn main {
    let s: String := "  Hello, World  "
    let clean: String := s.trim.to_lower
    print(clean)
    print(clean.split(", "))
    print(clean.replace("world") with("Kova"))
    print(clean.starts_with("hello"))
}
```

```text
hello, world
["hello", "world"]
hello, Kova
true
```

Les positions comptent des caractères, pas des octets : `"héllo".length` vaut 5 et
`"héllo"[1..3]` vaut `"él"`.

## Les caractères

Un `Char` est un caractère, entre apostrophes : `'a'`, `'é'`, `'\n'`. `s.chars` donne les
caractères d'un texte, `c.to_int` le code d'un caractère et `n.to_char` le caractère d'un code (un
`Char?` : tout nombre n'en est pas un). `print` montre un `Char` à l'intérieur d'une autre valeur
avec ses apostrophes.

```kova
fn main {
    let codes: List<Int> := "Kova".chars.map { c: Char => c.to_int }
    print(codes)
    print(97.to_char)
}
```

```text
[75, 111, 118, 97]
'a'
```

Toutes les méthodes des textes, de `split` aux motifs et au base64, sont dans
[String](/fr/library/string/).
