---
title: Un programme, tous les systèmes
description: Le même fichier devient une appli pour Linux, Windows, macOS, Android et iPhone, avec l'allure du système.
sidebar:
  order: 19
---

Un programme écrit avec [`import ui`](/fr/guide/windows/) n'est pas attaché à un ordinateur. Le même
fichier peut être construit pour un téléphone, et il garde sa forme : un état, une `view`, un
`update`. Kova le dessine avec les vues du système pour lequel il est construit : chaque appli
ressemble donc à celles qui l'entourent.

## Un fichier, une commande par système

```console
$ kova build clock.kova                          # this computer
$ kova build clock.kova --target android         # an .apk for phones
$ kova build clock.kova --target ios-simulator   # an app for Xcode's simulator
$ kova build clock.kova --target web             # a page for a browser
```

Rien ne change dans le fichier entre ces lignes. [Kova sur Android](/fr/tools/android/) et
[Kova sur iOS](/fr/tools/ios/) disent ce que chaque système demande pour installer le résultat.

## Savoir où l'on tourne

La plus grande part d'un programme s'en moque. Quand un morceau s'en soucie, `system.os` dit pour
quel système le programme est construit : `"linux"`, `"windows"`, `"mac"`, `"android"`, `"ios"` ou
`"web"`. C'est connu à la construction : une version pour téléphone ne porte donc jamais la branche
de l'ordinateur.

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

Avec `kova run`, `system.os` est le système sur lequel tu es.

## Ce qui s'adapte tout seul

Tu n'écris rien pour cela :

- **Les vues sont celles du système.** Un bouton est un vrai bouton : GTK, Win32, AppKit, UIKit, les
  vues d'Android. Les champs de texte, les cases à cocher et les listes se comportent comme dans les
  autres applis de l'utilisateur.
- **Les couleurs suivent le système.** Sur Android, l'appli prend les couleurs de Material 3, tirées
  du fond d'écran à partir d'Android 12, et elle passe du clair au sombre *pendant qu'elle est
  ouverte*, dès que l'utilisateur change. Sur iPhone, les couleurs du système font de même.
- **Les tailles sont indépendantes de la densité.** `size(20)` ou `margin(8)` donnent le même
  aspect sur un petit téléphone et sur un grand écran.
- **La langue est celle du système**, sauf si le `header` en dit autrement.

## Ce qui est fait pour les téléphones

Quelques éléments existent parce que les téléphones les ont. Sur les autres systèmes, ils
s'affichent comme l'équivalent le plus proche : le fichier se construit et tourne donc partout.

| Tu écris | Sur un téléphone | Ailleurs |
|---|---|---|
| `ui.tabs(names, i) { … }.icons([…])` | des onglets en bas, une icône chacun | une rangée d'onglets |
| `vue.title("Notes")` | le titre de la page, dans la barre | rien |
| `vue.searchable(text) { … }` | un champ de recherche dans la barre | un champ au-dessus de la page |
| `ui.list(rows, i, { … }) delete { … }` | glisser une ligne vers la gauche la supprime | une liste simple : ajoute aussi un bouton |
| `ui.input(text) { … }.lines(8)` | un champ de 8 lignes, Entrée passe à la ligne | un champ d'une ligne |
| `vue.markdown` | `**gras**` et `[liens](…)` s'affichent comme tels | le texte tel qu'écrit, avec ses étoiles |
| `vue.background(ui.card).rounded(12)` | une carte aux couleurs du système | pas de fond |

[La page de la bibliothèque `ui`](/fr/library/ui/) les liste tous. La règle est celle du reste de
Kova : un élément qu'un système n'a pas n'empêche jamais ton programme de s'y construire.

## Un exemple complet : un minuteur Pomodoro

Un minuteur Pomodoro décompte 25 minutes de travail, puis 5 de pause. C'est un petit programme, et
il montre la seule chose qu'un téléphone change : **le programme ne tourne pas toujours**.

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

Deux lignes portent l'idée. `screen.every(250, Event.Tick)` demande un `Tick` quatre fois par
seconde, seulement tant que le minuteur tourne : un minuteur en pause ne demande rien. Et `Tick` ne
retire pas une seconde : il lit l'**horloge** et calcule ce qui reste à partir de l'instant où le
minuteur doit finir, `ends`.

La différence compte sur un téléphone. Compter les tops serait juste sur un ordinateur, où la
fenêtre tourne tout le temps. Un téléphone endort une appli quand l'utilisateur en ouvre une autre,
et les tops s'arrêtent. En lisant l'horloge, le premier `Tick` après le retour affiche la bonne
heure, quoi qu'il se soit passé entre-temps. Le programme complet, avec les sessions comptées et un
bip fabriqué en mémoire (un WAV écrit par le programme lui-même, sans fichier de son à emporter),
est `examples/pomodoro.kova` dans le dépôt de Kova.

## Tester sans téléphone

Pas besoin du téléphone à chaque changement. Le programme est le même fichier : l'essentiel se teste
donc sur l'ordinateur. `KOVA_UI_SCRIPT=1` affiche chaque vue en texte et clique les boutons que tu
nommes, comme le montre [Fenêtres](/fr/guide/windows/#tester-une-fenêtre). Les détails de téléphone
(l'aspect, le glissement, le clavier) demandent un téléphone ou un émulateur :
`kova build --target android-intel` fabrique un `.apk` pour l'émulateur que fournissent les outils
d'Android.

## Ce qu'un téléphone n'a pas encore

- **Pas de travail en arrière-plan.** Quand Android ou iOS range l'appli, le programme attend, et
  peut être fermé. Le Pomodoro ne peut pas sonner pendant qu'une autre appli est devant. Un service
  qui continue à tourner demande une partie d'Android que Kova n'a pas encore.
- **Pas de notifications.** Rien ne peut s'afficher hors de l'appli.
- **Les fichiers restent dans l'appli.** `import files` marche dans le dossier privé de l'appli ;
  pour lire un fichier ailleurs, demande à l'utilisateur avec `ui.open_file`.
- **Une seule fenêtre.** Un téléphone montre un écran d'appli à la fois ; `vue.pushing(…)` empile
  des pages avec un bouton retour.
