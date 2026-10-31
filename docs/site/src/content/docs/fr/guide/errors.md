---
title: Valeurs absentes et erreurs
description: T? pour une valeur qui peut manquer, T or E pour une opération qui peut échouer, et le ? qui les transmet.
sidebar:
  order: 9
---

Deux choses tournent mal dans tout programme : une valeur qui n'est pas là (pas de telle clé, un
texte qui n'est pas un nombre), et une opération qui échoue (un fichier absent, un site qui ne
répond pas). Beaucoup de langages laissent passer les deux sans rien dire, avec un `null` qui
plante loin de là où il est né, ou une exception que rien dans le code n'annonçait. En Kova, les
deux sont écrites dans le **type**, et le compilateur s'assure que chacune est traitée.

## `T?` : une valeur qui peut manquer

`Int?` est un `Int` ou `none`. `"42".to_int` donne un `Int?`, puisque le texte pourrait ne pas être
un nombre ; `ages["Zed"]` sur une map aussi :

```kova
fn main {
    let a: Int? := "42".to_int
    let b: Int? := "forty".to_int
    print(a)
    print(b)
}
```

```text
42
none
```

Un `Int?` ne peut pas servir d'`Int` tant que le programme ne l'a pas vérifié :

```kova
fn main {
    let a: Int? := "4".to_int
    print(a + 1)
}
```

```txt
erreur: `+` s'applique à `Int`, `Float`, `String` ou `BigInt`, pas à `Int?`
 --> main.kova:3:11
  |
3 |     print(a + 1)
  |           ^
```

Quatre façons de le vérifier :

```kova
fn main {
    let a: Int? := "4".to_int
    // 1. Dans `if a != none`, `a` est un `Int`.
    if a != none {
        print(a + 1)
    }
    // 2. `if n := …` nomme la valeur, pour ce qui n'est pas encore dans une variable.
    if n := "8".to_int {
        print(n + 1)
    }
    // 3. `match` nomme aussi la valeur, et dit quoi faire sans elle.
    match "x".to_int {
        none => print("not a number")
        n => print(n * 2)
    }
    // 4. Après être sorti quand elle manque, `a` est un `Int` jusqu'à la fin.
    if a = none {
        return
    }
    print(a * 10)
}
```

```text
5
9
not a number
40
```

`if n := e` prend un `else` comme tout `if`, et `while line := read_line { … }` continue tant qu'il y a
une valeur. Cela ne marche que sur `T?` : `n` est un nouveau nom, et `:=` ne peut pas changer une
variable ici.

C'est ce qu'on appelle le **rétrécissement** : après la vérification, le compilateur sait que la
valeur est là, et le type devient `Int`. Rien n'est déballé à la main, et rien ne peut être oublié.

Un champ, un paramètre ou un résultat qui peut manquer le dit de la même façon : `born: Int?`,
`-> String?`. `none` est la valeur absente, et un simple `Int` est accepté partout où un `Int?`
est attendu.

## `T or E` : une opération qui peut échouer

Une fonction qui peut échouer rend un `T or E` : un résultat de type `T`, ou une erreur de type
`E`. Elle donne `ok(valeur)` ou `err(erreur)` :

```kova
fn half(n: Int) -> Int or String {
    if n % 2 != 0 {
        return err("{n} is odd")
    }
    return ok(n / 2)
}

fn main {
    match half(10) {
        ok(h) => print(h)
        err(e) => print("error: {e}")
    }
    match half(7) {
        ok(h) => print(h)
        err(e) => print("error: {e}")
    }
}
```

```text
5
error: 7 is odd
```

L'erreur peut être de n'importe quel type : un texte, ou mieux, une enum qui liste ce qui peut
mal tourner, comme le fait la bibliothèque (`files.IoError` est `NotFound`, `Denied`, …).

Un résultat n'est jamais jeté en silence. Appeler une fonction qui peut échouer sans regarder son
résultat est refusé, puisque l'échec passerait inaperçu :

```kova
import files

fn main {
    files.write("note.txt", "hello")
}
```

```txt
erreur: ce résultat n'est pas regardé : un échec passerait inaperçu
 --> main.kova:4:5
  |
4 |     files.write("note.txt", "hello")
  |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  = aide : écris `files.write("note.txt", "hello")?` pour faire remonter l'échec, ou traite-le avec `match`
```

