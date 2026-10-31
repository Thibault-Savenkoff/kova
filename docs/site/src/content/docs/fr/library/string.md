---
title: String
description: Les textes en Kova, et toutes leurs méthodes.
sidebar:
  order: 2
---

Un `String` est un texte : n'importe quels caractères Unicode, autant qu'on veut. Il s'écrit entre
guillemets droits, et il possède ses caractères, donc il se [déplace](/fr/guide/ownership/) comme
toute valeur qui possède de la mémoire (`.clone` le copie).

```kova
fn main {
    let name: String := "Ada"
    let line: String := "{name} wrote the first program in {1843}"
    print(line)
    print(line.length)
    print(name + " Lovelace")
}
```

```text
Ada wrote the first program in 1843
35
Ada Lovelace
```

## Écrire des textes

- `{…}` dans un texte y met une valeur : un nom, un appel, un calcul (`"{a + b}"`). La valeur doit
  avoir un texte pour les gens : les nombres, `Bool`, `Char`, `String`, les dates, et les types
  qui suivent [`Display`](/fr/guide/interfaces-and-generics/#les-interfaces-intégrées)
  ([Textes](/fr/guide/texts/)).
- `\n` est un saut de ligne, `\t` une tabulation, `\"` un guillemet, `\\` une barre oblique
  inverse, et `\{` une accolade qui n'ouvre pas de valeur.
- Un **texte brut**, entre `"""`, garde tout tel qu'écrit, accolades et barres obliques inverses
  comprises : `"""{"title": "Dune"}"""`. Sur plusieurs lignes, il commence à la ligne qui suit son
  `"""`, et l'indentation du `"""` fermant est retirée de chaque ligne :

```kova
fn main {
    let page: String := """
        <h1>{title}</h1>
          <p>C:\Users</p>
        """
    print(page)
}
```

```text
<h1>{title}</h1>
  <p>C:\Users</p>
```

- `+` joint deux textes. Il ne transforme jamais un nombre en texte tout seul : écris `"{n}"` ou
  `n.to_string`.
- `=`, `!=`, `<`, `<=`, `>`, `>=` comparent des textes, `<` dans l'ordre des codes de leurs
  caractères (`"B" < "a"`).

Les positions dans un texte comptent des **caractères**, à partir de 0, pas des octets :
`"héllo".length` vaut 5, et `é` est un caractère.

## Regarder dedans

| Méthode | Donne |
| --- | --- |
| `s.length -> Int` | le nombre de caractères. |
| `s.contains(part: String) -> Bool` | si `part` est quelque part dans `s` (`""` est dans tout texte). |
| `s.starts_with(part: String) -> Bool` | si `s` commence par `part`. |
| `s.ends_with(part: String) -> Bool` | si `s` finit par `part`. |
| `s.find(part: String) -> Int?` | la position du premier `part`, ou `none` s'il n'y en a pas. |
| `s[a..b] -> String` | les caractères de la position `a` jusqu'à `b` non compris (`s[a..=b]` le comprend). Après la fin, le programme s'arrête. |
| `s.chars -> List<Char>` | chaque caractère, dans l'ordre. |

```kova
fn main {
    let s: String := "héllo world"
    print(s.contains("world"))
    print(s.find("o"))
    print(s.find("z"))
    print(s[0..5])
    print(s.chars[1])
}
```

```text
true
4
none
héllo
é
```

## Faire un nouveau texte

Un `String` ne change jamais sur place : ces méthodes donnent un nouveau texte et laissent `s` tel
quel.

| Méthode | Donne |
| --- | --- |
| `s.trim -> String` | `s` sans espaces, tabulations ni sauts de ligne aux deux bouts. |
| `s.is_empty -> Bool` | si `s` n'a aucun caractère. |
| `s.lines -> List<String>` | les lignes de `s`, sans leurs sauts de ligne (`\n` ou `\r\n`). |
| `s.padded(width: Int) -> String` | `s` suivi d'espaces, jusqu'à `width` caractères : les colonnes d'un tableau. Un texte plus long reste tel quel. |
| `s.padded_left(width: Int) -> String` | pareil avec les espaces avant : pour les nombres. |
| `s.reversed -> String` | les caractères de `s`, le dernier en premier. |
| `s.to_upper -> String` | `s` en majuscules, lettres accentuées comprises (`"été"` → `"ÉTÉ"`). |
| `s.to_lower -> String` | `s` en minuscules. |
| `s.replace(old: String) with(new: String) -> String` | `s` avec chaque `old` remplacé par `new`. `old` ne peut pas être `""`. |
| `s.repeat(n: Int) -> String` | `s` écrit `n` fois à la suite. |
| `s.wrapped(width: Int) -> String` | `s` coupé en lignes d'au plus `width` caractères, entre les mots. |

```kova
fn main {
    print("  hello \n".trim)
    print("Été".to_upper)
    print("a-b-a".replace("a") with("x"))
    print("ab".repeat(3))
    print("Kova checks every pattern before the program runs.".wrapped(20))
}
```

```text
hello
ÉTÉ
x-b-x
ababab
Kova checks every
pattern before the
program runs.
```

## Couper et joindre

| Méthode | Donne |
| --- | --- |
| `s.split(separator: String) -> List<String>` | les morceaux entre chaque `separator` ; deux séparateurs à la suite donnent un morceau vide. `separator` ne peut pas être `""`. |
| `s.words -> List<String>` | les mots, où que soient les espaces, tabulations et sauts de ligne, jamais un vide. |
| `l.join(separator: String) -> String` | sur une `List<String>` : ses textes avec `separator` entre chacun. |

```kova
fn main {
    print("a,,b".split(","))
    print("  the quick\tfox ".words)
    print(["x", "y", "z"].join(" + "))
}
```

```text
["a", "", "b"]
["the", "quick", "fox"]
x + y + z
```

## Transformer un texte en valeur

Chacune donne `none` quand le texte n'est pas exactement une valeur de ce genre, espaces compris,
pour que le programme décide quoi faire alors :

| Méthode | Lit |
| --- | --- |
| `s.to_int -> Int?` | un nombre entier : `"42"`, `"-5"` (pas `"+5"`, `" 5"` ni `"1_000"`). |
| `s.to_float -> Float?` | un nombre avec un point : `"2.5"`, `"1e3"`, `"-0.1"`. |
| `s.to_bool -> Bool?` | exactement `"true"` ou `"false"`. |
| `s.to_bigint -> BigInt?` | un nombre entier de n'importe quelle taille. |
| `s.to_date -> Date?` | une date écrite `"2026-10-23"`, qui doit exister (`"2026-02-30"` donne `none`). |
| `s.to_datetime -> DateTime?` | une date et une heure écrites `"2026-10-23 14:05:09"`. |

```kova
fn main {
    match "42".to_int {
        none => print("not a number")
        n => print(n + 1)
    }
    print("forty".to_int)
    print("2.5".to_float)
    print("2026-10-23".to_date)
}
```

```text
43
none
2.5
2026-10-23
```

Dans l'autre sens, chaque valeur affichable a `to_string`, le texte que `print` écrirait.

## Les motifs

Un motif décrit à quoi ressemble un texte, pour le vérifier, en trouver des morceaux ou les
changer. Les motifs de Kova sont les expressions régulières habituelles :

| Motif | Correspond à |
| --- | --- |
| `\d`, `\w`, `\s` | un chiffre ; une lettre, un chiffre ou `_` ; un espace, une tabulation ou un saut de ligne |
| `.` | n'importe quel caractère |
| `[a-z]`, `[^,]` | un de ces caractères ; n'importe quel caractère sauf ceux-ci |
| `x*`, `x+`, `x?`, `x{2,5}` | `x` répété : autant de fois qu'on veut, au moins une fois, au plus une fois, de 2 à 5 fois (`*?`, `+?` : le moins possible) |
| `a\|b` | `a` ou `b` |
| `(…)` | un groupe, pour le répéter ou le lire avec `groups` |
| `^`, `$`, `\b` | le début du texte, sa fin, le bord d'un mot |

| Méthode | Donne |
| --- | --- |
| `s.matches(pattern: String) -> Bool` | si **tout** le texte correspond au motif. |
| `s.all_matches(pattern: String) -> List<String>` | chaque morceau de `s` qui correspond, de gauche à droite. |
| `s.replace_matches(pattern: String) with(new: String) -> String` | `s` avec chaque morceau qui correspond remplacé par `new`. |
| `s.groups(pattern: String) -> List<String>?` | les groupes `(…)` du premier morceau qui correspond, ou `none` si aucun. |

```kova
fn main {
    let line: String := "12 apples, 7 pears and 145 plums"
    print(line.all_matches("""\d+"""))
    print("ana@example.org".matches("[a-z.]+@[a-z.]+"))
    print("a   b \t c".replace_matches("""\s+""") with(" "))
    let parts: List<String>? := "on 2026-10-25".groups("""(\d+)-(\d+)-(\d+)""")
    if parts != none {
        print("{parts[2]}/{parts[1]}/{parts[0]}")
    }
}
```

```text
["12", "7", "145"]
true
a b c
25/10/2026
```

Le motif est écrit dans l'appel, donc le compilateur le lit et refuse un motif cassé avant que le
programme tourne. Écris-le en texte brut quand il contient un `\` : `"""\d+"""` plutôt que
`"\\d+"`. Kova exécute les motifs avec son propre moteur, dont le temps grandit avec la longueur du
texte et n'explose jamais, quel que soit le motif.

## Empreintes et base64

| Méthode | Donne |
| --- | --- |
| `s.sha256 -> String` | l'empreinte SHA-256 du texte, 64 chiffres hexadécimaux. |
| `s.hmac_sha256(key: String) -> String` | la signature HMAC-SHA-256 du texte avec une clé secrète. |
| `s.to_base64 -> String` | le texte écrit en base64. |
| `s.to_base64url -> String` | pareil, avec l'alphabet des adresses et sans `=`. |
| `String.from_base64(code: String) -> String?` | le texte à partir du base64, `none` si `code` n'est pas le base64 d'un texte. |
| `String.from_base64url(code: String) -> String?` | pareil pour `to_base64url`. |

```kova
fn main {
    print("hello".sha256)
    print("Kova".to_base64)
    print(String.from_base64("S292YQ=="))
}
```

```text
2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
S292YQ==
"Kova"
```

Une empreinte n'est pas une façon de garder des mots de passe : utilise
[`secure.password_hash`](/fr/library/secure/) pour ça.

## Les couleurs

`s.red`, `s.green`, `s.yellow`, `s.blue`, `s.magenta`, `s.cyan`, `s.gray`, `s.bold` et
`s.underline` donnent le texte dans cette couleur ou ce style, et peuvent s'enchaîner :
`"error:".red.bold`. Les couleurs ne se voient que dans un terminal : écrit dans un fichier ou
passé à un autre programme, le texte reste simple. `NO_COLOR=1` le garde simple dans un terminal
aussi, et `FORCE_COLOR=1` le colore partout.

## Chaque texte a aussi

- `s.clone -> String` : une copie.
- `s.to_string -> String` : le texte lui-même.
