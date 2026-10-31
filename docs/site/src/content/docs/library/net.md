---
title: net
description: TCP and UDP, for speaking any protocol yourself.
sidebar:
  order: 22
---

```kova
import net
```

`import web` speaks HTTP for you. `import net` gives what HTTP is built on, to speak any other
protocol: a game server, a mail client, a device on your network.

There are two ways to send data over a network:

- **TCP** opens a connection, a `net.Link`. What you send arrives whole and in order, or you get
  an error. Use it unless you have a reason not to.
- **UDP** sends separate messages, `net.Datagram`, with no connection. It is faster to start, but a
  message may be lost or arrive twice, and nothing tells you. Games and live sound use it, because
  an old message is worth nothing.

## TCP

| Function | What it does |
| --- | --- |
| `net.connect(host, port) -> net.Link or net.Error` | opens a connection. |
| `net.connect(host, port) within(seconds)` | gives up after that time on connecting and on each wait for data. |
| `net.connect_secure(host, port) -> net.Link or net.Error` | the same with TLS, the encryption of HTTPS. `within(seconds)` works too. |
| `net.listen(port) -> net.Listener or net.Error` | waits for connections from this computer only. |
| `net.listen_network(port) -> net.Listener or net.Error` | waits for connections from other computers too. |

| Method | What it does |
| --- | --- |
| `link.send(text)` | sends a text. Gives `Nothing or net.Error`, like every sending below. |
| `link.send_bytes(data: List<Int>)` | sends bytes. |
| `link.receive -> List<Int> or net.Error` | what has arrived, waiting if nothing has. Empty once the other end has closed. |
| `link.read_line -> String? or net.Error` | the next line without its line break. `none` once the other end has closed. |
| `link.peer` | who is at the other end, for a connection from `accept`. |
| `link.close` | closes the connection. |
| `listener.accept -> net.Link or net.Error` | waits for the next connection. |
| `listener.accept_within(seconds) -> net.Link or net.Error` | the same, with `TimedOut` after that time. |
| `listener.close` | stops listening. |

TCP sends a flow of bytes, not messages: two `send` may arrive in one `receive`, one may arrive in
two. So a protocol has to say where a message ends. The simplest way is one message per line,
which is what `read_line` is for.

A server does two things at once, waiting and talking, so here the listener runs in a
[thread](/library/threads/):

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
    let port: Int := random(20000..32000)
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

| Function or method | What it does |
| --- | --- |
| `net.udp(port) -> net.Socket or net.Error` | opens a socket on this port, for this computer only. `0` lets the system choose the port. |
| `net.udp(port) within(seconds)` | `receive` gives `TimedOut` after that time. |
| `net.udp_network(port)` | the same, reachable from other computers. `within(seconds)` works too. |
| `socket.send_to(host, port, data: List<Int>)` | sends one message. |
| `socket.receive -> net.Datagram or net.Error` | waits for one message. |
| `socket.close` | closes the socket. |

A `net.Datagram` has `host: String`, `port: Int` and `data: List<Int>`: answer to `host` and `port`
to reach whoever wrote.

```kova
import net

fn main {
    let port: Int := random(20000..32000)
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

## Errors

`net.Error` has the cases `NoHost(host)`, `Refused(host)`, `TimedOut(host)`, `Cut(host)`,
`BadCertificate(host)`, `PortTaken(port)` and `Closed`.

`Refused` means the computer answered and nothing listens on that port. `TimedOut` means nothing
answered at all, which is often a firewall.

Closing a link from one thread does not wake a `receive` waiting in another: give the waiting side
a time limit with `within`.