Une opération qui ne donne rien quand elle réussit rend un `Nothing or E`, et un `ok` seul :

```kova
fn validate(age: Int) -> Nothing or String {
    if age < 0 {
        return err("an age is never negative")
    }
    return ok
}

fn main {
    match validate(-3) {
        ok => print("fine")
        err(e) => print(e)
    }
}
```

```text
an age is never negative
```

## `?` : transmettre

La plupart du temps, une fonction qui rencontre une erreur ne peut rien en faire, sinon la donner
à son appelant. `?` après un appel fait exactement ça : si l'appel a échoué, la fonction rend
l'erreur tout de suite ; sinon, `?` donne la valeur.

```kova
fn half(n: Int) -> Int or String {
    if n % 2 != 0 {
        return err("{n} is odd")
    }
    return ok(n / 2)
}

fn quarter(n: Int) -> Int or String {
    let h: Int := half(n)?
    return half(h)
}

fn main {
    print(quarter(12))
    print(quarter(6))
}
```

```text
ok(3)
err("3 is odd")
```

`?` se voit : chaque endroit où une fonction peut s'arrêter tôt est marqué, contrairement à une
exception qui peut sortir de n'importe quel appel. Il marche aussi sur `T?`, dans une fonction qui
rend un `T?` : un `none` est alors rendu tout de suite.

Le résultat de la fonction elle-même doit le permettre, et le compilateur le dit quand ce n'est pas
le cas, comme dans une fonction qui ne rend rien :

```kova
import files

fn show {
    let text: String := files.read("note.txt")?
    print(text)
}

fn main {
    show
}
```

```txt
erreur: `?` rend l'erreur (`files.IoError`) à l'appelant : la fonction doit renvoyer `… or files.IoError` ; elle renvoie rien
 --> main.kova:4:25
  |
4 |     let text: String := files.read("note.txt")?
  |                         ^^^^^^^^^^^^^^^^^^^^^^^
  = aide : ou traite l'erreur toi-même : `match … { ok(v) => …, err(e) => … }`
```

`main` et les tests n'ont personne à qui rendre l'erreur : là, `?` arrête le programme et dit
pourquoi. Un petit script peut donc lire son fichier en une ligne :

```kova
import files

fn main {
    let text: String := files.read("note.txt")?
    print(text)
}
```

```txt
erreur: NotFound("note.txt")
  --> main.kova:4:25
```

## `else` : une valeur de secours

`else` après un optionnel ou un résultat donne la valeur qu'il contient, ou celle de droite s'il
n'y en a pas. Il dit en une ligne ce que fait le programme quand la valeur manque :

```kova
fn main {
    let ages: Map<String, Int> := ["Rex" := 3]
    print(ages["Rex"] else 0)
    print(ages["Fido"] else 0)
    print("12x".to_int else -1)
}
```

```text
3
0
-1
```

Le côté droit n'est calculé que s'il sert. `x` reste utilisable ensuite : `else` le lit et copie la
valeur.

### Tes propres erreurs

Un programme a en général sa propre enum d'erreurs. Quand un de ses cas contient l'erreur
rencontrée, `?` l'y met tout seul :

```kova
import files

enum AppError {
    Empty
    Io(error: files.IoError)
}

fn load(path: String) -> String or AppError {
    let text: String := files.read(path)?
    if text = "" {
        return err(AppError.Empty)
    }
    return ok(text)
}

fn main {
    match load("missing.txt") {
        ok(text) => print(text)
        err(e) => print(e)
    }
}
```

```text
AppError.Io(NotFound("missing.txt"))
```

## Quand c'est le programme qui se trompe

`T?` et `T or E` sont pour ce qui peut arriver à un programme correct : un utilisateur qui tape un
mot au lieu d'un nombre, un fichier qui a été effacé. Une erreur dans le programme lui-même (une
position après la fin d'une liste, une division par zéro, un [contrat](/fr/guide/contracts-and-tests/)
non respecté) n'est pas quelque chose à traiter : le programme s'arrête tout de suite, dit quoi et
où, et se termine avec le code de sortie 101. Pour s'arrêter exprès, `system.exit(code)` termine le
programme avec ce code.
