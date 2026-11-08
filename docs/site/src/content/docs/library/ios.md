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

## What to know

- Every message is sent on the main thread, as UIKit wants, so the program may go on at its own pace.
- An object you make stays alive until you `release` it; the objects a message gives back are kept for you
  too. A program that makes thousands of views should release the ones it drops.
- An Objective-C exception (a wrong selector for the object, a view added twice) is not caught: the app stops,
  as it would in Swift. Kova checks that the object answers the selector and that the number of arguments is
  right before it sends.
- `ui` and `ios` work together: `ui.run` builds its views in the same window, and `ios.root` reaches them.
