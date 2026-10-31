---
title: sqlite
description: Une base de données dans un fichier, avec SQL.
sidebar:
  order: 24
---

```kova
import sqlite
```

SQLite garde toute une base de données dans un seul fichier, sans serveur à installer ni à lancer.
Elle convient aux données propres d'un programme, jusqu'à quelques gigaoctets : réglages, notes,
contenu d'un site.

Ce module utilise le SQLite que le système a déjà : il vient avec macOS et Windows, et avec presque
tous les Linux. Ton programme n'a besoin de rien d'autre à la compilation. Sur un Linux qui ne l'a
pas, le programme s'arrête au premier appel à SQLite, en nommant la bibliothèque introuvable ;
installe le paquet `libsqlite3-0` ou `sqlite-libs`.

| Fonction ou méthode | Ce qu'elle fait |
| --- | --- |
| `sqlite.open(path) -> sqlite.Database or sqlite.Error` | ouvre un fichier de base de données, créé s'il manque. `":memory:"` en donne une qui vit jusqu'à `close`. |
| `db.run(statement) with(values)` | exécute une instruction qui ne donne pas de lignes : `INSERT`, `UPDATE`, `CREATE TABLE`. Donne `Nothing or sqlite.Error`. `with` est facultatif. |
| `db.query(statement) with(values) -> List<sqlite.Row> or sqlite.Error` | exécute une instruction et donne toutes ses lignes. |
| `db.script(statements)` | exécute plusieurs instructions séparées par `;`, sans valeurs. Donne `Nothing or sqlite.Error`. |
| `db.last_id -> Int` | le numéro donné à la dernière ligne insérée. |
| `db.changes -> Int` | combien de lignes le dernier `INSERT`, `UPDATE` ou `DELETE` a touchées. |
| `db.close` | ferme la base. |
| `row.text(name) -> String?` | la colonne de ce nom. `none` si elle est absente, `NULL` ou d'un autre type. |
| `row.int(name) -> Int?` | la même chose pour un entier. |
| `row.float(name) -> Float?` | la même chose pour un nombre ; un entier se lit aussi comme un `Float`. |
| `row.bytes(name) -> List<Int>?` | la même chose pour un `BLOB`. |
| `row.get(name) -> sqlite.Value?` | la colonne telle quelle, quel que soit son type. |

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

## Ne construis jamais une instruction avec un texte reçu

Chaque `?` d'une instruction est rempli depuis `with`, dans l'ordre. Une `sqlite.Value` est `.Null`,
`.Int(n)`, `.Float(x)`, `.Text(s)` ou `.Bytes(data)`.

Écrire la valeur directement dans l'instruction paraît plus simple, et c'est la faille classique des
bases de données. Avec `"… WHERE name = '{name}'"`, un visiteur qui tape `' OR '1'='1` change le sens
de l'instruction et lit toutes les lignes. Une valeur passée par `?` reste toujours une valeur.

## Plusieurs changements d'un coup

Une transaction fait compter plusieurs changements pour un seul : tous ont lieu, ou aucun. Elle rend
aussi les insertions nombreuses bien plus rapides, puisque le fichier n'est écrit qu'une fois :

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

Si une étape échoue, lance `db.script("ROLLBACK")` pour annuler ce qui précède.

## Erreurs

`sqlite.Error` a trois cas, chacun avec le message de SQLite :

- `Constraint(message)` : une règle de la table a refusé le changement (`UNIQUE`, `NOT NULL`, une clé étrangère).
- `Busy(message)` : une autre connexion tient le fichier. Réessaie un peu plus tard.
- `Failed(message)` : tout le reste, comme une erreur dans l'instruction.

`run` et `query` n'exécutent que la première instruction et ignorent la suite : utilise `script` pour plusieurs.
