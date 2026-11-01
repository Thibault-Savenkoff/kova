---
title: Kova on Android
description: kova build --target android makes one .apk file, with nothing to install.
sidebar:
  order: 3
---

A Kova program can be an Android app. Kova needs neither Android's tools nor Java:

```console
$ kova build counter.kova --target android
✓ counter.apk (Android, 391 KB) in 1.4 s
```

`counter.apk` holds your program compiled for phones (arm64) and for emulators and Chromebooks
(x86-64), and it is signed. It runs on Android 8.0 and later.

| Target | What the `.apk` holds |
|---|---|
| `android` | arm64 and x86-64 |
| `android-arm` | arm64 only: smaller, every recent phone |
| `android-intel` | x86-64 only: emulators, some Chromebooks |

## Putting it on a phone

Android only installs apps from outside its store if you allow it:

1. Send `counter.apk` to the phone: by cable, by mail, through a shared folder.
2. Open the file on the phone.
3. Android asks whether this source may install apps: allow it, then install.

If you have Android's `adb` tool, `adb install counter.apk` does the same.

## What a program finds on a phone

| In the program | On the phone |
|---|---|
| `print` | a text screen; also `adb logcat -s kova` |
| `read_line` | a field appears under the text |
| `import ui` | real Android views: buttons, fields, check boxes, sliders, lists |
| `ui.canvas` | a view drawn by the program; `on_mouse` receives the finger |
| `ui.gpu` | OpenGL ES 3: shaders written for OpenGL 3.3 are adapted when they compile |
| `view.menus` | the title bar's menu |
| `ui.message`, `ui.confirm` | Android's own dialogs |
| `ui.open_file` | Android's file picker; the program receives a copy |
| `import sound` | the phone plays the WAV |
| `web.get`, `web.post`, `web.send` | Android's own connections, so HTTPS uses its certificates |
| `import files` | the app's own private folder, which is also the working folder |

The program is real machine code: threads, `web.serve` and `net` work as on Linux. The program's
language is the phone's, unless the `header` says otherwise. The sizes you write are density
independent pixels, so a view looks the same on any screen.

`header { name := "Counter" }` names the app and the file, and `icon := "counter.svg"` gives it its
icon, as on the other systems. Without a name, the app takes its file's, or its folder's for a
`main.kova`. Android tells apps apart by that name: two apps with the same one replace each other.

## How it behaves

An app has no end of its own: Android runs it, and the user leaves it with the back gesture.

A program without a window keeps its text on the screen when it ends, until the user leaves. A
program with a window closes when `ui.run` returns; if it stops on an error, the error stays on
the screen. Leaving a window makes `ui.run` return, as closing a window does on a computer.

## Why it needs no tools

An `.apk` is a zip file. Kova writes everything in it itself: the program as a library, the app's
description in Android's binary form, and the signature. The Java part of an app, which turns a
Kova view into Android views, is already compiled inside `kova`.

## Your key

Android refuses to update an app signed by another key. The first `--target android` build makes
your key, in `~/.config/kova/android.key` (`%APPDATA%\kova\android.key` on Windows):

- keep it, or your phones will refuse updates to your apps until they are uninstalled;
- never share it, or put it in a project;
- to use it on another machine, copy that file there, or point `KOVA_ANDROID_KEY` at it.

## What is not there yet

- **The Play Store** wants another package format and version numbers: for now, apps are installed
  from the file. An app is always version 1.0, named `dev.kova.app.<name>`.
- **`ui.image` and `d.image`** read files the program can reach: nothing is packed into the app yet.
- `ui.save_file` gives a path in the app's folder that the Files app can open.
  `ui.open_folder` answers `none`.
- `net` has no TLS here: its secure connections fail, `web.get` does not.
- `ui.place` and `ui.animate` of the window do nothing: an app fills its screen.
- `extern` reaches only the libraries Android lets apps load.
