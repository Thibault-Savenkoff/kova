---
title: files
description: Lire, écrire, copier et trouver des fichiers et des dossiers.
sidebar:
  order: 8
---

```kova
import files
```

Tout ce qui touche au disque peut échouer : le fichier manque, le dossier est interdit, le disque
est plein. Les fonctions de `files` donnent donc un `T or files.IoError`, et Kova s'assure que le
programme le regarde ([Valeurs manquantes et erreurs](/fr/guide/errors/)).

```kova
import files

fn main {
    let saved: Nothing or files.IoError := files.write("note.txt", "hello\n")
    match files.read("note.txt") {
        ok(text) => write(text)
        err(e) => print("cannot read: {e}")
    }
    print(files.read("missing.txt"))
    let removed: Nothing or files.IoError := files.remove("note.txt")
}
```

```text
hello
err(NotFound("missing.txt"))
```

Les chemins s'écrivent avec `/` sur chaque système (Windows les accepte aussi), et sont relatifs au
dossier d'où le programme est lancé, sauf s'ils partent de la racine.

## Lire et écrire

| Fonction | Fait |
| --- | --- |
| `files.read(path) -> String or IoError` | tout le texte d'un fichier (`NotText` s'il n'est pas du texte UTF-8). |
| `files.write(path, text) -> Nothing or IoError` | écrit `text` dans le fichier, en remplaçant ce qui y était, et le crée si besoin. |
| `files.append(path, text) -> Nothing or IoError` | ajoute `text` à la fin du fichier, et le crée si besoin. |
| `files.sha256(path) -> String or IoError` | l'empreinte SHA-256 du fichier, n'importe quel fichier, texte ou non. |

`files.lines(path) -> List<String> or IoError` donne ses lignes ; pour un tableau, voir
[CSV](/fr/library/json-and-csv/#csv).

## Fichiers binaires

Un fichier est une suite d'octets. Kova te les donne dans une simple `List<Int>` de nombres de 0 à
255, donc tout ce que font les listes marche dessus : `data[0]`, `data[4..8]`, `data.length`, `+`,
`=`, `for`. Une List prend un octet par valeur tant que ses valeurs restent entre 0 et 255 : un
fichier de 100 Mo prend donc 100 Mo en mémoire.

| Fonction | Fait |
| --- | --- |
| `files.read_bytes(path) -> List<Int> or IoError` | tous les octets d'un fichier, n'importe quel fichier. |
| `files.write_bytes(path, data: List<Int>) -> Nothing or IoError` | écrit `data`, en remplaçant ce qui y était, et crée le fichier si besoin. |
| `files.append_bytes(path, data: List<Int>) -> Nothing or IoError` | ajoute `data` à la fin du fichier, et le crée si besoin. |

Pour lire un format, cherche où se trouve chaque valeur, puis reconstruis le nombre à partir de ses
octets. Un nombre rangé sur deux octets, le grand d'abord, vaut `first * 256 + second`, c'est-à-dire
`(first << 8) | second` : chaque octet monte de 8 bits pour laisser la place au suivant. Ici un
petit en-tête contient un genre à la place 0 et une taille sur deux octets aux places 1 et 2 :

```kova
import files

fn main {
    files.write_bytes("header.bin", [7, 1, 44])?
    let data: List<Int> := files.read_bytes("header.bin")?
    print(data[0])
    print((data[1] << 8) | data[2])
    files.remove("header.bin")?
}
```

```text
7
300
```

Dans l'autre sens, `>>` et `&` découpent un nombre en octets : `n >> 8` est le grand octet de `n`,
et `n & 255` le petit. Un nombre hors de 0..255 dans une liste donnée comme octets arrête le
programme avec `un octet va de 0 à 255` : `write_bytes(path, [n >> 8, n & 255])` est donc sûr, et
`write_bytes(path, [n])` te prévient quand `n` ne tient pas.

Octets et textes se changent l'un en l'autre avec `s.to_bytes` et `data.to_text`
([List](/fr/library/list/#octets)).

## Fichiers et dossiers

| Fonction | Fait |
| --- | --- |
| `files.exists(path) -> Bool` | s'il y a un fichier ou un dossier à cet endroit. |
| `files.is_folder(path) -> Bool` | si c'est un dossier. |
| `files.size(path) -> Int or IoError` | la taille en octets. |
| `files.modified(path) -> DateTime or IoError` | quand il a changé pour la dernière fois, en heure locale. |
| `files.list(folder) -> List<String> or IoError` | les noms dans un dossier, triés. |
| `files.matching(pattern) -> List<String> or IoError` | les chemins qui vont avec un motif, triés (ci-dessous). |
| `files.make_folder(path) -> Nothing or IoError` | crée un dossier, et les dossiers au-dessus si besoin. |
| `files.remove(path) -> Nothing or IoError` | supprime un fichier. |
| `files.copy(path) to(destination) -> Nothing or IoError` | copie un fichier, en remplaçant `destination` s'il existe. |
| `files.move(path) to(destination) -> Nothing or IoError` | déplace ou renomme un fichier, en remplaçant `destination` s'il existe. |

```kova
import files

fn main {
    let made: Nothing or files.IoError := files.write("a.txt", "hello")
    let copied: Nothing or files.IoError := files.copy("a.txt") to("b.txt")
    print(files.size("b.txt"))
    let moved: Nothing or files.IoError := files.move("b.txt") to("c.txt")
    print(files.matching("*.txt"))
    for f in ["a.txt", "c.txt"] {
        let gone: Nothing or files.IoError := files.remove(f)
    }
}
```

```text
ok(5)
ok(["a.txt", "c.txt"])
```

### Les motifs

Dans `files.matching`, `*` remplace n'importe quels caractères d'un nom, `?` un caractère, et `**`
n'importe quel nombre de dossiers : `"photos/**/*.jpg"` trouve chaque `.jpg` sous `photos`, à
n'importe quelle profondeur.

## `files.IoError`

Chaque cas contient le chemin concerné :

| Cas | Veut dire |
| --- | --- |
| `NotFound(path)` | il n'y a pas de tel fichier ou dossier |
| `Denied(path)` | le système ne le permet pas |
| `IsFolder(path)` | un dossier là où un fichier était attendu |
| `NotText(path)` | le fichier n'est pas du texte UTF-8 (`files.read`) |
| `Other(path)` | tout le reste (un disque plein…) |

`IoError` suit `Display`, donc `"{e}"` écrit `NotFound("missing.txt")`.

`import files as fs` donne un autre nom au module : `fs.read(…)`.
