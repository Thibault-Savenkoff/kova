---
title: JSON et CSV
description: Lire et écrire du JSON et des tableaux, vérifiés contre tes propres types.
sidebar:
  order: 7
---

Kova lit le JSON et le CSV **dans tes propres types** : tu déclares la struct que tu attends, et
Kova vérifie le texte contre elle, champ par champ. Un programme ne parcourt jamais de données sans
type, et un champ manquant ou faux est une erreur claire, pas un plantage trois fonctions plus
loin.

## JSON

Chaque struct a `T.from_json(text)` et `value.to_json`, sans import ni déclaration :

```kova
struct Author {
    name: String
    born: Int?
}

struct Book {
    title: String
    tags: List<String>
    author: Author
}

fn main {
    let b := Book { title := "Dune", tags := ["sf"], author := Author { name := "Herbert", born := 1920 } }
    let text: String := b.to_json
    print(text)
    match Book.from_json(text) {
        ok(book) => print(book.author.name)
        err(e) => print("not a book: {e}")
    }
    print(Book.from_json("""{"title": "Dune"}"""))
}
```

```text
{"title":"Dune","tags":["sf"],"author":{"name":"Herbert","born":1920}}
Herbert
err(Missing("tags"))
```

| Écrit | Donne |
| --- | --- |
| `Book.from_json(text) -> Book or JsonError` | la struct lue depuis un objet JSON |
| `book.to_json -> String` | la struct en objet JSON, sur une ligne |
| `List<Book>.from_json(text)`, `books.to_json` | pour un tableau JSON |
| `Map<String, Book>.from_json(text)`, `shelf.to_json` | pour un objet dont les clés ne sont pas connues d'avance |

### Ce que le JSON peut contenir

Les champs peuvent être `Int`, `Float`, `Bool`, `String`, `Date` et `DateTime` (des textes comme
`"2026-10-31"`), une enum sans champs (le nom de son cas, `"Red"`), `T?`, `List<T>`,
`Map<String, T>` et des structs faites de ceux-ci. Le compilateur refuse `from_json` sur une struct
qui a un autre genre de champ (un `Char`, une enum avec des champs) : dis comment l'écrire, avec un champ `String` par exemple.

- Un champ que la struct n'a pas est ignoré, donc un programme ne lit que ce dont il a besoin dans
  une grosse réponse.
- Un champ `T?` absent ou `null` vaut `none` ; tout autre champ absent est une erreur.
- Un nombre à virgule est refusé pour un champ `Int`.

### Quand ça échoue

`JsonError` dit où :

| Cas | Veut dire |
| --- | --- |
| `NotJson(line: Int)` | le texte n'est pas du JSON ; la ligne où il cesse d'avoir un sens |
| `Missing(field: String)` | un champ dont la struct a besoin n'y est pas |
| `WrongKind(field: String)` | un champ contient un autre genre de valeur (un texte pour un `Int`…) |

Le champ est écrit comme un chemin, `author.name` ou `tags[2]`, pour le retrouver dans un gros
texte.

Avec le web, c'est une ligne : `Book.from_json(page.body)` ([web](/fr/library/web/)). Dans un texte
Kova, écris le JSON en [texte brut](/fr/guide/texts/#les-textes-bruts), `"""{"title": "Dune"}"""`,
pour que ses accolades et ses guillemets restent tels quels.

## CSV

La première ligne d'un tableau nomme ses colonnes. `List<Fruit>.from_csv(text)` trouve la colonne
de chaque champ par son nom, dans n'importe quel ordre (les autres colonnes sont laissées de côté),
et lit un `Fruit` par ligne :

```kova
struct Fruit {
    name: String
    price: Float
    stock: Int?
}

fn main {
    let text: String := """
        name;price;stock
        apple;2,5;12
        "pear, green";3;
        """
    match List<Fruit>.from_csv(text) {
        ok(fruits) => {
            print(fruits[1].name)
            print(fruits.to_csv)
        }
        err(e) => print("not a table: {e}")
    }
    print(List<Fruit>.from_csv("name,stock\nplum,4"))
}
```

```text
pear, green
name,price,stock
apple,2.5,12
"pear, green",3.0,

err(Missing("price"))
```

| Écrit | Donne |
| --- | --- |
| `List<Fruit>.from_csv(text) -> List<Fruit> or CsvError` | une struct par ligne |
| `fruits.to_csv -> String` | les noms des colonnes, puis une ligne par élément, avec `,` |
| `fruits.to_csv separator(';')` | pareil avec un autre séparateur |
| `List<List<String>>.from_csv(text)`, `rows.to_csv` | chaque cellule telle qu'écrite, première ligne comprise, pour un tableau dont les colonnes ne sont pas connues d'avance |

- Les champs peuvent être `Int`, `Float`, `Bool`, `Char`, `String`, `Date`, `DateTime`, ou l'un
  d'eux avec `?` : une cellule vide vaut alors `none`.
- Le séparateur est trouvé dans la première ligne : `,`, `;` ou une tabulation.
- Avec `;`, un nombre peut avoir une virgule décimale (`2,5`), comme l'écrivent les tableurs en
  français et dans beaucoup d'autres langues.
- Une cellule qui contient le séparateur, un guillemet ou un saut de ligne est écrite entre
  guillemets, comme le dit la norme CSV (RFC 4180).

`CsvError` vaut `NotCsv(line)` (un `"` jamais fermé), `Missing(column)` ou
`WrongKind(line, column)`.

Pour lire ou écrire le fichier lui-même, utilise [`files.read`](/fr/library/files/) et
`files.write`.
