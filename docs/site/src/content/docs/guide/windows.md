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
        .Plus => app.clicks := app.clicks + 1
        .Minus => app.clicks := app.clicks - 1
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

The window opens at the size of its content. To choose its first size, add `width(…)` and
`height(…)`, together or alone: `ui.run("Counter", App { clicks := 0 }, view, update) width(400)`.
It never becomes smaller than its content, and the person using it can resize it.

Nothing else ever changes the window. There is no label to find and change, no button to
enable from somewhere else: what is on the screen is always `view(state)`. A window that shows
something wrong is a `view` to read, or an `update` to fix, never a forgotten change somewhere.

## Building a view

| Function | Shows |
| --- | --- |
| `ui.text(s)` | a line of text |
| `ui.button(label, event)` | a button; a click sends `event` to `update` |
| `ui.input(value) { s: String => event }` | a text field holding `value`; each change sends the block's event to `update` |
| `ui.column([…])` | views one under the other |
| `ui.row([…])` | views side by side |
| `ui.column([…]) align(center)` | the column's views centered in the window (`left`, `center`, `right`) |
| `ui.row([…]) align(center)` | the row's views centered in its height (`top`, `center`, `bottom`) |

The event of a button is of the type `update` takes, usually an enum listing what can happen. It
may carry values: `Named(name: String)`, `Remove(index: Int)`.

Without `align`, everything starts at the top left. To center a row of buttons in the window, put
it in a centered column: `ui.column([ui.row([…])]) align(center)`.

The compiler checks the shapes: `view` takes the state and gives a `ui.View`, `update` takes the
state as `var` and an event, and every button's event has the right type.

## Styling a view

A view follows the system's look until you say otherwise. Style methods follow each other in any
order, and each gives a new view:

```kova
import ui

struct App {
    name: String
}

enum Event {
    Renamed(name: String)
}

fn view(app: App) -> ui.View {
    let wrong: Bool := app.name.length > 8
    return ui.column([
        ui.text("Your name").size(20).bold,
        ui.input(app.name) { s: String => Event.Renamed(s.clone) }.color(if wrong { ui.red } else { ui.black }),
        ui.text("Eight letters at most").size(11).color(ui.gray).margin(4),
    ])
}

fn update(app: var App, e: Event) {
    match e {
        .Renamed(name) => app.name := name.clone
    }
}

fn main {
    ui.run("Style", App { name := "" }, view, update)
}
```

Here the field's text turns red while the name is too long. The field stays the same field: the
person typing keeps their cursor.

Colors are `ui.red`, `ui.blue` and the others, or any color with `ui.rgb(37, 99, 235)`. On a column
or a row, `color`, `size` and `bold` go to all its views; a view's own style wins. `margin` adds
empty pixels around the view it is written on.

A button's text keeps the system's color: Windows does not let a native button change it, and a
window should behave the same everywhere. Its `size` and `bold` work.

An image comes from a file, and `.size` gives its width, the height following:

```kova
ui.image("logo.png").size(64)
```

If the file does not exist, the program stops and names it, rather than leaving a hole in the
window. A relative path starts from the folder the program was launched in.

## Text fields

A field shows a value of the state, and each change the user makes, letter by letter, becomes an
event: the block receives the text the field now holds and gives the event.

```kova
import ui

enum Event {
    Renamed(name: String)
}

struct App {
    name: String
}

fn view(app: App) -> ui.View {
    return ui.column([
        ui.input(app.name) { s: String => Event.Renamed(s.clone) },
        ui.text("Hello {app.name}"),
    ])
}

fn update(app: var App, e: Event) {
    match e {
        .Renamed(name) => app.name := name.clone
    }
}

fn main {
    ui.run("Greeting", App { name := "" }, view, update)
}
```

```text
[Greeting] column(input "", text "Hello ")
```

