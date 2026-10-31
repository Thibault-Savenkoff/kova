---
title: Kova in a web page
description: kova build --target web makes one .html file that runs your program in any browser.
sidebar:
  order: 2
---

A Kova program can run in a browser. There is nothing to install and nothing to serve:

```console
$ kova build counter.kova --target web
✓ counter.html (Web, 141 KB) in 13 ms
```

`counter.html` is a single file. Open it with a double click, send it to someone, or put it on any
web host: the program is inside it, compiled to WebAssembly, and it runs on the visitor's machine.

The same program builds for Linux, Windows, macOS and the web. What changes is what the page gives
it in place of a system.

## What a program finds in a page

| In the program | In the page |
|---|---|
| `print`, `write` | text on the page; errors in red |
| `read_line` | a field appears; the program waits for Enter |
| `system.sleep` | the program waits, the page stays alive |
| `import ui` | real page elements: buttons, fields, lists, tables |
| `ui.canvas` | a `<canvas>` |
| `ui.gpu` | WebGL 2: shaders written for OpenGL 3.3 are adapted when they compile |
| `import sound` | the browser plays the WAV |
| `import files` | files in the page's memory |
| `web.get`, `web.post`, `web.send` | the browser's `fetch` |

Everything else (texts, Lists, Maps, JSON, regular expressions, dates, hashes, `secure`…) works as
it does everywhere. The numbers are the same to the last digit: the same code computes them.

## Waiting without blocking

A web page has one rule: nothing may block it, or it freezes. Yet `system.sleep(1.0)` waits, and so
does `read_line`. Kova keeps them simple to write: when a built-in has to wait, the functions on the
way to it save where they are and return, the page goes on, and they come back to the same line
afterwards. You write ordinary code.

A long computation with no wait in it does keep the page busy until it ends, and what it prints
appears at its end. To show progress, give the page a moment: `system.sleep(0.0)` is not enough, a
wait must be above 0, so `system.sleep(0.001)` every so often does it.

## Windows

`import ui` needs no change. `ui.run` shows the view in the page, and the title becomes the tab's.

- `frame(false)` removes the margin around the content. `width`, `height`, `ui.place` and
  `ui.minimize` do nothing: a page does not move the browser. `ui.full_screen` asks the browser.
- `ui.fill` takes the room of the browser window, and a resize draws again.
- `ui.image` and `d.image` take an address, relative to the page: put the image next to the `.html`.

## Files

A page has no disk. `import files` works on files kept in the page's memory: a program can write
then read them back, make folders and list them. They end with the page.

Two doors lead outside:

- `ui.open_file` asks the user for a file. It comes into the page's memory, and the path given back
  opens it with `files.read` as usual.
- `ui.save_file` gives a name. Writing to that name downloads the file on the user's machine.

`ui.open_folder` answers `none`: a page cannot be given a folder.

## Asking other sites

`web.get` goes through the browser, so the browser's rule applies: a site answers a page from
another address only if it allows it (CORS). When it does not, or when the machine does not answer,
the result is `err(Refused(…))`: the browser does not tell the two apart. Your own server, serving
the page, always answers.

## What a page cannot do

These are refused when you build, with one line saying which:

| Refused | Why |
|---|---|
| `extern` | no C library in a page |
| `import threads`, `workers(n)` | not in a page yet |
| `web.serve`, `import net` | a page cannot listen, nor open raw sockets |
| `system.run` | a page cannot start programs |

## How far recursion goes

The stack is the browser's, far smaller than a program's on a system: around 6,000 calls inside each
other in a simple function, fewer in a function with many variables, and it varies with the browser.
Past it, the program stops with an error and code 101. A deep walk that works on a system may need a
loop here.

## Size and speed

`hello` is 141 KB in a page, and only what the program uses is kept. Measured under node, programs
run about 1.3 to 4 times slower than the same program built for the system.

To put the module in a page of your own, `-o program.wasm` writes it alone. It needs the JavaScript
the `.html` holds to run.
