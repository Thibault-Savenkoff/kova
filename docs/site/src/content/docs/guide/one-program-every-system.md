---
title: One program, every system
description: The same file becomes an app for Linux, Windows, macOS, Android and iPhone, with the system's own look.
sidebar:
  order: 19
---

A program written with [`import ui`](/guide/windows/) is not tied to a computer. The same file can
be built for a phone, and it keeps its shape: a state, a `view`, an `update`. Kova draws it with
the views of the system it is built for, so each app looks like the apps around it.

## One file, one command per system

```console
$ kova build clock.kova                          # this computer
$ kova build clock.kova --target android         # an .apk for phones
$ kova build clock.kova --target ios-simulator   # an app for Xcode's simulator
$ kova build clock.kova --target web             # a page for a browser
```

Nothing in the file changes between these lines. [Kova on Android](/tools/android/) and
[Kova on iOS](/tools/ios/) say what each system needs to install the result.

## Knowing where you run

Most of a program does not care. When one part does, `system.os` says which system the program
was built for: `"linux"`, `"windows"`, `"mac"`, `"android"`, `"ios"` or `"web"`. It is known when
the program is made, so a build for a phone never carries the computer's branch.

```kova
import system

fn on_phone -> Bool {
    return system.os = "android" || system.os = "ios"
}

fn main {
    if on_phone {
        print("a phone: big buttons")
    } else {
        print("a computer: a menu bar")
    }
}
```

```text
a computer: a menu bar
```

With `kova run`, `system.os` is the system you are on.

## What adapts by itself

You write nothing for these:

- **The views are the system's own.** A button is a real button: GTK, Win32, AppKit, UIKit, Android's
  views. Text fields, check boxes and lists behave as the user's other apps do.
- **The colors follow the system.** On Android the app takes Material 3 colors, from the user's
  wallpaper on Android 12 and later, and it changes between light and dark *while it is open*,
  the moment the user switches. On iPhone the system colors do the same.
- **Sizes are density independent.** `size(20)` or `margin(8)` look alike on a small phone and on a
  large screen.
- **The language is the system's**, unless the `header` says otherwise.

## What is made for phones

A few elements exist because phones have them. On the other systems they show as the closest
thing, so the file still builds and runs:

| You write | On a phone | Elsewhere |
|---|---|---|
| `ui.tabs(names, i) { … }.icons([…])` | tabs at the bottom, with an icon each | a row of tabs |
| `view.title("Notes")` | the title of the page, in the bar | nothing |
| `view.searchable(text) { … }` | a search field in the bar | a field above the page |
| `ui.list(rows, i, { … }) delete { … }` | swipe a row to the left to delete it | a plain list: add a button too |
| `ui.input(text) { … }.lines(8)` | a field of 8 lines, Return starts a new line | a one-line field |
| `view.markdown` | `**bold**` and `[links](…)` are shown as such | the text as written, with its stars |
| `view.background(ui.card).rounded(12)` | a card in the system's colors | no background |

[The `ui` library page](/library/ui/) lists them all. The rule is the one of the rest of Kova: an
element a system does not have never stops your program from building there.

## A worked example: a Pomodoro timer

A Pomodoro timer counts down 25 minutes of work, then 5 of rest. It is a small program, and it
shows the one thing a phone changes: **the program is not always running**.

```kova
import ui
import system

enum Event {
    Tick
    Toggle
    Reset
}

struct App {
    left: Int
    ends: Float
    running: Bool
}

let WORK_SECONDS: Int := 1500

fn clock(seconds: Int) -> String {
    let minutes: Int := seconds / 60
    let rest: Int := seconds % 60
    return "{minutes.to_string.padded_left(2) with("0")}:{rest.to_string.padded_left(2) with("0")}"
}

fn view(app: App) -> ui.View {
    let screen: ui.View := ui.column([
        ui.text(clock(app.left)).size(64).bold,
        ui.progress(WORK_SECONDS - app.left, WORK_SECONDS),
        ui.row([
            ui.button(if app.running { "Pause" } else { "Start" }, Event.Toggle),
            ui.button("Reset", Event.Reset),
        ]),
    ]) align(center)
    return if app.running { screen.every(250, Event.Tick) } else { screen }
}

fn update(app: var App, e: Event) {
    match e {
        .Tick => {
            app.left := (app.ends - system.clock).ceil
            if app.left <= 0 {
                app.running := false
                app.left := WORK_SECONDS
            }
        }
        .Toggle => {
            app.running := !app.running
            app.ends := system.clock + app.left.to_float
        }
        .Reset => {
            app.running := false
            app.left := WORK_SECONDS
        }
    }
}

fn main {
    let start: App := App { left := WORK_SECONDS, ends := 0.0, running := false }
    ui.run("Pomodoro", start, view, update) width(360) height(420)
    print("closed")
}
```

```text
[Pomodoro] column center(text "25:00" size(64) bold, text "▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱ 0%", row(button "Start", button "Reset"))
closed
```

Two lines carry the idea. `screen.every(250, Event.Tick)` asks for a `Tick` four times a second,
only while the timer runs: a paused timer asks for nothing. And `Tick` does not subtract one second:
it reads the **clock** and works out what is left from the moment the timer must end, `ends`.

The difference matters on a phone. Counting ticks would be right on a computer, where the window
runs all the time. A phone puts an app to sleep when the user opens another one, and the ticks
stop. Reading the clock, the first `Tick` after the user comes back shows the right time, whatever
happened meanwhile. The complete program, with the sessions counted and a beep made in memory
(a WAV written by the program itself, no sound file to carry), is `examples/pomodoro.kova` in
Kova's repository.

## Testing without a phone

You do not need the phone for every change. The program is the same file, so most of it can be
tested on the computer: `KOVA_UI_SCRIPT=1` prints each view as text and clicks the buttons you
name, as [Windows](/guide/windows/#testing-a-window) shows. Phone details (the look, the swipe,
the keyboard) need a phone or an emulator: `kova build --target android-intel` makes an `.apk` for
the emulator Android's tools provide.

## What a phone does not have yet

- **No work in the background.** When Android or iOS puts the app away, the program waits, and may
  be closed. The Pomodoro cannot ring while another app is in front. A service that keeps running
  needs a part of Android that Kova does not have yet.
- **No notifications.** Nothing can be shown outside the app.
- **Files stay inside the app.** `import files` works in the app's own private folder; to read a
  file elsewhere, ask the user with `ui.open_file`.
- **One window.** A phone shows one app screen at a time; `view.pushing(…)` stacks pages with a
  back button.
