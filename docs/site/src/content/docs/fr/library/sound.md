---
title: sound
description: Jouer des sons WAV.
sidebar:
  order: 25
---

```kova
import sound
```

| Fonction | Ce qu'elle fait |
| --- | --- |
| `sound.play(path: String) -> Nothing or sound.Error` | joue un fichier WAV, et revient quand il est fini. |
| `sound.play_bytes(data: List<Int>) -> Nothing or sound.Error` | joue un WAV gardé en mémoire. |
| `sound.wav(samples: List<Float>) rate(r) -> List<Int>` | fabrique un WAV d'un canal à partir de ses échantillons, chacun de `-1.0` à `1.0` ; `rate` dit combien par seconde, 44100 s'il est omis. |

`sound.Error` vaut `NotFound(path)`, `NotSound` (pas un WAV non compressé) ou `NoDevice` (pas de carte
son, ou pas de serveur de son sous Linux). Seul le WAV non compressé se joue pour l'instant, pas le
MP3 ni l'Ogg.

Kova utilise ce que le système a : PulseAudio ou PipeWire sous Linux, `PlaySound` sous Windows,
`NSSound` sur Mac. Rien à installer, sauf sur un Linux sans serveur de son (paquet `libpulse0` ou
`pulseaudio-libs`).

## Jouer sans arrêter le programme

`sound.play` attend la fin : un programme joue ainsi deux sons à la suite rien qu'en écrivant deux
lignes. Pour continuer à travailler pendant ce temps, joue dans un fil à part
([threads](/fr/library/threads/)) :

```kova
import sound
import threads

fn music {
    match sound.play("music.wav") {
        ok => {}
        err(e) => print("pas de musique")
    }
}

fn main {
    let playing: threads.Task<Nothing> := threads.spawn { music }
    print("le programme continue")
    playing.wait
}
```

## Fabriquer un son toi-même

Un son est une longue liste de nombres : à quel point le haut-parleur est poussé, 44100 fois par
seconde. Une note pure est une sinusoïde, et sa fréquence dit combien de fois elle monte et descend
par seconde ; 440 est le la sur lequel s'accordent les orchestres. Sa hauteur dans la liste, ici
`0.3`, est le volume :

```kova
import sound

fn tone(hertz: Float, seconds: Float) -> List<Float> {
    var out: List<Float> := []
    for i in 0..(seconds * 44100.0).round {
        let time: Float := i.to_float / 44100.0
        out.add(0.3 * (2.0 * Float.pi * hertz * time).sin)
    }
    return out
}

fn main {
    let data: List<Int> := sound.wav(tone(440.0, 0.5))
    print(data.length)
}
```

```text
44144
```

Donne-le à `sound.play_bytes` pour l'entendre. À partir de là, tout est de l'arithmétique sur des
listes : deux listes additionnées nombre par nombre sonnent ensemble, une liste suivie d'une autre
joue l'une après l'autre, et une liste multipliée par un nombre qui tombe vers 0 s'éteint. Un WAV est
un en-tête de 44 octets suivi de 2 octets par échantillon, d'où le 44144 ci-dessus.