In [test mode](#testing-a-window), it prints the view above. In a window, "Hello …" follows the field as the user types. The block lends `s` ([Ownership](/guide/ownership/)),
so the event takes a copy: `s.clone`.

The field always shows the state. If `update` does not take a change (an age field refusing a
letter, say), the field goes back to what the state holds, so the screen never shows a value the
program does not have.

## Widgets that hold a value

A checkbox, a slider, a drop-down list, a list or a table each show one value, and that value lives
in your state, like the text of a field. The widget receives it, and a block turns what the user did
into an event:

```kova
import ui

enum Event {
    Dark(on: Bool)
    Volume(v: Int)
    Size(i: Int)
}

struct App {
    dark: Bool
    volume: Int
    size: Int
}

fn view(app: App) -> ui.View {
    return ui.column([
        ui.checkbox("Dark", app.dark, .Dark),
        ui.slider(app.volume, 0, 10, .Volume),
        ui.choice(["Small", "Medium", "Large"], app.size, .Size),
        ui.text("Volume {app.volume}"),
    ])
}

fn update(app: var App, e: Event) {
    match e {
        .Dark(on) => app.dark := on
        .Volume(v) => app.volume := v
        .Size(i) => app.size := i
    }
}

fn main {
    ui.run("Settings", App { dark := false, volume := 3, size := 0 }, view, update)
}
```

`.Volume` stands for the block `{ v: Int => Event.Volume(v) }`: where a block making an `Event` is
expected, a variant's name is enough when its fields are what the block receives. When you need to
do something first, write the block: `ui.slider(app.volume, 0, 10) { v: Int => Event.Volume(v * 2) }`.

Moving the slider changes nothing by itself on the Kova side: the event reaches `update`, the state
changes, and `view` draws the slider at its new place. So a program can refuse a value, or tie two
widgets together, simply by what `update` does.

`ui.choice`, `ui.list`, `ui.table` and `ui.tabs` work with a position counted from 0, not with the
text of the line: two lines may read the same, and a position stays right when the texts are
translated. `ui.list` and `ui.table` take `-1` for "nothing selected".

`ui.tabs` is only the row of tabs. Which page shows under it is yours to write, and that is one `if`
in `view`:

```kova
let page: ui.View := if app.tab = 0 { settings(app) } else { about(app) }
return ui.column([ui.tabs(["Settings", "About"], app.tab, .Tab), page])
```

## Menus

A menu line is a button somewhere else: it sends an event to `update`. `menus` goes on the whole
view, the outermost one:

```kova
return ui.column([content]).menus([
    ui.menu("File", [ui.item("Open", Event.Open), ui.separator, ui.item("Quit", Event.Quit)]),
])
```

On a Mac, the bar sits at the top of the screen, and always holds the application's menu (with Quit)
and an Edit menu, so that copy and paste work in text fields.

## Dialogs

`ui.open_file`, `ui.save_file` and `ui.open_folder` show the system's own window and give back the
path, or `none` when the user cancels. `ui.message` and `ui.confirm` ask something. All of them wait
for the answer, so `update` reads like a sentence:

```kova
fn update(app: var App, e: Event) {
    match e {
        .Open => if path := ui.open_file("Open a note") {
            app.path := path.clone
        }
        .Quit => if ui.confirm("Quit", "Leave without saving?") {
            ui.close
        }
    }
}
```

In `KOVA_UI_SCRIPT` mode, no window opens: each dialog prints what it would ask, then reads its
answer from the next input line.

## Drawing

Native widgets cover forms. For a game, a chart or a look of your own, `ui.canvas` is an area you
draw on yourself. It is a view like the others, so it goes in a column next to texts and buttons,
and it follows the same rule: the state says what is there, `view` draws it, `update` changes it.

```kova
import ui

enum Event {
    Pressed(x: Int, y: Int)
    Tick
}

struct App {
    x: Int
    y: Int
    ticks: Int
}

fn view(app: App) -> ui.View {
    return ui.canvas(300, 200) { d: ui.Draw =>
        d.clear(ui.white)
        d.circle(app.x, app.y) radius(20) color(ui.blue)
        d.text("{app.ticks} ticks", 10, 10) color(ui.gray)
    }
    .on_mouse { m: ui.Mouse => Event.Pressed(m.x, m.y) }
    .every(1000, Event.Tick)
}

fn update(app: var App, e: Event) {
    match e {
        .Pressed(x, y) => {
            app.x := x
            app.y := y
        }
        .Tick => app.ticks := app.ticks + 1
    }
}

fn main {
    ui.run("Dot", App { x := 150, y := 100, ticks := 0 }, view, update)
}
```

```text
[Dot] canvas 300x200 clear(#ffffff) circle(150 100 20 #2563eb 0) text(10 10 16 #6b7280 "0 ticks") every(1000)
```

The block runs each time the view is made, so a `for` or an `if` inside it draws what the state
holds. There is nothing to erase and nothing to move by hand: change the state, and the next
drawing shows it.

Positions are in pixels from the top left corner of the canvas. `d.width` and `d.height` give its
size.

`ui.fill` in place of a number gives the canvas all the room there is, so it follows the window:
`ui.canvas(ui.fill, 36)` is a bar as wide as the window, `ui.canvas(ui.fill, ui.fill)` takes
everything the other views leave. `d.width` and `d.height` then give what it really got, and it is
drawn again each time that changes.

```kova
d.clear(ui.white)
d.rect(10, 20) size(100, 50) color(ui.red)
d.rect(10, 20) size(100, 50) outline(2)
d.circle(150, 80) radius(30) color(ui.blue)
d.line(0, 0) to(100, 100) thickness(3)
d.text("Score", 10, 90) size(20) color(ui.gray)
```

`color` is black when left out. `outline(n)` draws only the edge, `n` pixels thick, inside the
shape; without it, the shape is filled. A canvas starts white.

### Mouse, keyboard, clock

Three methods make a canvas listen, each turning what happened into one of your events:

| Written | Sends its event |
| --- | --- |
| `.on_mouse { m: ui.Mouse => … }` | when a button is pressed or released, and when the mouse moves. |
| `.on_key { k: ui.Key => … }` | at each key pressed, while the canvas has the keyboard. |
| `.every(ms, event)` | every `ms` milliseconds. |

`m.x` and `m.y` are the place in the canvas. `m.pressed`, `m.released` and `m.moved` say what
happened, `m.right` that it is the right button. A move with a button held down still gives
`m.moved`, so dragging needs a line in your state:

```kova
.Mouse(x, y, pressed, released) => {
    if pressed {
        app.dragging := true
    }
    if released {
        app.dragging := false
    }
    if app.dragging {
        app.x := x
    }
}
```

`k.name` is what the key writes (`"a"`, `"A"`, `"é"`, `"1"`), or one word for the others, the same
on every system: `Left`, `Right`, `Up`, `Down`, `Space`, `Enter`, `Escape`, `Tab`, `Backspace`,
`Delete`, `Home`, `End`, `PageUp`, `PageDown`, `F1` to `F12`.

`.every` also works on any other view, to refresh a clock display for instance. For an animation,
16 milliseconds gives about 60 images a second. A program slower than its clock skips ticks rather
than falling behind, and only the last mouse move waits, but a press is never lost.

```kova
import ui

enum Event {
    Tick
    Key(name: String)
}

struct App {
    x: Int
    speed: Int
}

fn view(app: App) -> ui.View {
    return ui.canvas(300, 60) { d: ui.Draw =>
        d.circle(app.x, 30) radius(10) color(ui.red)
    }
    .on_key { k: ui.Key => Event.Key(k.name.clone) }
    .every(16, Event.Tick)
}

fn update(app: var App, e: Event) {
    match e {
        .Tick => app.x := app.x + app.speed
        .Key(name) => match name {
            "Left" => app.speed := -2
            "Right" => app.speed := 2
            _ => {}
        }
    }
}

fn main {
    ui.run("Ball", App { x := 150, speed := 0 }, view, update)
}
```

```text
[Ball] canvas 300x60 circle(150 30 10 #dc2626 0) every(16)
```

The canvas receives the keyboard when the window opens and each time it is clicked; a click in a
text field takes it back.

In [test mode](#testing-a-window), a canvas prints what it draws, and `k=press 10 20 1`,
`k=release 10 20 1`, `k=move 10 20 0` or `k=key Left` plays a gesture on listener `k`. Listeners
are counted with the text fields, in the order the view is written; for a canvas, the mouse before
the keyboard if `.on_mouse` is written first. `1` is the left button, `2` the right. Clocks are
counted with the buttons.

## Drawing with the graphics card

`ui.canvas` asks the system to draw shapes one at a time, which is simple and enough for most windows.
For thousands of shapes, for 3D, or for effects computed on every pixel, `ui.gpu` hands the work to
the graphics card. It takes more to learn, because you write two small programs for the card itself.
They are called *shaders*, and they are written in GLSL, the language of OpenGL, not in Kova:

- the **vertex shader** runs once per corner of a triangle, and says where that corner lands. The
  window goes from -1 to 1 in both directions, whatever its size in pixels, with 1 at the top.
- the **fragment shader** runs once per pixel of the triangle, and says its color. What the vertex
  shader writes in an `out` variable reaches it blended between the three corners: that is how a
  triangle goes smoothly from red to green to blue below.

```kova
import ui

let vertex_code: String := """
#version 330 core
layout(location = 0) in vec2 place;
layout(location = 1) in vec3 shade;
uniform float turn;
out vec3 kept;
void main() {
    float c = cos(turn), s = sin(turn);
    gl_Position = vec4(c * place.x - s * place.y, s * place.x + c * place.y, 0.0, 1.0);
    kept = shade;
}
"""

let fragment_code: String := """
#version 330 core
in vec3 kept;
out vec4 color;
void main() { color = vec4(kept, 1.0); }
"""

enum Event {
    Tick
}

fn view(turn: Float) -> ui.View {
    let area: ui.View := ui.gpu(400, 400) { g: ui.Gpu =>
        g.clear(ui.black)
        g.shader(vertex_code, fragment_code)
        g.uniform("turn", [turn])
        g.triangles([
            0.0, 0.8, 1.0, 0.0, 0.0,
            -0.7, -0.5, 0.0, 1.0, 0.0,
            0.7, -0.5, 0.0, 0.0, 1.0,
        ]) parts([2, 3])
    }
    return area.every(16, Event.Tick)
}

fn update(turn: var Float, e: Event) {
    turn += 0.02
}

fn main {
    ui.run("Triangle", 0.0, view, update)
}
```

The list given to `g.triangles` is flat: `parts([2, 3])` says each vertex is 2 numbers, then 3, which
the shaders read as attribute 0 (`place`) and attribute 1 (`shade`). Three vertices make one triangle.
Without `parts`, a vertex is 2 numbers.

`g.uniform` hands the shaders a value that stays the same for the whole draw, here the angle. It
takes 1 to 4 numbers (`float`, `vec2`, `vec3`, `vec4`), or 16 for a `mat4`, column by column. A 3D
program computes its matrix in Kova, and multiplies each vertex by it in the vertex shader.

The rest works like a canvas. `view` paints the whole frame again, so an animation is a clock that
changes the state. `.on_mouse`, `.on_key` and `ui.fill` work the same way. Each frame starts black,
with the depth reset, and something nearer hides something farther: for 3D, give each vertex a third
number. Colors with an alpha below 1 blend with what is behind them.

`g.texture(width, height, pixels)` gives the shaders an image, as 4 bytes per pixel (red, green, blue,
alpha), first line first. They read it through a `uniform sampler2D`, which needs no `g.uniform`.

A shader is compiled when the window first draws it. If it has a mistake, the program stops with
what the graphics driver said:

```console
error: a shader does not compile: 0:4(23): error: `oops' undeclared
```

`ui.gpu` needs OpenGL 3.3, which nearly every computer since 2010 has, and shaders start with
`#version 330 core`. Apple has stopped improving OpenGL, but every Mac still runs it. Vertices travel
as text at each frame, which holds well into the tens of thousands, and not millions.

## A window with your own frame

`frame(false)` on `ui.run` removes what the system draws around the window: title bar, buttons,
border. Everything inside is yours, so a canvas can be the title bar. Four functions, called from
`update`, replace what is gone:

| Written | Does |
| --- | --- |
| `ui.close` | closes the window; `ui.run` ends and the program goes on. |
| `ui.minimize` | sends the window to the taskbar or the Dock. |
| `ui.maximize` | makes the window as big as it can be, beside the menu bar and the Dock or the taskbar, as the Finder does, or comes back if it already is. |
| `ui.full_screen` | takes the whole screen, with nothing around, or comes back. On a Mac, this is the real full screen, as with the green button. |
| `ui.drag` | the system moves the window with the mouse, until the button is released. |

```kova
import ui

enum Event {
    Pressed(x: Int, y: Int, pressed: Bool)
    Closed
}

fn view(n: Int) -> ui.View {
    return ui.column([
        ui.canvas(300, 32) { d: ui.Draw =>
            d.clear(ui.rgb(40, 44, 52))
            d.text("My program", 10, 8) color(ui.white)
        }
        .on_mouse { m: ui.Mouse => Event.Pressed(m.x, m.y, m.pressed) },
        ui.button("Close", Event.Closed),
    ])
}

fn update(n: var Int, e: Event) {
    match e {
        .Pressed(x, y, pressed) => {
            if pressed {
                ui.drag
            }
        }
        .Closed => ui.close
    }
}

fn main {
    ui.run("Bare", 0, view, update) frame(false)
}
```

```text
[Bare] column(canvas 300x32 clear(#282c34) text(10 8 16 #ffffff "My program"), button "Close")
(no frame)
```

`ui.drag` belongs just after a press, while the button is held: it starts from that press, and does
nothing if there has been none. The release then goes to the system, so `m.released` does not come.

Kova's 16 pixels around the content go too, so a title bar reaches the window's edges. Use
`.margin` on what needs room.

Without a frame, the window cannot be resized by its edges: give it its size with `width` and
`height`, and offer `ui.maximize` if it should grow.


## Moving and resizing the window yourself

`ui.window` tells where the window is, `ui.screen` what the screen leaves it, without the menu bar,
the Dock or the taskbar. Both give a `ui.Area`, with `x`, `y`, `width` and `height`.
`ui.place(x, y) size(width, height)` puts the window there, at once.

`ui.animate` takes the window to an area over time. It is called from `update`, Kova does the rest:
`ui.animate(ui.screen) time(1000)` fills the screen in one second.

`time` is in milliseconds, 300 if you leave it out. Without a curve, the window starts slowly, goes
faster, and ends slowly, with no rebound.

The goal is any `ui.Area`: `ui.Area { x := 100, y := 100, width := 640, height := 480 }`. The program
goes on hearing clicks during the animation, and a new `ui.animate` replaces the one running.

You can give your own curve, and build any other movement with a clock and `ui.place`:
[Animations](/guide/animations/) explains how, from the first image to rebounds.

On Linux, `ui.place` does nothing yet: Wayland lets no program choose its window's place, and GTK 4
cannot resize an open window. `ui.window` gives 0 for `x` and `y`, and `ui.screen` the whole screen,
panels included.

## Testing a window

A window cannot be looked at by a test. With `KOVA_UI_SCRIPT=1`, no window opens: each view is
printed as text, and the number of the button to click (counted from 0, in the order they are
written) is read from the input, or `k=text` to type `text` into field `k` (fields are counted
apart, from 0 too); the end of the input closes the window. With no input:

```text
[Counter] column(text "0 clicks", row(button "+1", button "-1"))
closed
```

With the input `0`, then `0`, then `1`, it shows 1 click, 2, then 1 again, then closes. A test can
so drive a window program through every case, on any machine, even without a screen.

## What comes next

For a widget `import ui` does not have yet, a program can call the system's own library through
[`extern`](/guide/calling-c/).
