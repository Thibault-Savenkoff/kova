---
title: arguments
description: Lire la ligne de commande d'un programme.
sidebar:
  order: 18
---

```kova
import arguments
```

`system.args` donne les mots écrits après le nom du programme. `arguments.read` les trie, donc
`outil --port 8080 --verbose photos` ne demande pas de boucle à toi. Donne-lui `system.args` ;
l'exemple ci-dessous utilise une liste écrite pour pouvoir montrer son résultat.

```kova
import arguments

fn main {
    let given: List<String> := ["photos", "--port", "8080", "--verbose", "--name=Ada"]
    let a: arguments.Arguments := arguments.read(given) options(["port"])
    print(a.words)
    print(a.flag("verbose"))
    print(a.option("port") else "80")
    print(a.option("name") else "personne")
}
```

```text
["photos"]
true
8080
Ada
```

`options([…])` nomme les options qui prennent une valeur. Cette partie est facultative, car une
valeur peut toujours s'écrire `--port=8080`.

| Écrit | Lu comme |
| --- | --- |
| `--port 8080`, `-p 8080` | une option, si `port` ou `p` est nommé dans `options([…])` |
| `--port=8080` | une option, toujours |
| `--verbose`, `-v` | un drapeau |
| `--` | tout ce qui suit est un mot, même s'il commence par `-` |
| tout le reste | un mot |

| Membre | Ce qu'il donne |
| --- | --- |
| `a.flag(name) -> Bool` | si ce drapeau a été écrit. |
| `a.option(name) -> String?` | la valeur de cette option, `none` si elle n'a pas été écrite. |
| `a.words -> List<String>` | les autres mots, dans l'ordre. |
