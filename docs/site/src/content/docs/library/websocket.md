---
title: websocket
description: A connection that stays open between a web page and a program.
sidebar:
  order: 23
---

```kova
import websocket
```

In HTTP, the client asks and the server answers, once. A **WebSocket** starts as an HTTP question,
then stays open: either side can send a message at any moment. This is what chats and live
dashboards use, and browsers speak it natively.

| Function or method | What it does |
| --- | --- |
| `websocket.connect(address) -> websocket.Socket or websocket.Error` | opens a connection to `ws://…`, or `wss://…` for an encrypted one. |
| `websocket.accept(link: move net.Link) -> websocket.Socket or websocket.Error` | answers a client that reached a [`net.Listener`](/library/net/). |
| `socket.send(text)` | sends a text message. Gives `Nothing or websocket.Error`. |
| `socket.send_bytes(data: List<Int>)` | sends a binary message. |
| `socket.receive -> websocket.Message or websocket.Error` | waits for the next message. |
| `socket.close` | says goodbye and closes the connection. |

A `websocket.Message` is `Text(text)`, `Bytes(data)` or `Closed`, when the other end has left.
The pings a connection sends to stay alive are answered for you.

Unlike TCP, WebSocket keeps messages whole: one `send` is one `receive`.

A server takes a connection from `net.Listener.accept` and gives it to `websocket.accept`. That
hands it over for good, hence `move`. Then it reads until `Closed`:

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

To serve browsers from other computers, listen with `net.listen_network`.

`websocket.Error` has the cases `BadAddress(address)`, `Refused(reason)` when the other end does
not speak WebSocket, `Network(error)` with the `net.Error` underneath, and `BadFrame` when what
arrives is not valid WebSocket. Since `Network` holds a `net.Error`, `?` converts one into the
other, as in `listener.accept?` above.
