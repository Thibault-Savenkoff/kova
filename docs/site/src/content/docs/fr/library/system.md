---
title: system, args et le terminal
description: Les arguments du programme, son entrée et sa sortie, les variables d'environnement, lancer d'autres programmes, les codes de sortie.
sidebar:
  order: 9
---

## Toujours là

Celles-ci n'ont besoin d'aucun import :

| Fonction | Fait |
| --- | --- |
| `print(value)` | écrit la valeur et un saut de ligne sur la sortie. N'importe quelle valeur : `print(rex)` montre ses champs. |
| `write(value)` | pareil, sans le saut de ligne. |
| `read_line -> String?` | la ligne suivante de l'entrée, sans son saut de ligne ; `none` à la fin de l'entrée. |
| `args -> List<String>` | les mots donnés après le nom du programme : `kova run app.kova a b`, `./app a b`. |
| `exit(code: Int)` | termine le programme tout de suite avec ce code de sortie (0 veut dire réussite). |

```kova
fn main {
    var total: Int := 0
    while true {
        let line: String? := read_line
        if line = none {
            break
        }
        match line.to_int {
            none => print("not a number: {line}")
            n => total += n
        }
    }
    print("total: {total}")
}
```

```sh
printf '4\n5\nsix\n' | kova run sum.kova
```

```console
not a number: six
total: 9
```

Un programme qui s'arrête sur une erreur (un contrat rompu, un indice hors de la liste) l'écrit sur
la sortie d'erreur et se termine avec le code 101.

Pour des couleurs dans le terminal, voir [`String`](/fr/library/string/#les-couleurs) ; pour des
lignes datées sur la sortie d'erreur, [`log`](/fr/library/log/).

## `import system`

```kova
import system
```

| Fonction | Donne |
| --- | --- |
| `system.env(name: String) -> String?` | la valeur d'une variable d'environnement, `none` si elle n'est pas définie. |
| `system.run(program: String) -> system.Finished or system.RunError` | lance un programme sans arguments, et l'attend. |
| `system.run(program) args(words: List<String>) -> system.Finished or system.RunError` | pareil avec des arguments. |

```kova
import system

fn main {
    print(system.env("KOVA_NO_SUCH_NAME"))
    match system.run("kova-no-such-program") args(["--version"]) {
        ok(done) => print("{done.output} (code {done.code})")
        err(e) => print(e)
    }
}
```

```text
none
NotFound("kova-no-such-program")
```

- Le programme est cherché dans les dossiers de `PATH` quand son nom ne contient pas de dossier.
- Son entrée est vide ; `system.Finished` contient `output` (ce qu'il a écrit), `errors` (ce qu'il
  a écrit sur la sortie d'erreur) et `code` (son code de sortie, 128 + le numéro du signal si un
  signal l'a arrêté).
- Un programme qui a tourné et échoué reste `ok` : regarde `code`. `err` veut dire qu'il n'a pas
  pu démarrer : `NotFound(program)`, `Denied(program)` ou `Other(program)`.
- Les mots vont au programme tels quels. Aucun shell ne les lit, donc une espace, un `;` ou un `$`
  dedans n'est qu'un caractère : rien de ce qu'écrit un utilisateur ne peut devenir une autre
  commande.
