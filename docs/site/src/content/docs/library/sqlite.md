---
title: sqlite
description: A database in one file, with SQL.
sidebar:
  order: 24
---

```kova
import sqlite
```

SQLite keeps a whole database in one file, with no server to install or start. It suits a program's
own data, up to a few gigabytes: settings, notes, a site's content.

This module uses the SQLite the system already has: it comes with macOS and Windows, and with
nearly every Linux. Your program needs nothing else at build time. Where a Linux lacks it, the
program stops when it first calls SQLite, with the name of the library it did not find; install the
package `libsqlite3-0` or `sqlite-libs`.

| Function or method | What it does |
| --- | --- |
| `sqlite.open(path) -> sqlite.Database or sqlite.Error` | opens a database file, made if missing. `":memory:"` is one that lives until `close`. |
| `db.run(statement) with(values)` | runs one statement that gives no lines: `INSERT`, `UPDATE`, `CREATE TABLE`. Gives `Nothing or sqlite.Error`. `with` is optional. |
| `db.query(statement) with(values) -> List<sqlite.Row> or sqlite.Error` | runs one statement and gives all its lines. |
| `db.each(statement) { row: sqlite.Row => … }` | gives each line to the block as it comes, keeping none in memory: for tables too large to load. Gives `Nothing or sqlite.Error`. `db.each(statement) with(values) { … }` fills the `?`. |
| `db.script(statements)` | runs several statements separated by `;`, none with values. Gives `Nothing or sqlite.Error`. |
| `db.last_id -> Int` | the number given to the last line inserted. |
| `db.changes -> Int` | how many lines the last `INSERT`, `UPDATE` or `DELETE` touched. |
| `db.close` | closes the database. |
| `row.text(name) -> String?` | the column of that name. `none` if it is absent, `NULL` or of another type. |
| `row.int(name) -> Int?` | the same for an integer. |
| `row.float(name) -> Float?` | the same for a number; an integer reads as a `Float` too. |
| `row.bytes(name) -> List<Int>?` | the same for a `BLOB`. |
| `row.get(name) -> sqlite.Value?` | the column as it is, whatever its type. |

```kova
import sqlite

fn main {
    let db: sqlite.Database := sqlite.open(":memory:")?
    db.run("CREATE TABLE people (name TEXT, age INTEGER)")?
    db.run("INSERT INTO people VALUES (?, ?)") with([.Text("Ada"), .Int(36)])?
    db.run("INSERT INTO people VALUES (?, ?)") with([.Text("Linus"), .Null])?

    for row in db.query("SELECT name, age FROM people ORDER BY name")? {
        let name: String := row.text("name") else "?"
        let age: Int := row.int("age") else 0
        print("{name}: {age}")
    }
    db.close
}
```

```text
Ada: 36
Linus: 0
```

## Never build a statement from a text you were given

Each `?` in a statement is filled from `with`, in order. A `sqlite.Value` is `.Null`, `.Int(n)`,
`.Float(x)`, `.Text(s)` or `.Bytes(data)`.

Writing the value straight into the statement looks simpler, and it is the classic security
hole of databases. With `"… WHERE name = '{name}'"`, a visitor who types `' OR '1'='1` changes what
the statement means, and reads every line. A value given through `?` is only ever a value.

## Several changes at once

A transaction makes several changes count as one: all of them happen, or none. It also makes many
inserts far faster, since the file is written once:

```kova
import sqlite

fn fill(db: sqlite.Database) -> Nothing or sqlite.Error {
    db.script("BEGIN")?
    for i in 0..100 {
        db.run("INSERT INTO numbers VALUES (?)") with([.Int(i)])?
    }
    db.script("COMMIT")?
    return ok
}

fn main {
    let db: sqlite.Database := sqlite.open(":memory:")?
    db.script("CREATE TABLE numbers (n INTEGER)")?
    fill(db)?
    print(db.query("SELECT count(*) AS total FROM numbers")?[0].int("total"))
    db.close
}
```

```text
100
```

If a step fails, run `db.script("ROLLBACK")` to undo what came before it.

## Errors

`sqlite.Error` has three cases, each with SQLite's message:

- `Constraint(message)`: a rule of the table refused the change (`UNIQUE`, `NOT NULL`, a foreign key).
- `Busy(message)`: another connection holds the file. Try again a little later.
- `Failed(message)`: anything else, such as a mistake in the statement.

`run`, `query` and `each` take one statement: given more, they run nothing and give `Failed`. Use
`script` for several.
