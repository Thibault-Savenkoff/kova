---
title: Animations
description: How an animation works, how to build your own curves, and how to move anything over time.
sidebar:
  order: 18
---

An animation is a series of still images, shown fast enough for the eye to see movement. Kova has no
hidden animation engine: a program changes a number over time and draws again. This chapter builds
that from nothing, then shows how to make any movement. Everything here is ordinary Kova, so
everything can be changed.

Read [Windows](/guide/windows/) first: it explains the state, `view` and `update`.

## The three questions

Every animation answers the same three questions:

1. **How long?** A number of steps, or a time in milliseconds.
2. **From where to where?** Two values: a start and a goal.
3. **How does it get there?** At a steady speed, slowing down, rebounding: this is the **curve**.

They come together in one calculation, done at each image:

```text
value = start + (goal - start) * progress
```

With `progress` at `0.0`, the value is the start. At `1.0`, it is the goal. At `0.5`, it is halfway.
The whole chapter is about choosing `progress`.

## A first animation, by hand

This ball crosses a canvas when you click. The state is one number, the current step:

```kova
import ui

enum Event {
    Start
    Tick
}

let steps: Int := 60

fn view(step: Int) -> ui.View {
    let t: Float := step.to_float / steps.to_float
    let x: Int := 40 + (320.0 * t).to_int
    return ui.column([
        ui.button("Go", Event.Start),
        ui.canvas(400, 100) { d: ui.Draw =>
            d.clear(ui.white)
            d.circle(x, 50) radius(20) color(ui.blue)
        },
    ]).every(16, Event.Tick)
}

fn update(step: var Int, e: Event) {
    match e {
        Start => step := 1
        Tick => {
            if step > 0 && step < steps {
                step := step + 1
            }
        }
    }
}

fn main {
    ui.run("Ball", 0, view, update)
}
```

```text
[Ball] column(button "Go", canvas 400x100 clear(#ffffff) circle(40 50 20 #2563eb 0)) every(16)
```

Here is what happens:

1. `.every(16, Event.Tick)` sends `Tick` every 16 milliseconds, about 60 times a second.
2. Each `Tick` adds 1 to `step`, until it reaches `steps`.
3. Each change of the state rebuilds the view, as always.
4. `view` turns the step into `t`, from `0.0` to `1.0`: `step.to_float / steps.to_float`.
5. `40 + 320.0 * t` is the calculation above: the start is 40, the goal 360.

So 60 steps of 16 milliseconds last about a second. To change the duration, change `steps`.

The ball moves at a steady speed, because `t` is used directly as the progress. That looks mechanical:
nothing in the real world starts and stops at full speed. A curve fixes that.

## What a curve is

A curve is a function that receives `t`, the **time** spent, and gives the **progress**. Both usually
go from `0.0` to `1.0`. To use one in the ball, replace `320.0 * t` with `320.0 * curve(t)`.

Before putting a curve in a window, look at it. This program needs no window:

```kova
fn curve(t: Float) -> Float {
    return t * t
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t} -> {curve(t)}")
    }
}
```

```text
0.0 -> 0.0
0.25 -> 0.0625
0.5 -> 0.25
0.75 -> 0.5625
1.0 -> 1.0
```

Read the table as: a quarter of the time gone, the ball has done `0.0625` of the way. Half the time
gone, only a quarter of the way. It starts slowly, then speeds up. The reason is simple: multiplying
a number below 1 by itself makes it smaller, and the smaller it is, the more it shrinks.

More multiplications give a stronger effect: `t * t * t`, also written `t.pow(3)`, starts more
slowly and ends faster.

## Building curves from others

### Reversing a curve

To slow down at the end instead, play the same movement backwards. Replace `t` with `1.0 - t` to
reverse the time, and take the result away from `1.0` to reverse the distance:

```kova
fn speed_up(t: Float) -> Float {
    return t * t
}

fn slow_down(t: Float) -> Float {
    return 1.0 - speed_up(1.0 - t)
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t} -> {slow_down(t)}")
    }
}
```

```text
0.0 -> 0.0
0.25 -> 0.4375
0.5 -> 0.75
0.75 -> 0.9375
1.0 -> 1.0
```

This works with any curve: `1.0 - c(1.0 - t)` is `c` played backwards.

