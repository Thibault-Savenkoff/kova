---
title: Appeler du C
description: Utiliser une bibliothèque du système (SQLite, la bibliothèque C, Win32…) avec extern.
sidebar:
  order: 16
---

La plupart de ce qu'offre un système (des bases de données comme SQLite, le graphisme, les
fonctions du système lui-même) est une bibliothèque écrite en C. `extern` déclare une fois les
fonctions d'une telle bibliothèque, avec des types Kova, et le programme les appelle ensuite comme
les fonctions d'un module :

```kova
extern c from "c" {
    fn abs(n: Int32) -> Int32
    fn length(text: String) -> Int from "strlen"
    fn getenv(name: String) -> String?
}

fn main {
    print(c.abs(-42))
    print(c.length("héllo"))
    print(c.getenv("KOVA_SURELY_NOT_SET"))
}
```

```text
42
6
none
```

- `extern c from "c"` nomme le bloc `c`, pour la bibliothèque `"c"`.
- Chaque `fn` donne une signature Kova à une fonction C. `from "strlen"` donne son nom C quand le
  nom Kova est différent.
- Aucun compilateur C n'intervient, et aucun fichier d'en-tête n'est lu : la déclaration est tout
  le contrat.

## Trouver la bibliothèque

Une bibliothèque se nomme sans son préfixe ni son suffixe : `"sqlite3"` est `libsqlite3.so` sous
Linux, `libsqlite3.dylib` sous macOS et `sqlite3.dll` sous Windows. `"c"` et `"m"` sont la
bibliothèque C du système. Elle est cherchée au premier appel, donc un programme qui ne l'appelle
jamais tourne sans elle.

Quand un système nomme autrement une bibliothèque ou une fonction, dis-le après son nom :
`extern sqlite from "sqlite3" windows "winsqlite3"`, `fn gmtime(…) from "gmtime" windows "_gmtime64"`.
Les systèmes sont `linux`, `mac` et `windows` ; chaque compilation prend ses propres noms.

## Là où va l'attention

Le C n'a pas de filet. Une fonction déclarée avec de mauvais types, ou appelée contre ses règles
(un pointeur libéré deux fois), peut faire planter le programme d'une façon que Kova ne peut pas
voir. Le danger est donc rassemblé à un seul endroit : le bloc `extern`. Vérifie chaque
déclaration avec la documentation de la bibliothèque ; les appels ressemblent ensuite à tous les
autres, et se comportent pareil.

Ne déclare jamais une fonction qui prend un nombre variable d'arguments (`printf`) : sur certains
processeurs, ses arguments iraient aux mauvais endroits.

## Ce qui passe

