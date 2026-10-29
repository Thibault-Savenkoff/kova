---
title: secure and chance
description: random for games and simulations, secure for passwords, tokens and encryption.
sidebar:
  order: 12
---

Kova has two kinds of chance, on purpose:

- `random` is fast and can be **repeated**: with the same `KOVA_SEED`, a program draws the same
  numbers, which makes games, simulations and tests reproducible. It must never protect anything:
  its numbers can be guessed.
- `secure` asks the system for chance that cannot be guessed, for passwords, keys and tokens.

## `random`

No import needed.

| Function | Gives |
| --- | --- |
| `random(a..b) -> Int` | a whole number from `a` up to, not including, `b`; `random(a..=b)` includes `b`. |
| `l.shuffled`, `l.chosen` | a list in random order, one element drawn ([List](/library/list/#order-and-chance)). |

```kova
fn main {
    let die: Int := random(1..=6)
    print(die >= 1 && die <= 6)
}
```

```text
true
```

`KOVA_SEED=42 kova run game.kova` draws the same numbers at every run, built or not.

## `import secure`

```kova
import secure
```

### Secure chance

| Function | Gives |
| --- | --- |
| `secure.random(a..b) -> Int` | like `random`, from the system's own chance. |
| `secure.token(bytes: Int) -> String` | that many bytes of chance, in hexadecimal (`token(16)` is 32 characters): a session key, a reset link. |
| `secure.uuid -> String` | a random UUID (version 4), `"3f2b…-…"`, 36 characters. |

### Passwords

A password is never stored, nor its SHA-256: a stolen list of fingerprints is broken in hours.
Store what `secure.password_hash` gives, and check a password against it:

```kova
import secure

fn main {
    let stored: String := secure.password_hash("correct horse")
    print(secure.password_matches("correct horse") hash(stored))
    print(secure.password_matches("wrong horse") hash(stored))
}
```

```text
true
false
```

| Function | Gives |
| --- | --- |
| `secure.password_hash(password) -> String` | an Argon2id hash to keep, with its own random salt. |
| `secure.password_hash(password) memory(mb) passes(n)` | the same with more cost (8 to 1024 MB, 1 to 16 passes). |
| `secure.password_matches(password) hash(stored) -> Bool` | whether `password` is the one `stored` was made from. |

Argon2id is slow and memory-hungry on purpose (19 MB and 2 passes by default), so each guess costs
an attacker the same. The text keeps its settings, so they can be raised later without making the
stored hashes invalid.

### Encryption

```kova
import secure

fn main {
    let key: String := secure.key
    let code: String := secure.encrypt("meet at noon") key(key)
    print(secure.decrypt(code) key(key))
    print(secure.decrypt(code) key(secure.key))
}
```

```text
"meet at noon"
none
```

| Function | Gives |
| --- | --- |
| `secure.key -> String` | a new random key, as text, to keep secret. |
| `secure.encrypt(text) key(k) -> String` | a code only that key opens (XChaCha20-Poly1305). |
| `secure.decrypt(code) key(k) -> String?` | the text back; `none` if the key is another, or if the code was changed. |

The same text encrypted twice gives two different codes, and a code that was changed, even by one
character, is refused rather than giving a wrong text.

### Fingerprints and signatures

`s.sha256`, `files.sha256(path)` and `s.hmac_sha256(key)` are in [String](/library/string/#fingerprints-and-base64)
and [files](/library/files/).

Every algorithm here (SHA-256, HMAC, BLAKE2b, Argon2id, XChaCha20-Poly1305) is written in Kova's
runtime, checked against the examples of their standards and against reference libraries, and
works the same on every system.