### Joining two curves

For a start and an end that are both slow, speed up in the first half and slow down in the second.
Each half is squeezed into half of the time and half of the way:

```kova
fn speed_up(t: Float) -> Float {
    return t * t
}

fn slow_down(t: Float) -> Float {
    return 1.0 - speed_up(1.0 - t)
}

fn smooth(t: Float) -> Float {
    if t < 0.5 {
        return speed_up(t * 2.0) / 2.0
    }
    return 0.5 + slow_down(t * 2.0 - 1.0) / 2.0
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t} -> {smooth(t)}")
    }
}
```

```text
0.0 -> 0.0
0.25 -> 0.125
0.5 -> 0.5
0.75 -> 0.875
1.0 -> 1.0
```

`t * 2.0` makes the first half go from `0.0` to `1.0`, and `/ 2.0` brings the result back to the
first half of the way. `ui.animate` uses a curve of this kind by default.

## Rebounds

### Going past the goal

The progress is not limited to `1.0`. At `1.1`, the calculation gives a value 10 % beyond the goal.
So a curve that climbs above `1.0` before coming back makes the window or the ball go too far, then
return:

```kova
fn past(t: Float, strength: Float) -> Float {
    let back: Float := t - 1.0
    return 1.0 + (strength + 1.0) * back.pow(3) + strength * back.pow(2)
}

fn main {
    for strength in [1.0, 1.7, 3.0] {
        var highest: Float := 0.0
        for i in 0..=100 {
            highest := highest.max(past(i.to_float / 100.0, strength))
        }
        print("strength {strength}: goes up to {highest}")
    }
}
```

```text
strength 1.0: goes up to 1.037026
strength 1.7: goes up to 1.0998424
strength 3.0: goes up to 1.25
```

`back` goes from `-1.0` to `0.0`, so the curve starts at `0.0` and ends at `1.0`. `strength` decides
how far it goes: above, `1.7` gives about 10 %, and `3.0` gives 25 %.

Going past the goal cannot be seen when the goal is the whole screen, because the window leaves the
screen. For that case, a rebound must stay on the inside.

### Staying inside

A ball falling on the floor touches it, comes up, touches it again, and each bounce is smaller. Three
ingredients make that curve:

1. `(t * Float.pi * 3.0).cos` is a wave, going up and down.
2. `.abs` turns the wave's lower half upwards: the result is a series of arcs, between `0.0` and
   `1.0`.
3. `(1.0 - t).pow(3)` goes from `1.0` to `0.0`: it makes each arc smaller than the last.

```kova
fn arcs(t: Float) -> Float {
    return (t * Float.pi * 3.0).cos.abs
}

fn fade(t: Float) -> Float {
    return (1.0 - t).pow(3)
}

fn bounce(t: Float) -> Float {
    return 1.0 - arcs(t) * fade(t)
}

fn main {
    for i in 0..=6 {
        let t: Float := i.to_float / 6.0
        print("{bounce(t)}")
    }
}
```

```text
0.0
1.0
0.7037037037037036
1.0
0.9629629629629629
1.0
1.0
```

At each point where the arcs reach `0.0`, the curve gives `1.0`: the goal is touched. Between those
points, it falls back. Since `arcs(t) * fade(t)` is never negative, the curve never goes above `1.0`.

Each number is a setting:

| To change | Change |
| --- | --- |
| how many times the goal is touched | `3.0`: the number of arcs |
| how fast the rebounds die | the `3` of `pow(3)`: higher dies faster |
| how long it all lasts | the time given to the animation |

Without `.abs`, the wave keeps both its halves, and the curve goes on both sides of the goal, like a
spring:

```kova
fn spring(t: Float) -> Float {
    return 1.0 - (t * Float.pi * 5.0).cos * (1.0 - t).pow(3)
}

fn main {
    for i in 0..=5 {
        print("{spring(i.to_float / 5.0)}")
    }
}
```

```text
0.0
1.512
0.784
1.064
0.992
1.0
```

Take time to play with these numbers: a table printed by a small program shows at once what a change
does, before any window opens.

## Moving the window

`ui.animate` asks the same three questions: `time` for how long, the window's place and a
`ui.Area` for from where to where, and `curve` for how. It does the steps and the clock, and rebuilds
the view at each image, so a `ui.fill` canvas follows the window.

