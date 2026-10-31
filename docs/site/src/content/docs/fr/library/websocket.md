---
title: websocket
description: Une connexion qui reste ouverte entre une page web et un programme.
sidebar:
  order: 23
---

```kova
import websocket
```

En HTTP, le client demande et le serveur répond, une fois. Un **WebSocket** commence comme une
question HTTP, puis reste ouvert : chaque côté peut envoyer un message à tout moment. C'est ce
qu'utilisent les chats et les tableaux de bord en direct, et les navigateurs le parlent nativement.

| Fonction ou méthode | Ce qu'elle fait |
| --- | --- |
| `websocket.connect(address) -> websocket.Socket or websocket.Error` | ouvre une connexion vers `ws://…`, ou `wss://…` pour une connexion chiffrée. |
| `websocket.accept(link: move net.Link) -> websocket.Socket or websocket.Error` | répond à un client arrivé sur un [`net.Listener`](/fr/library/net/). |
| `socket.send(text)` | envoie un message texte. Donne `Nothing or websocket.Error`. |
| `socket.send_bytes(data: List<Int>)` | envoie un message binaire. |
| `socket.receive -> websocket.Message or websocket.Error` | attend le message suivant. |
| `socket.close` | dit au revoir et ferme la connexion. |

Un `websocket.Message` est `Text(text)`, `Bytes(data)` ou `Closed`, quand l'autre bout est parti.
Les pings qu'une connexion envoie pour rester en vie reçoivent leur réponse sans toi.

Contrairement à TCP, WebSocket garde les messages entiers : un `send` donne un `receive`.

Un serveur prend une connexion de `net.Listener.accept` et la donne à `websocket.accept`. Il la
cède pour de bon, d'où `move`. Ensuite il lit jusqu'à `Closed` :

```kova
import net
import threads
import websocket

fn echo(listener: net.Listener) -> Nothing or websocket.Error {
    let link: net.Link := listener.accept?
    var socket: websocket.Socket := websocket.accept(move link)?
    while true {
        match socket.receive? {
            .Text(text) => socket.send("echo: {text}")?
            .Bytes(data) => socket.send_bytes(data)?
            .Closed => break
        }
    }
    socket.close
    return ok
}

fn main {
    let port: Int := random(20000..32000)
    let listener: net.Listener := net.listen(port)?
    let server: threads.Task<Nothing or websocket.Error> := threads.spawn { echo(listener) }

    var socket: websocket.Socket := websocket.connect("ws://127.0.0.1:{port}/chat")?
    socket.send("hello")?
    match socket.receive? {
        .Text(text) => print(text)
        _ => print("something else")
    }
    socket.close
    server.wait?
}
```

```text
echo: hello
```

Pour servir des navigateurs d'autres ordinateurs, écoute avec `net.listen_network`.

`websocket.Error` a les cas `BadAddress(address)`, `Refused(reason)` quand l'autre bout ne parle pas
WebSocket, `Network(error)` avec le `net.Error` en dessous, et `BadFrame` quand ce qui arrive n'est
pas du WebSocket valide. Comme `Network` contient un `net.Error`, `?` convertit l'un en l'autre,
comme dans `listener.accept?` ci-dessus.
