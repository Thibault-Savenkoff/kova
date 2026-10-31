---
title: Plusieurs choses à la fois
description: workers(n) répartit le travail entre des fils d'exécution, sans risque.
sidebar:
  order: 15
---

Un programme attend beaucoup : qu'un site réponde, qu'un disque lise, qu'un long calcul finisse.
Quand des morceaux de travail ne dépendent pas les uns des autres, les faire **en même temps**, sur
plusieurs fils d'exécution, rend le tout bien plus rapide. Kova l'offre en un mot, `workers(n)`,
aux trois endroits où c'est sûr et utile.

## Sur une liste

`l.map workers(n) { … }` donne la même liste que `l.map { … }`, mais répartit les éléments entre
`n` fils :

```kova
import system

fn slow_square(n: Int) -> Int {
    system.sleep(0.2)
    return n * n
}

fn main {
    let numbers: List<Int> := [1, 2, 3, 4, 5, 6, 7, 8]
    let start: Float := system.clock
    let squares: List<Int> := numbers.map workers(8) { n: Int => slow_square(n) }
    print(squares)
    print("{system.clock - start} seconds")
}
```

Compilé, ce programme prend 0,2 seconde au lieu de 1,6 : les huit attentes se font ensemble. Les
résultats restent dans l'ordre de la liste, quel que soit l'ordre dans lequel les fils finissent.

`l.each workers(n) { … }` exécute le bloc sur chaque élément de la même façon, pour ses effets
(écrire des fichiers, interroger des sites). Ses éléments sont pris dans un ordre qui change d'une
exécution à l'autre, donc deux `print` peuvent sortir inversés.

## Dans un serveur

`web.serve(8080) workers(8) { q: web.Request => … }` répond à jusqu'à 8 questions en même temps,
pour qu'une réponse lente ne fasse attendre personne d'autre ([le serveur web](/fr/library/web-server/)).

## Pourquoi c'est sûr

Des fils qui changent la même variable en même temps sont la source des pires bugs qui existent :
des résultats faux une fois sur mille, jamais sur la machine du développeur. Kova les exclut avec
une règle : **un bloc `workers` peut lire les variables autour de lui, jamais les changer.**

```kova
fn main {
    var count: Int := 0
    [1, 2, 3].each workers(2) { x: Int =>
        count += x
    }
}
```

```txt
erreur: `count` ne peut pas changer dans `workers(…)`
 --> main.kova:4:9
  |
4 |         count += x
  |         ^^^^^
  = aide : plusieurs fils d'exécution passent dans ce bloc en même temps : chacun changerait sa propre copie
```

Chaque fil commence avec sa propre copie de ce que le bloc lit, donc il n'y a rien de partagé à
verrouiller, et rien à oublier de verrouiller. Pour combiner des résultats, laisse `map` les
rassembler, puis travaille sur la liste qu'il donne (`squares.fold(0) { … }`).

## À savoir

- `kova run` prend un élément, ou une question, à la fois : seul le programme compilé utilise des
  fils. Les résultats sont les mêmes dans les deux cas.
- `workers(1)` travaille un à la fois ; `workers(0)` arrête le programme, puisque rien ne tournerait.
- `random` tire différemment dans chaque fil, donc un `random` dans le bloc donne d'autres nombres
  qu'un à la fois.
- Kova n'a pas d'`async`/`await` : un appel bloquant attend simplement dans son propre fil, et le
  code se lit dans l'ordre, comme il s'exécute.
