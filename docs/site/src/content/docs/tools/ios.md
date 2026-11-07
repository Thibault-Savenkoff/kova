---
title: Kova on iOS
description: kova build --target ios-simulator makes an app for Xcode's iPhone and iPad simulator.
sidebar:
  order: 4
---

A Kova program can be an iPhone and iPad app. For now it runs in Xcode's **simulator**, on a Mac
with an Apple chip; a real iPhone comes later (Apple only runs apps signed with its own
certificate).

```console
$ kova build counter.kova --target ios-simulator
✓ counter.app (iOS Simulator, 180 KB) in 4.5 s
```

`counter.app` is a folder: your program and the small file that names it. Kova needs neither
Xcode's compiler nor Swift to make it. On the Mac, with Xcode installed:

```console
$ xcrun simctl boot "iPhone 17"
$ xcrun simctl install booted counter.app
$ xcrun simctl launch --console-pty booted dev.kova.counter
```

The last word is the app's name in lower case, after `dev.kova.`. `--console-pty` shows what the
program prints.

## What a program finds on an iPhone

| In the program | On the iPhone |
|---|---|
| `print` | the console of `simctl launch --console-pty` |
| `import ui` | real UIKit views: texts, buttons, fields, switches, sliders, choices, images, lists |
| a window's column or row | a stack view, with its margins and styles |

Not there yet: `ui.canvas`, `ui.gpu`, menus, dialogs and sound. A canvas shows a line of text
where it would be.

## Trying a program without a finger

The simulator's command line cannot touch the screen. For tests, `KOVA_UI_TAP=0,0,1` makes the app
tap its own buttons by number, one for each time the window is drawn:

```console
$ SIMCTL_CHILD_KOVA_UI_TAP=0,0,1 xcrun simctl launch booted dev.kova.counter
```
