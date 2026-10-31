---
title: threads
description: Des fils, des canaux et des valeurs partagées entre fils.
sidebar:
  order: 15
---

```kova
import threads
```

[Plusieurs choses à la fois](/fr/guide/concurrency/) explique comment les utiliser ensemble. Un
programme qui utilise `threads` tourne en code natif, dans `kova run` et `kova test` aussi.

## threads.spawn

| Fonction | Fait |
| --- | --- |
| `threads.spawn { … }` | lance le bloc sur un nouveau fil, donne une `threads.Task<T>`, `T` étant la valeur du bloc |
| `threads.spawn(f)` | pareil avec une fonction sans paramètres |

Le bloc travaille sur ses propres copies de ce qu'il lit et ne peut pas changer ce qui l'entoure.

## threads.Task

| Méthode | Fait |
| --- | --- |
| `t.wait` | attend la fin du fil et donne sa valeur ; `t` est consommé |

## threads.Channel

| Écrit | Fait |
| --- | --- |
| `threads.Channel<T>.new` | un nouveau canal, sans limite |
| `threads.Channel<T>.limited(n)` | un nouveau canal qui garde `n` valeurs au plus (`n` vaut 1 ou plus) |
| `c.send(x)` | donne `x` au canal ; attend de la place dans un canal limité ; erreur sur un canal fermé |
| `c.receive` | attend et prend la valeur la plus ancienne : un `T?`, `none` une fois fermé et vide |
| `c.receive_within(secondes)` | pareil, pendant ce temps au plus : `none` aussi si rien n'est arrivé à temps |
| `c.close` | plus de valeurs : celles qui sont dedans restent à recevoir |
| `for x in c { … }` | reçoit jusqu'à ce que le canal soit fermé et vide ; chaque `x` appartient à la boucle |

Une copie d'un canal (`c.clone`, ou son usage dans un bloc de `spawn`) est le même canal.

## threads.select

```kova
threads.select {
    x from a => print(x)
    s from b => print(s)
    after(1.5) => print("trop long")
    closed => break
}
```

Attend la première valeur de plusieurs canaux. `after(secondes)` est facultatif ; `closed`, qui
s'exécute quand tous les canaux sont fermés et vides, est obligatoire. Voir
[Attendre plusieurs canaux](/fr/guide/concurrency/#attendre-plusieurs-canaux--threadsselect).

## threads.Shared

| Écrit | Fait |
| --- | --- |
| `threads.Shared<T>.new(valeur)` | une valeur que plusieurs fils partagent |
| `s.read` | une copie de la valeur |
| `s.change { x: var T => … }` | prête la valeur au bloc, un fil à la fois ; donne la valeur du bloc, s'il en a une |

Une copie d'un `Shared` est la même valeur. Un `change` dans un `change` de la même valeur s'attend
lui-même : Kova arrête le programme si aucun autre fil ne peut avancer.