`curve` takes a block, or the name of a function. So a curve you built above is used in one line:

```kova
import ui

enum Event {
    Grow
}

fn bounce(t: Float) -> Float {
    let arcs: Float := (t * Float.pi * 3.0).cos.abs
    return 1.0 - arcs * (1.0 - t).pow(3)
}

fn view(n: Int) -> ui.View {
    return ui.button("Grow", Event.Grow)
}

fn update(n: var Int, e: Event) {
    match e {
        Grow => ui.animate(ui.screen) time(1200) curve(bounce)
    }
}

fn main {
    ui.run("Grow", 0, view, update)
}
```

```text
[Grow] button "Grow"
```

The goal is any `ui.Area`, so the area is where the design happens. A size kept in the center of the
screen:

```kova
fn centered(width: Int, height: Int) -> ui.Area {
    let s: ui.Area := ui.screen
    return ui.Area { x := s.x + (s.width - width) / 2, y := s.y + (s.height - height) / 2, width, height }
}
```

A slide to the right, with the window's size unchanged:

```kova
let w: ui.Area := ui.window
ui.animate(ui.Area { x := w.x + 300, y := w.y, width := w.width, height := w.height }) time(500)
```

The program goes on hearing clicks during an animation, and a new `ui.animate` replaces the one
running, starting from where the window is. That makes a button that grows and shrinks simple: it
only has to choose its goal.

