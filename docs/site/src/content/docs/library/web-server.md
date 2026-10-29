---
title: web server
description: Answering questions from browsers and programs with web.serve.
sidebar:
  order: 11
---

A web server is a function from a question to an answer. In Kova, that is exactly what it looks
like:

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

`web.serve(8080)` answers each question with what its [block](/guide/blocks/) gives, until the
program stops (`exit(0)` in a case stops it). Open `http://localhost:8080/` in a browser to see it.

When it starts, the server writes where to open it, then a line for each question it answers, on
the error output:

```console
✓ Serving on port 8080
  http://localhost:8080/    this computer only
  Stop with Ctrl+C
14:30:01  127.0.0.1  GET /book  200
```

## The question: `web.Request`

| Field or method | Holds |
| --- | --- |
| `q.method -> String` | `GET`, `POST`… |
| `q.path -> String` | the path, decoded, without the `?…` part: `/search` |
| `q.query(name) -> String?` | a value of the `?…` part: `q.query("author")` for `/search?author=Herbert` |
| `q.header(name) -> String?` | a header, whatever its capitals |
| `q.headers -> Map<String, String>` | every header, names in small letters |
| `q.body -> String` | what was sent with a `POST` or a `PUT` |

## The answer: `web.Answer`

| Written | Answers |
| --- | --- |
| `web.Answer.text(t)` | a text (`text/plain`, status 200) |
| `web.Answer.json(value)` | a struct, a list or a map as JSON, written like `to_json` |
| `web.Answer.status(404)` | that status, with its name as the body (`404 Not Found`) |
| `web.Answer.file(path)` | a file, its type from its name (`.html`, `.css`, `.png`…); `404` if it does not exist or the path goes up with `..` |
| `a.header(name) is(value)` | the answer with this header: `.header("Cache-Control") is("no-store")` |

## Serving a folder

`web.serve_files("public") at(8080)` serves the files of a folder, and `index.html` for a
folder's own address. It is all a static site needs.

## Who can ask

`web.serve` listens on `127.0.0.1`: only this computer can ask, which is what a program in
development wants. `web.serve_network(8080) { … }` and `web.serve_files_network("public") at(8080)`
let the other computers of the network ask too (the system may first ask you to allow it); the
server then writes the addresses they can use.

## Several questions at once

`web.serve(8080) workers(8) { … }` answers up to 8 questions at the same time, each on its own
thread, so a slow answer keeps no one else waiting. The block may read the values around it but
not change them ([why](/guide/concurrency/#why-it-is-safe)). `kova run` still answers one at a
time: only the built program uses threads.

Without `workers`, the server still reads every connection at once and gives the block whichever
question is complete first, so a browser's spare connection that says nothing blocks no one.

## What it writes

| Written | Writes |
| --- | --- |
| `web.serve(8080) { … }` | where to open it, then a line per question |
| `web.serve(8080) log(web.Log.Start) { … }` | only where to open it |
| `web.serve(8080) log(web.Log.Off) { … }` | nothing |

`log(…)` also goes after `serve_files(…) at(…)`, and before or after `workers(…)`.

## What Kova handles itself

- A question it cannot read gets `400`, headers too long `431`, a body past 16 MB `413`, a body
  sent in pieces `501`: the block only sees questions that make sense.
- A port another program already has stops the program, with the exit code 101.
