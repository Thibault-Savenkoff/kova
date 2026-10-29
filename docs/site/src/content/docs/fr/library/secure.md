---
title: secure et le hasard
description: random pour les jeux et les simulations, secure pour les mots de passe, les jetons et le chiffrement.
sidebar:
  order: 12
---

Kova a deux sortes de hasard, exprès :

- `random` est rapide et peut être **répété** : avec le même `KOVA_SEED`, un programme tire les
  mêmes nombres, ce qui rend les jeux, les simulations et les tests reproductibles. Il ne doit
  jamais rien protéger : ses nombres peuvent être devinés.
- `secure` demande au système un hasard qui ne peut pas être deviné, pour les mots de passe, les
  clés et les jetons.

## `random`

Aucun import nécessaire.

| Fonction | Donne |
| --- | --- |
| `random(a..b) -> Int` | un nombre entier de `a` jusqu'à `b` non compris ; `random(a..=b)` comprend `b`. |
| `l.shuffled`, `l.chosen` | une liste dans un ordre au hasard, un élément tiré ([List](/fr/library/list/#ordre-et-hasard)). |

```kova
fn main {
    let die: Int := random(1..=6)
    print(die >= 1 && die <= 6)
}
```

```text
true
```

`KOVA_SEED=42 kova run game.kova` tire les mêmes nombres à chaque exécution, compilé ou non.

## `import secure`

```kova
import secure
```

### Un hasard sûr

| Fonction | Donne |
| --- | --- |
| `secure.random(a..b) -> Int` | comme `random`, avec le hasard du système lui-même. |
| `secure.token(bytes: Int) -> String` | autant d'octets de hasard, en hexadécimal (`token(16)` fait 32 caractères) : une clé de session, un lien de réinitialisation. |
| `secure.uuid -> String` | un UUID au hasard (version 4), `"3f2b…-…"`, 36 caractères. |

### Les mots de passe

Un mot de passe n'est jamais gardé, ni son SHA-256 : une liste d'empreintes volée se casse en
quelques heures. Garde ce que donne `secure.password_hash`, et vérifie un mot de passe avec :

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

| Fonction | Donne |
| --- | --- |
| `secure.password_hash(password) -> String` | une empreinte Argon2id à garder, avec son propre sel au hasard. |
| `secure.password_hash(password) memory(mb) passes(n)` | pareil avec un coût plus grand (8 à 1024 Mo, 1 à 16 passes). |
| `secure.password_matches(password) hash(stored) -> Bool` | si `password` est celui à partir duquel `stored` a été fait. |

Argon2id est lent et gourmand en mémoire exprès (19 Mo et 2 passes par défaut), donc chaque essai
coûte autant à un attaquant. Le texte garde ses réglages, donc ils peuvent être augmentés plus
tard sans rendre invalides les empreintes gardées.

### Le chiffrement

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

| Fonction | Donne |
| --- | --- |
| `secure.key -> String` | une nouvelle clé au hasard, en texte, à garder secrète. |
| `secure.encrypt(text) key(k) -> String` | un code que seule cette clé ouvre (XChaCha20-Poly1305). |
| `secure.decrypt(code) key(k) -> String?` | le texte de départ ; `none` si la clé est une autre, ou si le code a été changé. |

Le même texte chiffré deux fois donne deux codes différents, et un code changé, même d'un seul
caractère, est refusé plutôt que de donner un texte faux.

### Empreintes et signatures

`s.sha256`, `files.sha256(path)` et `s.hmac_sha256(key)` sont dans
[String](/fr/library/string/#empreintes-et-base64) et [files](/fr/library/files/).

Chaque algorithme ici (SHA-256, HMAC, BLAKE2b, Argon2id, XChaCha20-Poly1305) est écrit dans le
runtime de Kova, vérifié contre les exemples de leurs normes et contre des bibliothèques de
référence, et marche pareil sur chaque système.
