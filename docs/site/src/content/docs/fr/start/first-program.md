---
title: Ton premier programme
description: Écrire, lancer et compiler un programme Kova, et lire une erreur.
sidebar:
  order: 2
---

## Bonjour

Crée un fichier `hello.kova` :

```kova
fn main {
    print("Hello, Kova!")
}
```

Lance-le :

```sh
kova run hello.kova
```

```text
Hello, Kova!
```

`fn main` est l'endroit où chaque programme commence. Elle n'a pas de `()` : en Kova, une fonction
sans paramètres s'écrit et s'appelle sans parenthèses (tu croiseras `x.to_float` et `rex.sound` de
la même façon). `print` écrit une ligne.

## Lancer ou compiler

Il y a deux façons de démarrer un programme, et elles affichent toujours la même chose :

- `kova run hello.kova` le démarre tout de suite. Kova lit le programme, le vérifie, et l'exécute
  dans son propre interpréteur. C'est ce qu'on utilise en écrivant.
- `kova build hello.kova` crée un exécutable natif, `hello` (`hello.exe` sous Windows), à côté du
  fichier. C'est un petit fichier (environ 30 Ko ici) qui tourne seul, sur n'importe quelle
  machine de ce système, sans Kova installé :

```sh
kova build hello.kova
./hello
```

```console
✓ hello (Linux, 29 Ko) en 3 ms
Hello, Kova!
```

Ajoute `--target windows` (ou `linux`, `mac`…) pour créer l'exécutable d'un autre système, depuis
n'importe quel système. [La commande](/fr/tools/command/) les liste tous.

## Donner des mots au programme

Ce qui suit le nom du fichier sur la ligne de commande arrive au programme dans `system.args`, une liste
de textes :

```kova
import system

fn main {
    for word in system.args {
        print("Hello, {word}!")
    }
}
```

```sh
kova run hello.kova Ada Alan
```

```console
Hello, Ada!
Hello, Alan!
```

`{word}` dans un texte y met la valeur de `word`.

## Quand quelque chose ne va pas

Kova vérifie tout le programme avant d'en exécuter la moindre ligne. Une erreur est montrée avec
son endroit et, souvent, une façon d'en sortir :

```kova
fn main {
    let name: String := "Ada"
    print("Hello, " + name.length)
}
```

```txt
erreur: types différents : `String` attendu, `Int` donné
 --> main.kova:3:23
  |
3 |     print("Hello, " + name.length)
  |                       ^^^^^^^^^^^
  = aide : mettez la valeur dans le texte : `"total : {x}"`
```

`name.length` est un nombre, et `+` ne joint des textes qu'avec des textes : Kova ne transforme
jamais un type en un autre sans qu'on le lui demande. Ici, écris `"Hello, {name.length}"`, ou
`name.length.to_string`.

Dans l'éditeur, le même message apparaît sous le code pendant la frappe.

Un programme peut aussi s'arrêter pendant qu'il tourne : une position après la fin d'une liste, un
nombre trop grand pour un `Int`, un [contrat](/fr/guide/contracts-and-tests/) non respecté. Il dit
alors ce qui s'est passé et où, et se termine avec le code de sortie 101.

Suite : [un tour de Kova](/fr/start/tour/), ou directement le [guide du langage](/fr/guide/values/).
