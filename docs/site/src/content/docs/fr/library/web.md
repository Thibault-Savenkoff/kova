---
title: web
description: Interroger des sites avec GET, POST ou n'importe quelle requête, en HTTP et HTTPS.
sidebar:
  order: 10
---

```kova
import web
```

## Interroger un site

```kova
import web

fn main {
    match web.get("https://example.com/") {
        ok(page) => {
            print(page.status)
            print(page.header("Content-Type"))
            print(page.body.length)
        }
        err(e) => print("no answer: {e}")
    }
}
```

| Fonction | Donne |
| --- | --- |
| `web.get(url) -> web.Response or web.WebError` | demande une page. |
| `web.post(url) body(text) -> web.Response or web.WebError` | envoie un texte, un formulaire ou du JSON. |
| `web.send(request) -> web.Response or web.WebError` | envoie une requête construite pas à pas (ci-dessous). |

**Une réponse est toujours `ok`**, même un `404` ou un `500` : le site a bien répondu, et
`page.status` dit quoi. `err` veut dire qu'il n'y a eu aucune réponse. Cela garde les deux
questions séparées : « le réseau a-t-il marché ? », c'est le `match` ; « le site est-il
d'accord ? », c'est le statut.

Les redirections sont suivies, 10 au plus ; `page.url` dit où ça s'est terminé.

### `web.Response`

| Champ ou méthode | Contient |
| --- | --- |
| `page.status -> Int` | le statut : `200`, `404`… |
| `page.body -> String` | le texte de la réponse |
| `page.url -> String` | l'adresse qui a répondu, après les redirections |
| `page.headers -> Map<String, String>` | chaque en-tête, noms en minuscules |
| `page.header(name) -> String?` | un en-tête, quelles que soient les majuscules de `name` |

Avec du JSON, le corps devient une struct à toi en une ligne :
`Book.from_json(page.body)` ([JSON](/fr/library/json-and-csv/)).

## Construire une requête

Pour une autre méthode, des en-têtes ou une limite de temps, construis une `web.Request`, puis
envoie-la :

```kova
import web

fn main {
    let asked: web.Request := web.Request.put("https://example.com/notes/1")
        .header("Accept") is("text/plain")
        .body("hello")
        .seconds(5)
        .no_redirections
    match web.send(asked) {
        ok(page) => print(page.status)
        err(e) => print("no answer: {e}")
    }
}
```

| Écrit | Donne |
| --- | --- |
| `web.Request.get(url)`, `.post`, `.put`, `.patch`, `.delete`, `.head` | une requête avec cette méthode |
| `r.header(name) is(value)` | la requête avec cet en-tête |
| `r.body(text)` | la requête avec ce corps |
| `r.seconds(s: Float)` | le temps d'attente maximal pour chaque étape ; `0` attend pour toujours |
| `r.no_redirections` | une redirection est rendue telle quelle, au lieu d'être suivie |

Chaque étape donne une copie changée, donc une requête de base peut être gardée et réutilisée.
Sans `seconds`, Kova attend 30 secondes au plus pour chaque étape (la connexion, chaque morceau de
la réponse).

## `web.WebError`

Chaque cas contient l'adresse demandée :

| Cas | Veut dire |
| --- | --- |
| `BadUrl(url)` | l'adresse n'est pas une que Kova sait demander (`http://` ou `https://`) |
| `NoHost(url)` | le nom n'existe pas, ou il n'y a pas de réseau |
| `Refused(url)` | l'ordinateur est là, mais rien ne répond sur ce port |
| `Timeout(url)` | pas de réponse à temps |
| `Broken(url)` | la connexion s'est coupée au milieu |
| `BadAnswer(url)` | ce qui est revenu n'est pas du HTTP |
| `TooManyRedirects(url)` | plus de 10 redirections |
| `BadCertificate(url)` | le certificat du site n'est pas valide pour lui |
| `NoHttps(url)` | pas de bibliothèque TLS sur cet ordinateur Linux |
| `BadHeader(url)` | un en-tête que Kova écrit lui-même (`Host`, `Content-Length`…), ou un qui contient un saut de ligne |

## HTTPS

`https://` utilise le chiffrement de chaque système (SChannel sous Windows, Network.framework sous
macOS, OpenSSL sous Linux), donc les correctifs de sécurité arrivent avec les mises à jour du
système. Les certificats sont toujours vérifiés ; il n'y a pas d'interrupteur pour l'empêcher.

Sous Linux, un programme qui utilise `web` a besoin de la bibliothèque C du système, et `https://`
a besoin d'OpenSSL, trouvé quand le programme tourne. Sous Windows et macOS, rien d'autre n'est
nécessaire.

Pour répondre aux questions au lieu d'en poser, voir [le serveur web](/fr/library/web-server/).