| Kova | C | Remarques |
| --- | --- | --- |
| `Int` | `int64_t`, `long` (Linux, macOS), `size_t` | |
| `Int32` | `int` | un `Int` trop grand pour lui arrête le programme |
| `Float` | `double` | |
| `Bool` | `int` | pas le `bool` de C99 |
| `String` | `const char*` | le C reçoit une copie terminée par un octet 0 ; un résultat `String` est copié aussi |
| `WideString` | `const wchar_t*` (l'UTF-16 de Windows) | le programme donne et reçoit toujours des `String` |
| `Pointer` | n'importe quel pointeur | une adresse que le C donne et reprend ; Kova ne regarde jamais dedans |
| `String?`, `Pointer?` | un pointeur qui peut être `NULL` | `none` est `NULL` |

`Int32` n'existe que dans les déclarations `extern` : le programme voit un `Int`.

## Le C qui écrit dans une variable : `var`

Certaines fonctions donnent leur résultat par un pointeur, comme `sqlite3_open` avec la base
qu'elle ouvre. Un paramètre `var` donne au C l'adresse d'une variable ; après l'appel, la variable
contient ce que le C a écrit :

```kova
extern sqlite from "sqlite3" {
    fn open(path: String, db: var Pointer?) -> Int32 from "sqlite3_open"
    fn close(db: Pointer) -> Int32 from "sqlite3_close"
}

fn main {
    var db: Pointer? := none
    print(sqlite.open("notes.db", var db))
    if db = none {
        return
    }
    print(sqlite.close(db))
}
```

Après `if db = none { return }`, `db` est un simple `Pointer` jusqu'à ce qu'une ligne le change.

## Le C qui rappelle le programme

Certaines fonctions C appellent une fonction du programme : `qsort` pour comparer deux éléments,
un système de fenêtres quand on clique sur un bouton. Le paramètre a un type de
[bloc](/fr/guide/blocks/), et l'appel donne une fonction par son nom, ou un bloc qui n'utilise rien
autour de lui (le C peut l'appeler après la fin de l'appel) :

```kova
extern c from "c" {
    fn calloc(count: Int, size: Int) -> Pointer
    fn free(p: Pointer)
    fn strcpy(into: Pointer, text: String) -> Pointer
    // `strstr(p, "")` est `p` lui-même : ici, une façon de lire le texte en `p`.
    fn text(p: Pointer, nothing: String) -> String from "strstr"
    fn strncmp(a: Pointer, b: Pointer, n: Int) -> Int32
    fn qsort(base: Pointer, count: Int, size: Int, compare: {Pointer, Pointer -> Int32})
}

fn letters(a: Pointer, b: Pointer) -> Int {
    return c.strncmp(a, b, 1)
}

fn main {
    let p: Pointer := c.calloc(16, 1)
    c.strcpy(p, "kova")
    c.qsort(p, 4, 1, letters)
    print(c.text(p, ""))
    c.qsort(p, 4, 1) { a: Pointer, b: Pointer => c.strncmp(b, a, 1) }
    print(c.text(p, ""))
    c.free(p)
}
```

```text
akov
voka
```

La fonction voit des types Kova (un `Int32` est un `Int`). Ce dont elle a besoin du programme passe
par le `Pointer` que la plupart des bibliothèques C renvoient pour ça (le `data` de GTK, le
`lParam` de Win32).

## Les structs C

Une struct C se déclare dans le bloc `extern`, avec ses champs dans l'ordre du C, puis s'utilise
comme n'importe quelle struct : `c.Time { … }`, `t.hour`. Kova la dispose en mémoire exactement
comme le C.

```kova
extern c from "c" {
    struct Time {
        second: Int32
        minute: Int32
        hour: Int32
        day: Int32
        month: Int32
        year: Int32
        weekday: Int32
        yearday: Int32
        summer: Int32
        // Linux et macOS ajoutent ces deux-là ; le `tm` de Windows s'arrête avant et les ignore.
        offset: Int
        zone: Pointer?
    }
    fn calloc(count: Int, size: Int) -> Pointer
    fn free(p: Pointer)
    fn strftime(into: Pointer, size: Int, format: String, time: Time) -> Int
    fn text(p: Pointer, nothing: String) -> String from "strstr"
}

fn main {
    let t := c.Time { second := 0, minute := 30, hour := 9, day := 1, month := 0, year := 126, weekday := 0, yearday := 0, summer := 0, offset := 0, zone := none }
    let p: Pointer := c.calloc(64, 1)
    c.strftime(p, 64, "%d/%m/%Y %H:%M", t)
    print(c.text(p, ""))
    c.free(p)
}
```

```text
01/01/2026 09:30
```

Comment une struct passe :

| Déclaré | Le C voit |
| --- | --- |
| `t: Time` | `const struct tm*` : le C la lit seulement |
| `t: var Time` | `struct tm*` : le C la remplit, et la variable est relue après l'appel |
| `t: copy Time` | `struct tm` : la struct elle-même, par valeur (le `CGRect` d'Apple, le `div_t` du C) |
| `-> Time?` | un résultat `struct tm*`, copié (`none` pour `NULL`), comme le donne `localtime` |
| `-> copy Time` | un résultat `struct tm`, par valeur |

Les champs peuvent être des nombres, `Bool`, `Pointer`, `Pointer?`, des structs déclarées avant,
des textes (`String`, `WideString` et leurs `?`, prêtés au C pour l'appel) et des fonctions que le
C appelle (un type de bloc, auquel on donne une fonction par son nom, comme `WNDCLASSEXW` de
Windows veut son `lpfnWndProc`). Une struct qui contient un texte ne peut pas être donnée en `var`,
puisque le C pourrait changer un texte qui lui était seulement prêté.

## L'UTF-16 de Windows

Les fonctions `…W` de Windows prennent des textes UTF-16. `WideString` en déclare un, et le
programme continue d'écrire des textes ordinaires :

```kova
extern win from "user32" {
    fn message(window: Pointer?, text: WideString, title: WideString, kind: Int32) -> Int32 from "MessageBoxW"
}

fn main {
    win.message(none, "Bonjour !", "Kova", 0)
}
```

## L'exécution

Un programme avec `extern` tourne en code natif, même sous `kova run` et `kova test` : l'interpréteur
ne peut pas appeler du C. Le débogueur s'arrête au premier appel vers le C.

Pour les fenêtres, [`import ui`](/fr/guide/windows/) fait tout cela pour toi, sur chaque système.
