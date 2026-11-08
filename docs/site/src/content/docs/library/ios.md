---
title: ios
description: All of UIKit on an iPhone or iPad, by sending messages to native objects.
sidebar:
  order: 26
---

```kova
import ios
```

`ui` gives you the controls most programs need, drawn by the system. `ios` is for the rest: every class
of every framework of an iPhone or iPad, reached the way Smalltalk reaches objects. You name a class,
you send messages by their selector, and what comes back is another object. Nothing is wrapped in
advance, so nothing is missing.

It works on an iPhone or an iPad only (`kova build f.kova --target ios`). A program that imports it and runs
anywhere else stops with a message saying so.

## Objects and messages

```kova
import ios

fn main {
    let view: ios.Object := ios.root.child("view")
    let label: ios.Object := ios.new("UILabel")
    label.send("setText:", [ios.text("Hello from Kova")])
    label.send("setFrame:", [ios.rect(20.0, 200.0, 300.0, 40.0)])
    view.send("addSubview:", [label.value])
}
```

The selector is the one Apple's documentation gives, colons included. Each argument is written for what
the selector wants, and Kova reads the selector's types to hand it over: an `Int` becomes whichever
integer or Boolean it asks for, a `Float` a `CGFloat`, a text an `NSString`.

| Function | What it does |
|---|---|
| `ios.class(name)` | the class called `name`, or `none` |
| `ios.get(name)` | the class itself, to send it class messages (it stops the program if there is none) |
| `ios.new(name)` | a new object of the class `name` (it stops the program if there is none) |
| `ios.root` | the window's view controller; `root.child("view")` is the screen |
| `o.send(selector, args)` | sends a message, gives back a `Value` |
| `o.call(selector)` | the same without arguments |
| `o.child(selector)` | the same, for a message that gives back an object |
| `o.value` | the object written as an argument |
| `o.release` | lets the object go |
| `ios.text(s)`, `ios.number(n)`, `ios.decimal(x)`, `ios.flag(b)`, `ios.selector(name)` | arguments |
| `ios.rect(x, y, width, height)` | a `CGRect`; `.Numbers([…])` writes a point, a size or edge insets |
| `ios.handler(n)` | an object that tells you when it is called |
| `ios.wait` | waits for a handler to be called and gives its number |

A `Value` is `.Nothing`, `.Int`, `.Float`, `.Bool`, `.Text`, `.Object`, `.Selector` or `.Numbers`. What a
message gives back is read from its signature, and `.object`, `.int` and `.text` stop the program with a
clear message if it gave something else.

## Hearing a control

A control calls a target and an action when something happens. `ios.handler(7)` is a target that does one
thing: it makes `ios.wait` give 7.

```kova
let tap: ios.Object := ios.handler(7)
button.send("addTarget:action:forControlEvents:", [tap.value, ios.selector("invoke:"), ios.number(64)])
while true {
    let n: Int := ios.wait
    // n is 7: the button was touched
}
```

## Views you can read

Sending messages by selector reaches everything, but the usual views come with methods you can read. Each
one holds the native object it stands for (`.object`), so whatever a method does not cover is still one
`send` away.

```kova
import ios

fn main {
    let screen: ios.View := ios.screen
    screen.set_background(ios.Color.white)

    let title: ios.Label := ios.Label.new("Hello")
    title.set_size(28.0)
    title.set_color(ios.Color.blue)
    title.view.set_frame(20.0, 80.0, 300.0, 40.0)
    screen.add(title.view)

    let button: ios.Button := ios.Button.new("Greet")
    button.view.set_frame(20.0, 140.0, 120.0, 44.0)
    button.on_tap(1)
    screen.add(button.view)

    while true {
        if ios.wait = 1 {
            title.set_text("Tapped")
        }
    }
}
```

| Type | Made with | Methods |
|---|---|---|
| `ios.View` | `ios.View.new`, `ios.screen`, or `.view` of any type below | `set_frame(x, y, width, height)`, `set_background(color)`, `set_alpha(a)`, `set_hidden(b)`, `set_corner(radius)`, `add(view)`, `remove`, `width`, `height`, `release` |
| `ios.Label` | `ios.Label.new(words)` | `set_text`, `text`, `set_color`, `set_size(points)`, `set_centered(b)`, `set_lines(n)`, `view` |
| `ios.Button` | `ios.Button.new(title)` | `set_title`, `on_tap(n)`, `view` |
| `ios.TextField` | `ios.TextField.new(hint)` | `text`, `set_text`, `on_change(n)`, `view` |
| `ios.Switch` | `ios.Switch.new` | `is_on`, `set_on(b)`, `on_change(n)`, `view` |
| `ios.Color` | `ios.Color.rgb(r, g, b)` (each 0.0 to 1.0), `blue`, `red`, `green`, `gray`, `white`, `black`, `clear` | given to `set_background`, `set_color` |

`on_tap(n)`, like `on_change(n)`, makes `ios.wait` give `n` each time it happens.

## What to know

- Every message is sent on the main thread, as UIKit wants, so the program may go on at its own pace.
- An object you make stays alive until you `release` it; the objects a message gives back are kept for you
  too. A program that makes thousands of views should release the ones it drops.
- An Objective-C exception (a wrong selector for the object, a view added twice) is not caught: the app prints `error: NSInvalidArgumentException: Can't add self as subview` and stops
  with code 101, as it would in Swift. Kova checks that the object answers the selector and that the number of arguments is
  right before it sends.
- `ui` and `ios` work together: `ui.run` builds its views in the same window, and `ios.root` reaches them.
