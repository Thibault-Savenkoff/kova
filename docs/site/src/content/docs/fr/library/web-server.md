---
title: serveur web
description: Répondre aux questions des navigateurs et des programmes avec web.serve.
sidebar:
  order: 11
---

Un serveur web est une fonction qui va d'une question à une réponse. En Kova, c'est exactement à
quoi il ressemble :

```kova
import web

struct Book {
    title: String
    pages: Int
}

fn main {
    web.serve(8080) { q: web.Request =>
        match q.path {
            "/" => web.Answer.text("Hello from Kova")
            "/book" => web.Answer.json(Book { title := "Dune", pages := 412 })
            "/search" => match q.query("author") {
                none => web.Answer.status(400)
                author => web.Answer.text("books by {author}")
            }
            _ => web.Answer.file("public" + q.path)
        }
    }
}
```

`web.serve(8080)` répond à chaque question avec ce que donne son [bloc](/fr/guide/blocks/), jusqu'à
ce que le programme s'arrête (`system.exit(0)` dans un cas l'arrête). Ouvre `http://localhost:8080/` dans
un navigateur pour le voir.

Au démarrage, le serveur écrit où l'ouvrir, puis une ligne pour chaque question à laquelle il
répond, sur la sortie d'erreur :

```console
✓ Serveur sur le port 8080
  http://localhost:8080/    cet ordinateur seulement
  Ctrl+C pour arrêter
14:30:01  127.0.0.1  GET /book  200
```

## La question : `web.Request`

| Champ ou méthode | Contient |
| --- | --- |
| `q.method -> String` | `GET`, `POST`… |
| `q.path -> String` | le chemin, décodé, sans la partie `?…` : `/search` |
| `q.query(name) -> String?` | une valeur de la partie `?…` : `q.query("author")` pour `/search?author=Herbert` |
| `q.header(name) -> String?` | un en-tête, quelles que soient ses majuscules |
| `q.headers -> Map<String, String>` | chaque en-tête, noms en minuscules |
| `q.body -> String` | ce qui a été envoyé avec un `POST` ou un `PUT` |
| `q.bytes -> List<Int>` | la même chose en octets, pour un envoi de fichier : `body` change en `�` ce qui n'est pas du texte |

## La réponse : `web.Answer`

| Écrit | Répond |
| --- | --- |
| `web.Answer.text(t)` | un texte (`text/plain`, statut 200) |
| `web.Answer.json(value)` | une struct, une liste ou une map en JSON, écrite comme `to_json` |
| `web.Answer.status(404)` | ce statut, avec son nom comme corps (`404 Not Found`) |
| `web.Answer.file(path)` | un fichier, son type d'après son nom (`.html`, `.css`, `.png`…) ; `404` s'il n'existe pas ou si le chemin remonte avec `..` |
| `web.Answer.bytes(data: List<Int>)` | des octets produits par le programme, en `application/octet-stream` ; donne un autre `content-type` avec `.header(…) is(…)` |
| `a.status(404)` | la réponse avec ce statut, en gardant son corps : `web.Answer.text("no such book").status(404)` |
| `a.header(name) is(value)` | la réponse avec cet en-tête : `.header("Cache-Control") is("no-store")` |

## Servir un dossier

`web.serve_files("public") at(8080)` sert les fichiers d'un dossier, et `index.html` pour l'adresse
d'un dossier. C'est tout ce dont a besoin un site statique.

## Qui peut demander

`web.serve` écoute sur `127.0.0.1` : seul cet ordinateur peut demander, ce que veut un programme en
développement. `web.serve_network(8080) { … }` et `web.serve_files_network("public") at(8080)`
laissent aussi demander les autres ordinateurs du réseau (le système peut d'abord te demander de
l'autoriser) ; le serveur écrit alors les adresses qu'ils peuvent utiliser.

## Plusieurs questions à la fois

`web.serve(8080) workers(8) { … }` répond à 8 questions au plus en même temps, chacune sur son
propre fil d'exécution, donc une réponse lente ne fait attendre personne d'autre. Le bloc peut lire
les valeurs autour de lui mais pas les changer ([pourquoi](/fr/guide/concurrency/#pourquoi-cest-sûr)).
`kova run` répond toujours à une seule à la fois : seul le programme compilé utilise des fils
d'exécution.

Sans `workers`, le serveur lit quand même toutes les connexions à la fois et donne au bloc la
question qui est complète la première, donc la connexion de réserve d'un navigateur qui ne dit rien
ne bloque personne.

## Ce qu'il écrit

| Écrit | Écrit |
| --- | --- |
| `web.serve(8080) { … }` | où l'ouvrir, puis une ligne par question |
| `web.serve(8080) log(web.Log.Start) { … }` | seulement où l'ouvrir |
| `web.serve(8080) log(web.Log.Off) { … }` | rien |

`log(…)` va aussi après `serve_files(…) at(…)`, et avant ou après `workers(…)`.

## Ce que Kova gère lui-même

- Une question qu'il ne peut pas lire reçoit `400`, des en-têtes trop longs `431`, un corps de plus
  de 16 Mo `413`, un corps envoyé en morceaux `501` : le bloc ne voit que des questions qui ont un
  sens.
- Un port qu'un autre programme a déjà arrête le programme, avec le code de sortie 101.
