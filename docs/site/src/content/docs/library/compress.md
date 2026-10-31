---
title: compress and zip
description: Packing bytes with gzip, reading and writing zip archives.
sidebar:
  order: 21
---

```kova
import compress
import zip
```

These work on [bytes](/library/list/), a `List<Int>`. To a file, `files.read_bytes` and
`files.write_bytes`; a text becomes bytes with `s.to_bytes`.

```kova
import compress
import zip

fn main {
    let data: List<Int> := "the same words again ".repeat(100).to_bytes
    let packed: List<Int> := compress.gzip(data)
    print(packed.length < data.length)
    print(compress.gunzip(packed) = data)

    let note: zip.Entry := zip.Entry { name := "docs/note.txt", data := "hello".to_bytes }
    let archive: List<Int> := zip.pack([note])
    for e in zip.unpack(archive) else [] {
        print("{e.name}: {e.data.to_text else "?"}")
    }
}
```

```text
true
true
docs/note.txt: hello
```

| Function | What it does |
| --- | --- |
| `compress.gzip(data) -> List<Int>` | the bytes packed in the gzip format, which `gzip` and every other tool read. |
| `compress.gunzip(data) -> List<Int>?` | the bytes back; `none` if it is not gzip data or it was damaged. |
| `zip.pack(entries: List<zip.Entry>) -> List<Int>` | a zip archive. |
| `zip.unpack(data) -> List<zip.Entry>?` | its entries; `none` if it is not an archive or it was damaged. |

A `zip.Entry` holds `name`, the path inside the archive, and `data`.

Under them, three methods work on any bytes:

| Method | What it gives |
| --- | --- |
| `data.deflate -> List<Int>` | the bytes packed with DEFLATE, without gzip's envelope. |
| `data.inflate -> List<Int>?` | the bytes back, `none` if they are not DEFLATE data. It stops at 4 GB, so a tiny file cannot fill the memory. |
| `data.crc32 -> Int` | the CRC-32 checksum. |

Kova packs about as tightly as `gzip -9`. Everything other tools pack, Kova reads.
