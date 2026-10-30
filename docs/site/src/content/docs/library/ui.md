---
title: ui
description: Native windows from a state, a view and an update.
sidebar:
  order: 14
---

```kova
import ui
```

The guide's [Windows](/guide/windows/) chapter explains how a window program is made. This page
lists what `ui` has.

| Written | Does |
| --- | --- |
| `ui.run(title, state, view, update)` | shows a window titled `title`, drawn by `view(state)`, until it is closed. At each click, `update(var state, event)` runs, and the window is drawn again. |
| `ui.run(…) width(w) height(h)` | the same, the window opening `w` pixels wide and `h` high; each part may be left out. |
| `ui.run(…) frame(false)` | the window without the system's frame ([your own frame](/guide/windows/#a-window-with-your-own-frame)). |
| `ui.close`, `ui.minimize`, `ui.maximize`, `ui.drag` | from `update`: closes the window, minimizes it, maximizes it or restores it, moves it with the mouse after a press. |
| `ui.text(s: String) -> ui.View` | a line of text. |
| `ui.button(label: String, event: E) -> ui.View` | a button; a click sends `event` to `update`. |
| `ui.input(value: String) { s: String => E } -> ui.View` | a text field holding `value`; at each change the user makes, the block's event, made from the text the field now holds, goes to `update` ([Text fields](/guide/windows/#text-fields)). |
| `ui.image(path: String) -> ui.View` | an image file: PNG or JPEG on every system. A path that does not exist stops the program, naming it. `.size(w)` gives its width in pixels, the height following. |
| `ui.canvas(width: Int, height: Int) { d: ui.Draw => … } -> ui.View` | a drawing area ([Drawing](/guide/windows/#drawing)). `ui.fill` in place of a size takes all the room there is. |
| `ui.column(items: List<ui.View>) -> ui.View` | the views one under the other. |
| `ui.row(items: List<ui.View>) -> ui.View` | the views side by side. |
| `ui.column(…) align(left\|center\|right)` | the column takes the whole width it is given and places its views there. |
| `ui.row(…) align(top\|center\|bottom)` | the views at the top, in the middle or at the bottom of the row. |
| `view.color(c: ui.Color) -> ui.View` | the color of a text or a field. A button keeps the system's color. |
| `view.size(pixels: Int) -> ui.View` | the height of the text, for a text, a button or a field. |
| `view.bold -> ui.View` | the text in bold. |
| `view.margin(pixels: Int) -> ui.View` | empty pixels all around the view. |
| `view.on_mouse { m: ui.Mouse => E } -> ui.View` | on a canvas: the block's event at each press, release and move of the mouse. |
| `view.on_key { k: ui.Key => E } -> ui.View` | on a canvas: the block's event at each key pressed. |
| `view.every(ms: Int, event: E) -> ui.View` | sends `event` every `ms` milliseconds, on any view. |
| `ui.rgb(r: Int, g: Int, b: Int) -> ui.Color` | any color, each part from 0 to 255. |
| `ui.red`, `ui.green`, `ui.blue`, `ui.orange`, `ui.yellow`, `ui.purple`, `ui.gray`, `ui.black`, `ui.white` | ready-made colors. |

## Drawing on a canvas

| Written | Does |
| --- | --- |
| `d.clear(c: ui.Color)` | paints the whole canvas. |
| `d.rect(x, y) size(w, h) color(c) outline(n)` | a rectangle from its top left corner. |
| `d.circle(x, y) radius(r) color(c) outline(n)` | a circle from its center. |
| `d.line(x, y) to(x2, y2) color(c) thickness(n)` | a line, 1 pixel thick by default. |
| `d.text(s, x, y) size(n) color(c)` | a text from its top left corner, 16 pixels high by default. |
| `d.width`, `d.height` | the canvas's size. |

All numbers are `Int`, in pixels. `color` is black by default. `outline(n)` draws only the edge;
without it, the shape is filled.

| Field | Is |
| --- | --- |
| `m.x`, `m.y` | where the mouse is in the canvas. |
| `m.pressed`, `m.released`, `m.moved` | what just happened. |
| `m.right` | it is the right button. |
| `k.name` | what the key writes, or `Left`, `Right`, `Up`, `Down`, `Space`, `Enter`, `Escape`, `Tab`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `F1` to `F12`. |

## Placing and style

Everything is placed from the top left, on every system, unless `align` says otherwise.

Style methods follow each other in any order: `ui.text("Total").size(20).bold.color(ui.red)`. On a
column or a row, `color`, `size` and `bold` go to all its views, and a view's own style wins.
`margin` stays on the view it is written on.

The program writes the two functions `ui.run` takes:

- `fn view(app: App) -> ui.View`, where `App` is the type of the state;
- `fn update(app: var App, e: Event)`, where `Event` is the type of the buttons' events.

Their names are free, and a program has one `ui.run`. The compiler checks both shapes, and that
every button's event is an `Event`.

| System | Windows made with | Needs |
| --- | --- | --- |
| Linux | GTK 4 | GTK 4, there on most desktops |
| Windows | Win32 | nothing |
| macOS | AppKit | nothing |

## Environment variables

| Variable | Effect |
| --- | --- |
| `KOVA_UI_SCRIPT=1` | no window: each view is printed as text, and the button to click (its number from 0, in the order they are written), or `k=text` to type into field `k`, or a gesture such as `k=press 10 20 1` or `k=key Left` for a canvas, is read from the input; the end of the input closes the window. For tests. |
| `KOVA_UI_HIDDEN=1` | the window exists but is never shown (Windows, macOS), for a program that drives it. |

Without `width` and `height`, the window opens at the size of its content, at least 320 pixels wide. It is never smaller than its content, whatever size is asked, and whoever uses it can resize it: a click never puts back the first size, it only makes the window grow when the content needs more room.

A `ui` program runs as native code, even under `kova run`.
