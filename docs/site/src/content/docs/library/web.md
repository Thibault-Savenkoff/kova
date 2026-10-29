---
title: web
description: Asking sites with GET, POST or any request, over HTTP and HTTPS.
sidebar:
  order: 10
---

```kova
import web
```

## Asking a site

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

| Function | Gives |
| --- | --- |
| `web.get(url) -> web.Response or web.WebError` | asks for a page. |
| `web.post(url) body(text) -> web.Response or web.WebError` | sends a text, a form or JSON. |
| `web.send(request) -> web.Response or web.WebError` | sends a request built step by step (below). |

**An answer is always `ok`**, even a `404` or a `500`: the site did answer, and `page.status` says
what. `err` means there was no answer at all. This keeps the two questions apart: "did the network
work?" is the `match`, "did the site agree?" is the status.

Redirections are followed, 10 at most; `page.url` says where it ended.

### `web.Response`

| Field or method | Holds |
| --- | --- |
| `page.status -> Int` | the status: `200`, `404`… |
| `page.body -> String` | the text of the answer |
| `page.url -> String` | the address that answered, after redirections |
| `page.headers -> Map<String, String>` | every header, names in small letters |
| `page.header(name) -> String?` | one header, whatever the capitals of `name` |

With JSON, the body becomes a struct of your own in one line:
`Book.from_json(page.body)` ([JSON](/library/json-and-csv/)).

## Building a request

For another method, headers, or a time limit, build a `web.Request`, then send it:

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

| Written | Gives |
| --- | --- |
| `web.Request.get(url)`, `.post`, `.put`, `.patch`, `.delete`, `.head` | a request with that method |
| `r.header(name) is(value)` | the request with this header |
| `r.body(text)` | the request with this body |
| `r.seconds(s: Float)` | the most to wait for each step; `0` waits forever |
| `r.no_redirections` | a redirection is given back as it is, instead of followed |

Each step gives a changed copy, so a base request can be kept and reused. Without `seconds`, Kova
waits 30 seconds at most for each step (connecting, each piece of the answer).

## `web.WebError`

Each case holds the address asked:

| Case | Means |
| --- | --- |
| `BadUrl(url)` | the address is not one Kova can ask (`http://` or `https://`) |
| `NoHost(url)` | the name does not exist, or there is no network |
| `Refused(url)` | the computer is there, but nothing answers on that port |
| `Timeout(url)` | no answer in time |
| `Broken(url)` | the connection broke in the middle |
| `BadAnswer(url)` | what came back is not HTTP |
| `TooManyRedirects(url)` | more than 10 redirections |
| `BadCertificate(url)` | the site's certificate is not valid for it |
| `NoHttps(url)` | no TLS library on this Linux computer |
| `BadHeader(url)` | a header Kova writes itself (`Host`, `Content-Length`…), or one holding a line break |

## HTTPS

`https://` uses each system's own encryption (SChannel on Windows, Network.framework on macOS,
OpenSSL on Linux), so security fixes come with the system's updates. Certificates are always
checked; there is no switch to turn that off.

On Linux, a program using `web` needs the system's C library, and `https://` needs OpenSSL, found
when the program runs. On Windows and macOS, nothing else is needed.

To answer questions instead of asking them, see [the web server](/library/web-server/).
