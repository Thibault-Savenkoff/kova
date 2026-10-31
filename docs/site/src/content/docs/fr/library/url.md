---
title: url
description: Écrire des textes dans des adresses web et les relire.
sidebar:
  order: 17
---

```kova
import url
```

Une adresse web ne peut contenir que quelques caractères : tout le reste s'écrit avec `%` et deux
chiffres hexadécimaux. Une espace devient `%20`. Encode ce que tu mets dans une adresse, et elle ne
pourra pas casser.

| Fonction | Ce qu'elle fait |
| --- | --- |
| `url.encode(text) -> String` | le texte, sans danger dans une adresse. Seuls les lettres, les chiffres et `-_.~` restent tels quels. |
| `url.decode(text) -> String?` | le texte d'origine ; `+` est une espace, comme l'écrivent les formulaires. `none` si l'encodage est faux. |
| `url.query(values: Map<String, String>) -> String` | `nom=valeur&…`, trié par nom, chaque partie encodée. |
| `url.parse_query(text) -> Map<String, String>` | les noms et valeurs d'un tel texte. Une partie illisible est laissée de côté. |

```kova
import url

fn main {
    let search: String := url.query(["q" := "kova & co", "page" := "2"])
    print("https://example.com/search?{search}")
    print(url.parse_query(search)["q"] else "?")
    print(url.decode("caf%C3%A9") else "?")
    print(url.decode("100%"))
}
```

```text
https://example.com/search?page=2&q=kova%20%26%20co
kova & co
café
none
```