On Linux, `ui.animate` does nothing yet, like `ui.place`: see
[Moving and resizing the window yourself](/guide/windows/#moving-and-resizing-the-window-yourself).

## When `ui.animate` is not enough

`ui.animate` moves `x`, `y`, `width` and `height` together, in a straight line, with one curve. For
anything else, the program does each image itself with a clock, as for the ball.

### Doing things one after the other

The trick is to cut the time into parts. `part` gives the progress of one piece of the animation:
`0.0` before its beginning, `1.0` after its end:

```kova
fn part(t: Float, begin: Float, end: Float) -> Float {
    return ((t - begin) / (end - begin)).max(0.0).min(1.0)
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t}: first {part(t, 0.0, 0.5)}, then {part(t, 0.5, 1.0)}")
    }
}
```

```text
0.0: first 0.0, then 0.0
0.25: first 0.5, then 0.0
0.5: first 1.0, then 0.0
0.75: first 1.0, then 0.5
1.0: first 1.0, then 1.0
```

This window grows in width during the first half, then in height during the second:

```kova
import ui

enum Event {
    Grow
    Tick
}

let steps: Int := 60

fn part(t: Float, begin: Float, end: Float) -> Float {
    return ((t - begin) / (end - begin)).max(0.0).min(1.0)
}

fn between(a: Int, b: Int, e: Float) -> Int {
    return a + ((b - a).to_float * e).to_int
}

fn view(step: Int) -> ui.View {
    return ui.button("Grow", Event.Grow).every(16, Event.Tick)
}

fn update(step: var Int, e: Event) {
    match e {
        Grow => step := 1
        Tick => {
            if step > 0 && step <= steps {
                let t: Float := step.to_float / steps.to_float
                let s: ui.Area := ui.screen
                let width: Int := between(320, s.width, part(t, 0.0, 0.5))
                let height: Int := between(200, s.height, part(t, 0.5, 1.0))
                ui.place(s.x, s.y) size(width, height)
                step := step + 1
            }
        }
    }
}

fn main {
    ui.run("Grow", 0, view, update)
}
```

```text
[Grow] button "Grow" every(16)
```

Parts may overlap: `part(t, 0.0, 0.7)` and `part(t, 0.3, 1.0)` give a width that starts first and a
height that ends last. And each part may go through its own curve: `smooth(part(t, 0.0, 0.5))`.

### A path that is not a line

Each coordinate has its own calculation. Here `x` crosses the screen at a steady speed while `y`
follows a sine, which rises and falls once between `0.0` and `Float.pi`. The window draws an arch:

```kova
let t: Float := step.to_float / steps.to_float
let s: ui.Area := ui.screen
let x: Int := s.x + ((s.width - 400).to_float * t).to_int
let y: Int := s.y + 300 - (300.0 * (t * Float.pi).sin).to_int
ui.place(x, y) size(400, 300)
```

### Animating something else

`progress` can drive any number: a size, a thickness, a position in a canvas, or a color, by moving
each of its three parts:

```kova
fn between(a: Int, b: Int, e: Float) -> Int {
    return a + ((b - a).to_float * e).to_int
}

fn mixed(e: Float) -> ui.Color {
    let safe: Float := e.max(0.0).min(1.0)
    return ui.rgb(between(37, 220, safe), between(99, 38, safe), between(235, 38, safe))
}
```

`e.max(0.0).min(1.0)` is there because a rebound may give more than `1.0`, and `ui.rgb` stops the
program on a value above 255. A position may go past its goal; a color may not.

### Stopping the clock

A clock that keeps ticking rebuilds the view 60 times a second for nothing. Ask for it only during
the animation:

```kova
fn view(step: Int) -> ui.View {
    let page: ui.View := ui.button("Go", Event.Start)
    if step > 0 && step < steps {
        return page.every(16, Event.Tick)
    }
    return page
}
```

### Exact durations

Counting steps supposes each image takes 16 milliseconds. A view that takes longer to build makes the
animation longer, without skipping an image. `ui.animate` behaves the same way.

When the duration matters more than each image, measure it. `now` gives the seconds since 1970, with
a fraction. Keep the start in the state, and compute `t` from the time really spent:

```kova
struct App {
    started: Float
    running: Bool
}

fn update(app: var App, e: Event) {
    match e {
        Start => {
            app.started := now
            app.running := true
        }
        Tick => {
            if app.running {
                let t: Float := ((now - app.started) / 0.8).min(1.0)
                ui.place(100, 100) size(between(320, 800, smooth(t)), 400)
                app.running := t < 1.0
            }
        }
    }
}
```

`.min(1.0)` stops `t` at the end, and this animation lasts 0.8 seconds however slow the computer is:
a slower one shows fewer images.

## Several things at once

Each object needs its own timing, not its own clock. One `t` drives everything, and `part` gives each
object its piece of it. Here five balls leave one after the other. Each starts 0.1 later than the
last, and takes 0.6 of the time:

```kova
import ui

enum Event {
    Start
    Tick
}

let steps: Int := 90

fn part(t: Float, begin: Float, end: Float) -> Float {
    return ((t - begin) / (end - begin)).max(0.0).min(1.0)
}

fn slow_down(t: Float) -> Float {
    return t * (2.0 - t)
}

fn view(step: Int) -> ui.View {
    let t: Float := step.to_float / steps.to_float
    return ui.column([
        ui.button("Go", Event.Start),
        ui.canvas(400, 220) { d: ui.Draw =>
            d.clear(ui.white)
            for i in 0..5 {
                let begin: Float := i.to_float * 0.1
                let e: Float := slow_down(part(t, begin, begin + 0.6))
                d.circle(40 + (320.0 * e).to_int, 30 + i * 40) radius(15) color(ui.blue)
            }
        },
    ]).every(16, Event.Tick)
}

fn update(step: var Int, e: Event) {
    match e {
        Start => step := 1
        Tick => {
            if step > 0 && step < steps {
                step := step + 1
            }
        }
    }
}

fn main {
    ui.run("Balls", 0, view, update)
}
```

```text
[Balls] column(button "Go", canvas 400x220 clear(#ffffff) circle(40 30 15 #2563eb 0) circle(40 70 15 #2563eb 0) circle(40 110 15 #2563eb 0) circle(40 150 15 #2563eb 0) circle(40 190 15 #2563eb 0)) every(16)
```

Changing `0.1` changes the delay between the balls, and `0.6` how long each one takes. Check that the
last one ends in time: it begins at `4 * 0.1`, and `0.4 + 0.6` is `1.0`. A sum above `1.0` means the
last ball is cut off before it arrives.

## Movement without a curve: physics

A curve knows the whole movement in advance. That is not what you want for something that reacts: a
ball that can be thrown again in mid-air, or that hits a wall. Physics works differently: the state
holds where the object is **and how fast it goes**, and each image computes the next one from those.

Two lines are enough, always in this order:

1. The speed changes: `speed := speed + gravity`.
2. The position changes: `position := position + speed`.

This ball falls, and loses part of its speed each time it hits the floor:

```kova
import ui

enum Event {
    Drop
    Tick
}

struct Ball {
    y: Float
    speed: Float
}

let floor_at: Float := 170.0
let gravity: Float := 0.5
let kept: Float := 0.7

fn view(ball: Ball) -> ui.View {
    return ui.column([
        ui.button("Drop", Event.Drop),
        ui.canvas(200, 200) { d: ui.Draw =>
            d.clear(ui.white)
            d.circle(100, ball.y.to_int + 15) radius(15) color(ui.red)
        },
    ]).every(16, Event.Tick)
}

fn update(ball: var Ball, e: Event) {
    match e {
        Drop => {
            ball.y := 0.0
            ball.speed := 0.0
        }
        Tick => {
            ball.speed := ball.speed + gravity
            ball.y := ball.y + ball.speed
            if ball.y > floor_at {
                ball.y := floor_at
                ball.speed := -ball.speed * kept
            }
        }
    }
}

fn main {
    ui.run("Ball", Ball { y := floor_at, speed := 0.0 }, view, update)
}
```

```text
[Ball] column(button "Drop", canvas 200x200 clear(#ffffff) circle(100 185 15 #dc2626 0)) every(16)
```

`gravity` is how much the speed grows at each image. `kept` is what is left after the floor: `1.0`
bounces forever, `0.0` does not bounce at all. Friction is one more line, `speed := speed * 0.98`, and
a spring is a speed pulled towards the goal, `speed := speed + (goal - position) * 0.1`, with friction
to calm it.

Nothing here depends on the time left: pressing `Drop` during a bounce simply starts again from the
state. Physics has no duration, though. The movement ends when the speed has died, so it is tuned by
trying, and the same small program without a window shows it:

```kova
fn main {
    var y: Float := 0.0
    var speed: Float := 0.0
    var highest: Float := 170.0
    var going_up: Bool := false
    for frame in 0..400 {
        speed := speed + 0.5
        y := y + speed
        if y > 170.0 {
            y := 170.0
            speed := -speed * 0.7
            going_up := true
        }
        if going_up && speed > 0.0 {
            print("frame {frame}: rises to {170.0 - y}")
            going_up := false
        }
    }
}
```

```text
frame 44: rises to 77.89999999999989
frame 74: rises to 35.48999999999987
frame 94: rises to 13.850999999999885
frame 107: rises to 6.136199999999917
frame 116: rises to 2.6364399999999932
frame 122: rises to 0.34086899999996945
```

## Building a complex animation

Do not start from the code. A complex animation is a series of simple ones, so build it in this
order:

1. **Write the timeline in words.** "The menu opens in 0.4 s; from 0.2 s, each line slides in, one
   every 0.05 s; the last one rebounds." A sentence you cannot write is a movement you have not decided.
2. **Turn each moment into a part of `t`.** Divide the times by the total. With 1 second in all, the
   menu is `part(t, 0.0, 0.4)`, the first line `part(t, 0.2, 0.5)`.
3. **Choose what each part moves**, with `value = start + (goal - start) * progress`, one line each.
4. **Choose a curve for each part**, starting with none. Get the timing right at a steady speed first:
   curves hide timing mistakes.
5. **Slow it all down to look at it.** Multiply `steps` by 5 while you work: what happens in 16
   milliseconds cannot be judged.

To keep it readable, put each object's calculation in a function of `t`, such as
`fn menu_height(t: Float) -> Int`. `view` then only draws, and each function can be printed as a table
without opening a window. A state with an `enum` for the phase (`Closed`, `Opening`, `Open`, `Closing`)
tells which animation runs and which clicks count: a click on an opening menu can start `Closing` from
where the menu is, if the state keeps the current value as the next start.

For a movement that depends on what happens, a click or a collision, use physics. For one that must
last exactly a given time and end exactly in a given place, use a curve. Both can live in the same
program: each object picks its own.

## In short

- `value = start + (goal - start) * progress`, at each image.
- A curve turns the time spent into progress. Print its values to see it.
- `1.0 - c(1.0 - t)` reverses a curve. Dividing the time joins two of them.
- Above `1.0`, the value goes past its goal. `cos`, `abs` and a fade make rebounds that stay inside.
- `ui.animate` for a window's straight move, with your curve. A clock and `ui.place` for all the rest.
