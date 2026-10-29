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
| `ui.text(s: String) -> ui.View` | a line of text. |
| `ui.button(label: String, event: E) -> ui.View` | a button; a click sends `event` to `update`. |
| `ui.input(value: String) { s: String => E } -> ui.View` | a text field holding `value`; at each change the user makes, the block's event, made from the text the field now holds, goes to `update` ([Text fields](/guide/windows/#text-fields)). |
| `ui.column(items: List<ui.View>) -> ui.View` | the views one under the other. |
| `ui.row(items: List<ui.View>) -> ui.View` | the views side by side. |

Everything is placed from the top left, on every system.

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
| `KOVA_UI_SCRIPT=1` | no window: each view is printed as text, and the button to click (its number from 0, in the order they are written), or `k=text` to type into field `k`, is read from the input; the end of the input closes the window. For tests. |
| `KOVA_UI_HIDDEN=1` | the window exists but is never shown (Windows, macOS), for a program that drives it. |

Without `width` and `height`, the window opens at the size of its content, at least 320 pixels wide. It is never smaller than its content, whatever size is asked, and whoever uses it can resize it: a click never puts back the first size, it only makes the window grow when the content needs more room.

A `ui` program runs as native code, even under `kova run`.
