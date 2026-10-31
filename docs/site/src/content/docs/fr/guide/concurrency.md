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

## Tes propres fils

`workers` répartit un seul travail. Quand le travail a plusieurs parties différentes, ou qu'une
partie doit continuer pendant que le reste du programme fait autre chose, lance un fil toi-même avec
[`threads.spawn`](/fr/library/threads/). Il prend un bloc, le lance tout de suite sur un nouveau fil
et donne une `Task` : `wait` attend la fin du bloc et donne sa valeur.

```kova
import threads

fn slow_sum(n: Int) -> Int {
    var sum: Int := 0
    for i in 1..=n {
        sum += i
    }
    return sum
}

fn main {
    let first: threads.Task<Int> := threads.spawn { slow_sum(1000) }
    let second: threads.Task<Int> := threads.spawn { slow_sum(2000) }
    print(first.wait + second.wait)
}
```

```text
2501500
```

Le bloc travaille sur ses propres copies de ce qu'il lit, comme une [fonction gardée dans une
variable](/fr/guide/blocks/#garder-une-fonction), donc il ne peut pas changer une variable autour
de lui. Sa dernière ligne est sa valeur ; un bloc sans valeur donne une `threads.Task<Nothing>`, et
`wait` ne donne alors rien.

Un programme se termine quand tous ses fils sont terminés, même si personne n'a appelé `wait`.

## Passer des valeurs : les canaux

Un `threads.Channel<T>` est un tuyau entre des fils. `send` met une valeur à un bout et `receive`
sort la plus ancienne à l'autre bout, en attendant s'il n'y en a pas. Une boucle `for` reçoit jusqu'à
ce que le canal soit fermé.

Pour monter une équipe de travailleurs, lance-les d'abord, chacun en boucle sur le canal des tâches.
Envoie ensuite les tâches, puis appelle `close` pour dire qu'il n'y en aura plus. La boucle de chaque
travailleur se termine quand le canal est vide, et `wait` te dit qu'ils ont tous fini.

```kova
import threads

fn main {
    let jobs := threads.Channel<Int>.new
    let results := threads.Channel<Int>.new
    var team: List<threads.Task<Nothing>> := []
    for i in 0..3 {
        team.add(threads.spawn {
            for n in jobs {
                results.send(n * n)
            }
        })
    }
    for n in 1..=5 {
        jobs.send(n)
    }
    jobs.close
    for member in move team {
        member.wait
    }
    results.close

    var total: Int := 0
    for square in results {
        total += square
    }
    print(total)
}
```

```text
55
```

Trois choses font tenir l'ensemble :

- Une copie d'un canal est **le même tuyau**. C'est comme ça que le bloc ci-dessus l'atteint : sa
  copie de `jobs` et la tienne sont les deux bouts.
- `send` **donne** sa valeur, donc une valeur n'est jamais dans deux fils à la fois.
- `close` est le signal. Sans lui, les travailleurs attendraient pour toujours, et Kova arrêterait
  le programme en le disant (plus bas).

`threads.Channel<T>.new` garde autant de valeurs que tu en envoies. Si les envois peuvent prendre de
l'avance sur les réceptions, `threads.Channel<T>.limited(10)` en garde 10 au plus, et `send` attend
qu'il y ait de la place.

## Attendre plusieurs canaux : `threads.select`

`c.receive` attend un seul canal. Un fil qui en écoute plusieurs, ou qui ne doit pas attendre
indéfiniment, utilise `threads.select` : il prend la première valeur qui arrive, quel que soit le
canal, et exécute le cas correspondant.

```kova
import threads

fn main {
    let commandes := threads.Channel<Int>.new
    let notes := threads.Channel<String>.new
    commandes.send(12)
    notes.send("rappeler")
    notes.close
    commandes.close
    while true {
        threads.select {
            commande from commandes => print("commande {commande}")
            note from notes => print("note : {note}")
            after(5.0) => print("rien depuis cinq secondes")
            closed => break
        }
    }
}
```

```text
commande 12
note : rappeler
```

Chaque cas dit ce qui se passe :

- `nom from canal` : une valeur est arrivée, et `nom` la contient. Elle est à toi, comme avec
  `for x in c`. Les canaux peuvent contenir des types différents.
- `after(secondes)` : rien n'est arrivé pendant ce temps. Ce cas est facultatif.
- `closed` : tous les canaux sont fermés et vides, donc plus rien n'arrivera. Il est obligatoire,
  car une boucle autour de `select` doit bien finir : le plus souvent, tu écris `closed => break`.

Un canal fermé alors que les autres sont ouverts est simplement ignoré. Quand plusieurs canaux ont
une valeur, le premier écrit passe d'abord.

Pour attendre un seul canal pendant un temps limité, `c.receive_within(2.5)` est plus court. Il
donne `none` si rien n'est arrivé à temps, et aussi si le canal est fermé ; utilise `select` pour
distinguer les deux.

## Partager une valeur : `threads.Shared`

Quand des fils doivent tous voir une même valeur, un compteur ou un cache, mets-la dans un
`threads.Shared<T>`. Une copie est la même valeur, comme pour un canal. `change` prête la valeur à un
bloc, sans qu'aucun autre fil puisse y toucher avant la fin du bloc :

```kova
import threads

fn main {
    let count := threads.Shared<Int>.new(0)
    var team: List<threads.Task<Nothing>> := []
    for i in 0..4 {
        team.add(threads.spawn {
            for k in 0..1000 {
                count.change { n: var Int => n += 1 }
            }
        })
    }
    for member in move team {
        member.wait
    }
    print(count.read)
}
```

```text
4000
```

On écrit `n: var Int` parce que le bloc change ce qu'il emprunte. `change` donne la valeur du bloc,
s'il en a une, ce qui permet de lire un nouveau numéro. `read` donne une copie de la valeur.

Deux leçons des autres langages sont intégrées. Il n'y a pas de verrou à oublier : on n'atteint la
valeur que par `change` et `read`. Et le bloc ne peut pas garder la valeur, puisqu'elle est seulement
prêtée.

Préfère un canal quand un fil peut posséder le travail, et `Shared` pour ce qui est vraiment commun :
un `Shared` fait la queue entre les fils.

## Quand tous les fils attendent

Un fil qui attend une valeur que personne n'enverra bloquerait le programme en silence. Kova compte
les fils endormis sur un canal, une `Task` ou un `Shared`. Quand ils le sont tous, plus rien ne peut
les réveiller : le programme s'arrête avec une erreur à la ligne qui attendait.

```kova
import threads

fn main {
    let c := threads.Channel<Int>.new
    print(c.receive)
}
```

```txt
erreur: tous les threads attendent, et rien ne peut les réveiller
  --> main.kova:5:13
```

La cause habituelle est un canal jamais fermé. Un fil endormi dans `system.sleep` ou qui attend le
réseau n'est pas compté : il se réveillera tout seul.

## À savoir

- Un programme qui utilise `workers` ou `threads` tourne en code natif dans `kova run` et `kova test`
  aussi. `kova debug` suit les fils, un seul à la fois : un point d'arrêt dans un fil arrête tout le
  programme à cet endroit, et un pas reste dans ce fil. Avec `workers`, il prend un élément, ou une
  question, à la fois ; les résultats sont les mêmes.
- `workers(1)` travaille un à la fois ; `workers(0)` arrête le programme, puisque rien ne tournerait.
- `random` tire différemment dans chaque fil, donc un `random` dans le bloc donne d'autres nombres
  qu'un à la fois.
- Kova n'a pas d'`async`/`await` : un appel bloquant attend simplement dans son propre fil, et le
  code se lit dans l'ordre, comme il s'exécute.
