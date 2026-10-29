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

Pour lire un fichier ligne par ligne, `files.read(path)?.split("\n")` ; pour un tableau, voir
[CSV](/fr/library/json-and-csv/#csv).

## Fichiers et dossiers

| Fonction | Fait |
| --- | --- |
| `files.exists(path) -> Bool` | s'il y a un fichier ou un dossier à cet endroit. |
| `files.is_folder(path) -> Bool` | si c'est un dossier. |
| `files.size(path) -> Int or IoError` | la taille en octets. |
| `files.modified(path) -> DateTime or IoError` | quand il a changé pour la dernière fois, en heure locale. |
| `files.list(folder) -> List<String> or IoError` | les noms dans un dossier, triés. |
| `files.matching(pattern) -> List<String> or IoError` | les chemins qui vont avec un motif, triés (ci-dessous). |
| `files.make_dir(path) -> Nothing or IoError` | crée un dossier, et les dossiers au-dessus si besoin. |
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
| `IsDirectory(path)` | un dossier là où un fichier était attendu |
| `NotText(path)` | le fichier n'est pas du texte UTF-8 (`files.read`) |
| `Other(path)` | tout le reste (un disque plein…) |

`IoError` suit `Display`, donc `"{e}"` écrit `NotFound("missing.txt")`.

`import files as fs` donne un autre nom au module : `fs.read(…)`.
