---
title: JSON and CSV
description: Reading and writing JSON and tables, checked against your own types.
sidebar:
  order: 7
---

Kova reads JSON and CSV **into your own types**: you declare the struct you expect, and Kova
checks the text against it, field by field. A program never walks through untyped data, and a
missing or wrong field is a clear error, not a crash three functions later.

## JSON

Every struct has `T.from_json(text)` and `value.to_json`, without any import or declaration:

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

| Written | Gives |
| --- | --- |
| `Book.from_json(text) -> Book or JsonError` | the struct read from a JSON object |
| `book.to_json -> String` | the struct as a JSON object, on one line |
| `List<Book>.from_json(text)`, `books.to_json` | for a JSON array |
| `Map<String, Book>.from_json(text)`, `shelf.to_json` | for an object whose keys are not known before |

### What JSON can hold

Fields may be `Int`, `Float`, `Bool`, `String`, `Date` and `DateTime` (texts like `"2026-10-31"`),
an enum without fields (the name of its case, `"Red"`), `T?`, `List<T>`, `Map<String, T>` and structs
made of them. The compiler refuses `from_json` on a struct with another kind of field (a `Char`, an
enum with fields): say how to write it, as a `String` field, for instance.

- A field the struct does not have is ignored, so a program reads only what it needs from a large
  answer.
- A `T?` field that is missing or `null` is `none`; any other missing field is an error.
- A number with a fraction is refused for an `Int` field.

### When it fails

`JsonError` says where:

| Case | Means |
| --- | --- |
| `NotJson(line: Int)` | the text is not JSON; the line where it stops making sense |
| `Missing(field: String)` | a field the struct needs is not there |
| `WrongKind(field: String)` | a field holds another kind of value (a text for an `Int`…) |

The field is written as a path, `author.name` or `tags[2]`, to find it in a large text.

With the web, it is one line: `Book.from_json(page.body)` ([web](/library/web/)). In a Kova text,
write JSON as a [raw text](/guide/texts/#raw-texts), `"""{"title": "Dune"}"""`, so its braces and
quotes stay as they are.

## CSV

A table's first line names its columns. `List<Fruit>.from_csv(text)` finds each field's column by
its name, in any order (other columns are left out), and reads a `Fruit` per line:

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

| Written | Gives |
| --- | --- |
| `List<Fruit>.from_csv(text) -> List<Fruit> or CsvError` | a struct per line |
| `fruits.to_csv -> String` | the column names, then a line per element, with `,` |
| `fruits.to_csv separator(';')` | the same with another separator |
| `List<List<String>>.from_csv(text)`, `rows.to_csv` | every cell as written, the first line included, for a table whose columns are not known before |

- Fields may be `Int`, `Float`, `Bool`, `Char`, `String`, `Date`, `DateTime`, or one of them with
  `?`: an empty cell is then `none`.
- The separator is found in the first line: `,`, `;` or a tab.
- With `;`, a number may have a decimal comma (`2,5`), as spreadsheets write it in French and many
  other languages.
- A cell holding the separator, a quote or a line break is written between quotes, as the CSV
  standard (RFC 4180) says.

`CsvError` is `NotCsv(line)` (a `"` never closed), `Missing(column)` or `WrongKind(line, column)`.

To read or write the file itself, use [`files.read`](/library/files/) and `files.write`.
