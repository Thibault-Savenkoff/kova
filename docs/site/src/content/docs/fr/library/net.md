---
title: net
description: TCP et UDP, pour parler n'importe quel protocole toi-même.
sidebar:
  order: 22
---

```kova
import net
```

`import web` parle HTTP à ta place. `import net` donne ce sur quoi HTTP est construit, pour parler
n'importe quel autre protocole : un serveur de jeu, un client mail, un appareil de ton réseau.

Il y a deux façons d'envoyer des données sur un réseau :

- **TCP** ouvre une connexion, un `net.Link`. Ce que tu envoies arrive entier et dans l'ordre, ou
  tu reçois une erreur. Utilise-le sauf si tu as une raison de ne pas le faire.
- **UDP** envoie des messages séparés, des `net.Datagram`, sans connexion. C'est plus rapide à
  démarrer, mais un message peut se perdre ou arriver deux fois, et rien ne te le dit. Les jeux et
  le son en direct s'en servent, parce qu'un vieux message ne vaut plus rien.

## TCP

| Fonction | Ce qu'elle fait |
| --- | --- |
| `net.connect(host, port) -> net.Link or net.Error` | ouvre une connexion. |
| `net.connect(host, port) within(seconds)` | abandonne après ce temps, pour se connecter et à chaque attente de données. |
| `net.connect_secure(host, port) -> net.Link or net.Error` | la même chose avec TLS, le chiffrement de HTTPS. `within(seconds)` marche aussi. |
| `net.listen(port) -> net.Listener or net.Error` | attend des connexions venant de cet ordinateur seulement. |
| `net.listen_network(port) -> net.Listener or net.Error` | attend aussi des connexions venant d'autres ordinateurs. |

| Méthode | Ce qu'elle fait |
| --- | --- |
| `link.send(text)` | envoie un texte. Donne `Nothing or net.Error`, comme tous les envois ci-dessous. |
| `link.send_bytes(data: List<Int>)` | envoie des octets. |
| `link.receive -> List<Int> or net.Error` | ce qui est arrivé, en attendant si rien n'est là. Vide quand l'autre bout a fermé. |
| `link.read_line -> String? or net.Error` | la ligne suivante sans son saut de ligne. `none` quand l'autre bout a fermé. |
| `link.peer` | qui est à l'autre bout, pour une connexion venue d'`accept`. |
| `link.close` | ferme la connexion. |
| `listener.accept -> net.Link or net.Error` | attend la prochaine connexion. |
| `listener.accept_within(seconds) -> net.Link or net.Error` | la même chose, avec `TimedOut` après ce temps. |
| `listener.close` | arrête d'écouter. |

TCP envoie un flot d'octets, pas des messages : deux `send` peuvent arriver dans un seul `receive`,
un seul peut arriver en deux fois. Un protocole doit donc dire où finit un message. Le plus simple
est un message par ligne, et c'est à ça que sert `read_line`.

Un serveur fait deux choses à la fois, attendre et parler : ici, l'écoute tourne donc dans un
[fil](/fr/library/threads/) :

```kova
import net
import threads

fn answer(listener: net.Listener) -> Nothing or net.Error {
    var link: net.Link := listener.accept?
    let line: String := link.read_line? else "nothing"
    link.send("you said: {line}\n")?
    link.close
    return ok
}

fn main {
    let port: Int := random(30000..60000)
    let listener: net.Listener := net.listen(port)?
    let server: threads.Task<Nothing or net.Error> := threads.spawn { answer(listener) }

    var link: net.Link := net.connect("127.0.0.1", port)?
    link.send("hello\n")?
    print(link.read_line? else "closed")
    link.close
    server.wait?
}
```

```text
you said: hello
```

## UDP

| Fonction ou méthode | Ce qu'elle fait |
| --- | --- |
| `net.udp(port) -> net.Socket or net.Error` | ouvre un socket sur ce port, pour cet ordinateur seulement. `0` laisse le système choisir le port. |
| `net.udp(port) within(seconds)` | `receive` donne `TimedOut` après ce temps. |
| `net.udp_network(port)` | la même chose, joignable depuis d'autres ordinateurs. `within(seconds)` marche aussi. |
| `socket.send_to(host, port, data: List<Int>)` | envoie un message. |
| `socket.receive -> net.Datagram or net.Error` | attend un message. |
| `socket.close` | ferme le socket. |

Un `net.Datagram` a `host: String`, `port: Int` et `data: List<Int>` : réponds à `host` et `port`
pour joindre celui qui a écrit.

```kova
import net

fn main {
    let port: Int := random(30000..60000)
    let inbox: net.Socket := net.udp(port) within(2.0)?
    let sender: net.Socket := net.udp(0)?
    sender.send_to("127.0.0.1", port, "ping".to_bytes)?
    let message: net.Datagram := inbox.receive?
    print(message.data.to_text else "not a text")
    inbox.close
    sender.close
}
```

```text
ping
```

## Erreurs

`net.Error` a les cas `NoHost(host)`, `Refused(host)`, `TimedOut(host)`, `Cut(host)`,
`BadCertificate(host)`, `PortTaken(port)` et `Closed`.

`Refused` veut dire que l'ordinateur a répondu et que rien n'écoute sur ce port. `TimedOut` veut
dire que rien n'a répondu du tout, souvent à cause d'un pare-feu.

Fermer un lien depuis un fil ne réveille pas un `receive` qui attend dans un autre : donne une
limite de temps au côté qui attend avec `within`.
