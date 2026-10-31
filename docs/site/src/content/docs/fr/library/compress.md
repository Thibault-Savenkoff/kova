---
title: compress et zip
description: Compresser des octets avec gzip, lire et écrire des archives zip.
sidebar:
  order: 21
---

```kova
import compress
import zip
```

Ces modules travaillent sur des [octets](/fr/library/list/), un `List<Int>`. Vers un fichier,
`files.read_bytes` et `files.write_bytes` ; un texte devient des octets avec `s.to_bytes`.

```kova
import compress
import zip

fn main {
    let data: List<Int> := "les mêmes mots encore ".repeat(100).to_bytes
    let packed: List<Int> := compress.gzip(data)
    print(packed.length < data.length)
    print(compress.gunzip(packed) = data)

    let note: zip.Entry := zip.Entry { name := "docs/note.txt", data := "bonjour".to_bytes }
    let archive: List<Int> := zip.pack([note])
    for e in zip.unpack(archive) else [] {
        print("{e.name}: {e.data.to_text else "?"}")
    }
}
```

```text
true
true
docs/note.txt: bonjour
```

| Fonction | Ce qu'elle fait |
| --- | --- |
| `compress.gzip(data) -> List<Int>` | les octets compressés au format gzip, que `gzip` et tous les autres outils lisent. |
| `compress.gunzip(data) -> List<Int>?` | les octets d'origine ; `none` si ce n'est pas du gzip ou si c'est abîmé. |
| `zip.pack(entries: List<zip.Entry>) -> List<Int>` | une archive zip. |
| `zip.unpack(data) -> List<zip.Entry>?` | ses entrées ; `none` si ce n'est pas une archive ou si elle est abîmée. |

Un `zip.Entry` contient `name`, le chemin dans l'archive, et `data`.

Dessous, trois méthodes marchent sur n'importe quels octets :

| Méthode | Ce qu'elle donne |
| --- | --- |
| `data.deflate -> List<Int>` | les octets compressés avec DEFLATE, sans l'enveloppe de gzip. |
| `data.inflate -> List<Int>?` | les octets d'origine, `none` si ce n'est pas du DEFLATE. Elle s'arrête à 4 Go, donc un petit fichier ne peut pas remplir la mémoire. |
| `data.crc32 -> Int` | la somme de contrôle CRC-32. |

Kova compresse vite mais moins serré que `gzip -9` : compte des fichiers 10 à 20 % plus gros. Tout ce
que les autres outils compressent, Kova le lit.
