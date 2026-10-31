---
title: sound
description: Playing WAV sounds.
sidebar:
  order: 25
---

```kova
import sound
```

| Function | What it does |
| --- | --- |
| `sound.play(path: String) -> Nothing or sound.Error` | plays a WAV file, and comes back when it is over. |
| `sound.play_bytes(data: List<Int>) -> Nothing or sound.Error` | plays a WAV held in memory. |
| `sound.wav(samples: List<Float>) rate(r) -> List<Int>` | makes a WAV of one channel from its samples, each from `-1.0` to `1.0`; `rate` is how many per second, 44100 if left out. |

`sound.Error` is `NotFound(path)`, `NotSound` (not an uncompressed WAV: whole numbers of 8, 16, 24 or 32 bits, or 32-bit floats) or `NoDevice` (no sound card,
or no sound server on Linux). Only uncompressed WAV plays for now; MP3 and Ogg do not.

Kova uses what the system has: PulseAudio or PipeWire on Linux, `PlaySound` on Windows, `NSSound` on
a Mac. Nothing needs installing, except on a Linux without a sound server (package
`libpulse0` or `pulseaudio-libs`).

## Playing without stopping the program

`sound.play` waits for the end, so that a program can play two sounds one after the other just by
writing two lines. To go on working meanwhile, play in a thread of its own
([threads](/library/threads/)):

```kova
import sound
import threads

fn music {
    match sound.play("music.wav") {
        ok => {}
        err(e) => print("no music")
    }
}

fn main {
    let playing: threads.Task<Nothing> := threads.spawn { music }
    print("the program goes on")
    playing.wait
}
```

## Making a sound yourself

A sound is a long list of numbers: how far the speaker is pushed, 44100 times a second. A pure note
is a sine wave, and its frequency is how many times it goes up and down each second; 440 is the A
that orchestras tune to. Its height in the list, here `0.3`, is the volume:

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

Give it to `sound.play_bytes` to hear it. From there, everything is arithmetic on Lists: two lists
added number by number sound together, a list followed by another plays one after the other, and a
list multiplied by a number that falls towards 0 fades out. A WAV is a 44-byte header followed by
2 bytes per sample, which is the 44144 above.
