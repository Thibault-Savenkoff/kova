---
title: toml
description: Lire TOML, le format des fichiers de configuration.
sidebar:
  order: 19
---

```kova
import toml
```

TOML est le format des fichiers de configuration : des lignes `nom = valeur`, des sections `[nom]`,
des listes de sections `[[nom]]`.

`toml.to_json(text) -> String or toml.Error` le transforme en JSON. Tu lis ensuite ce JSON dans une
struct de ton programme avec [`from_json`](/fr/library/json-and-csv/), qui vérifie chaque champ et
son type : il n'y a qu'une façon de lire des données, pour les deux formats.

```kova
import toml

struct Server {
public:
    host: String
    port: Int
}

struct Config {
public:
    name: String
    debug: Bool
    servers: List<Server>
}

fn main {
    let text: String := """
    name = "shop"
    debug = true

    [[servers]]
    host = "alpha"
    port = 8080
    """
    let json: String := toml.to_json(text)?
    let config: Config := Config.from_json(json)?
    print(config.servers[0].host)
    print(config.debug)
}
```

```text
alpha
true
```

Un `toml.Error` contient `line` et `message`, et `e.to_string` écrit les deux :

```kova
import toml

fn main {
    match toml.to_json("port = 80\nport = 81") {
        ok(json) => print(json)
        err(e) => print(e.to_string)
    }
}
```

```text
line 2: `port` is defined twice
```

Tout TOML est lu : toutes les sortes de textes, les noms à points (`a.b = 1`), les listes, les tables
`{ x = 1 }`, les dates et les nombres `1_000` et `0xff`. Une date reste un texte dans le JSON, qu'un
champ `Date` sait lire. `inf` et `nan` sont refusés, car JSON ne sait pas les écrire.

Kova lit TOML mais ne l'écrit pas : un programme qui enregistre des réglages utilise `to_json`.
