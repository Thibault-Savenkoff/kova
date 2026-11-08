---
title: Kova on iOS
description: kova build --target ios-simulator makes an app for Xcode's iPhone and iPad simulator.
sidebar:
  order: 4
---

A Kova program can be an iPhone and iPad app. It runs in Xcode's **simulator** on a Mac with an Apple
chip, or on a **real iPhone** once you have signed it with your own Apple certificate (below).

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

A window scrolls when it is taller than the screen, `ui.canvas` draws with Core Graphics and hears
touches, dialogs are alerts and sound goes through `AVAudioPlayer`.

Menus are a row of buttons above the window, each opening its items (shortcuts are for a keyboard, which
an iPhone has not). A canvas wider than the screen scrolls sideways.

Not there yet: `ui.gpu`.

## On a real iPhone

Apple only runs apps signed with a certificate it made for you. A free Apple ID is enough.

### On a Mac: nothing to give Kova

Add your Apple ID in Xcode once (Settings → Accounts), plug the iPhone in with Developer Mode on
(Settings → Privacy & Security), then:

```console
$ kova build counter.kova --target ios --run
```

Kova asks Xcode for the profile of `dev.kova.counter` (it makes one the first time), signs the app
with your certificate from the keychain, installs it on the iPhone and starts it, showing what the
program prints. Run it from a Terminal on the Mac: the first time, macOS asks to let `codesign` use
your key (choose Always Allow). Without `--run`, you get the signed `counter.app` and install it
yourself.

The iPhone may be on the same Wi-Fi instead of on a cable: in Xcode, Window → Devices and
Simulators, select it once and tick Connect via network.

### On any computer

Kova signs by itself with a profile and an identity file, on Linux, Windows or Mac. What Apple needs,
you do **once, on a Mac**:

1. Plug the iPhone in, and turn on Settings → Privacy & Security → Developer Mode.
2. In Xcode, add your Apple ID (Settings → Accounts) and make an empty iOS App project whose
   identifier is the one your program will have: `dev.kova.` and its name in lower case (a program
   with `header { name := "essai" }` is `dev.kova.essai`). Choose your account under Signing &
   Capabilities and run it once on the iPhone. Xcode makes the certificate and the profile.
3. On the iPhone, trust yourself in Settings → General → VPN & Device Management.
4. Give Kova the two things Xcode made. The profile is the `.mobileprovision` file in
   `~/Library/Developer/Xcode/UserData/Provisioning Profiles`. The identity is your key and
   certificate as a PEM file, from the Keychain Access export (`.p12`):

```console
$ openssl pkcs12 -legacy -in identity.p12 -nodes -out ios.pem
```

Then, on any computer:

```console
$ kova build counter.kova --target ios --profile app.mobileprovision --identity ios.pem
✓ counter.app (iOS, 230 KB) in 6 s
```

Install it from a Mac with `xcrun devicectl device install app --device <name> counter.app`.

Kova checks that the profile allows the program's name (`dev.kova.counter`) and has not expired.
**A free profile lasts 7 days**: after that the app stops opening and you build it again with a
new profile. A paid developer account (99 $ a year) lasts a year, and its profile may allow every
name starting with `dev.kova.`. Keep `ios.pem` private: it is the key that signs your apps.

## Trying a program without a finger

The simulator's command line cannot touch the screen. For tests, `KOVA_UI_TAP=0,0,1` makes the app
tap its own buttons by number, one for each time the window is drawn:

```console
$ SIMCTL_CHILD_KOVA_UI_TAP=0,0,1 xcrun simctl launch booted dev.kova.counter
```
