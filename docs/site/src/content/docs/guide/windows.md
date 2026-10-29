---
title: Windows
description: Native windows with import ui, from a state, a view and an update.
sidebar:
  order: 17
---

`import ui` opens a window of the system itself: GTK on Linux, Win32 on Windows, AppKit on macOS.
The program looks and behaves like the other programs of the system, and needs nothing installed
on Windows and macOS (Linux needs GTK 4, present on most desktops).

## State, view, update

A window program is written in three parts, as in Elm or SwiftUI:

1. a **state**: a struct holding everything the window shows;
2. a **view**: a function that draws the state;
3. an **update**: a function that changes the state when something happens.

```kova
import ui

enum Event {
    Plus
    Minus
}

struct App {
    clicks: Int
}

fn view(app: App) -> ui.View {
    return ui.column([
        ui.text("{app.clicks} clicks"),
        ui.row([ui.button("+1", Event.Plus), ui.button("-1", Event.Minus)]),
    ])
}

fn update(app: var App, e: Event) {
    match e {
        Plus => app.clicks := app.clicks + 1
        Minus => app.clicks := app.clicks - 1
    }
}

fn main {
    ui.run("Counter", App { clicks := 0 }, view, update)
    print("closed")
}
```

`ui.run(title, state, view, update)` shows the window and returns when it is closed. At each click,
the button's event goes to `update`, which changes the state, and the window is drawn again from
`view`.

Nothing else ever changes the window. There is no label to find and change, no button to
enable from somewhere else: what is on the screen is always `view(state)`. A window that shows
something wrong is a `view` to read, or an `update` to fix, never a forgotten change somewhere.

## Building a view

| Function | Shows |
| --- | --- |
| `ui.text(s)` | a line of text |
| `ui.button(label, event)` | a button; a click sends `event` to `update` |
| `ui.column([…])` | views one under the other |
| `ui.row([…])` | views side by side |

The event of a button is of the type `update` takes, usually an enum listing what can happen. It
may carry values: `Named(name: String)`, `Remove(index: Int)`.

The compiler checks the shapes: `view` takes the state and gives a `ui.View`, `update` takes the
state as `var` and an event, and every button's event has the right type.

## Testing a window

A window cannot be looked at by a test. With `KOVA_UI_SCRIPT=1`, no window opens: each view is
printed as text, and the number of the button to click (counted from 0, in the order they are
written) is read from the input; the end of the input closes the window. With no input:

```text
[Counter] column(text "0 clicks", row(button "+1", button "-1"))
closed
```

With the input `0`, then `0`, then `1`, it shows 1 click, 2, then 1 again, then closes. A test can
so drive a window program through every case, on any machine, even without a screen.

## What comes next

`import ui` has text, buttons, rows and columns today. Text fields and more widgets are planned.
For a widget it does not have yet, a program can call the system's own library through
[`extern`](/guide/calling-c/).
