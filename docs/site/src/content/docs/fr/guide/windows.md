---
title: Fenêtres
description: Des fenêtres natives avec import ui, à partir d'un état, d'une vue et d'une mise à jour.
sidebar:
  order: 17
---

`import ui` ouvre une fenêtre du système lui-même : GTK sous Linux, Win32 sous Windows, AppKit sous
macOS. Le programme ressemble aux autres programmes du système et se comporte comme eux, et n'a
besoin de rien d'installé sous Windows et macOS (Linux a besoin de GTK 4, présent sur la plupart
des bureaux).

## État, vue, mise à jour

Un programme à fenêtre s'écrit en trois parties, comme en Elm ou en SwiftUI :

1. un **état** : une struct qui contient tout ce que la fenêtre montre ;
2. une **vue** : une fonction qui dessine l'état ;
3. une **mise à jour** : une fonction qui change l'état quand quelque chose se passe.

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

`ui.run(titre, état, vue, mise_à_jour)` montre la fenêtre et rend la main quand elle est fermée. À
chaque clic, l'événement du bouton va à `update`, qui change l'état, et la fenêtre est redessinée
à partir de `view`.

Rien d'autre ne change jamais la fenêtre. Il n'y a pas d'étiquette à retrouver pour la changer, pas
de bouton à activer depuis ailleurs : ce qui est à l'écran est toujours `view(état)`. Une fenêtre
qui montre quelque chose de faux, c'est une `view` à relire, ou un `update` à corriger, jamais un
changement oublié quelque part.

## Construire une vue

| Fonction | Montre |
| --- | --- |
| `ui.text(s)` | une ligne de texte |
| `ui.button(étiquette, événement)` | un bouton ; un clic envoie `événement` à `update` |
| `ui.column([…])` | des vues l'une sous l'autre |
| `ui.row([…])` | des vues côte à côte |

L'événement d'un bouton est du type que prend `update`, en général une enum qui liste ce qui peut
arriver. Il peut porter des valeurs : `Named(name: String)`, `Remove(index: Int)`.

Le compilateur vérifie les formes : `view` prend l'état et donne un `ui.View`, `update` prend l'état
en `var` et un événement, et l'événement de chaque bouton a le bon type.

## Tester une fenêtre

Un test ne peut pas regarder une fenêtre. Avec `KOVA_UI_SCRIPT=1`, aucune fenêtre ne s'ouvre :
chaque vue est affichée en texte, et le numéro du bouton à cliquer (compté à partir de 0, dans
l'ordre où ils sont écrits) est lu sur l'entrée ; la fin de l'entrée ferme la fenêtre. Sans
entrée :

```text
[Counter] column(text "0 clicks", row(button "+1", button "-1"))
closed
```

Avec l'entrée `0`, puis `0`, puis `1`, il montre 1 clic, 2, puis 1 à nouveau, puis se ferme. Un
test peut ainsi faire passer un programme à fenêtre par tous ses cas, sur n'importe quelle machine,
même sans écran.

## La suite

`import ui` a aujourd'hui du texte, des boutons, des lignes et des colonnes. Les champs de texte et
d'autres éléments sont prévus. Pour un élément qu'il n'a pas encore, un programme peut appeler la
bibliothèque du système elle-même avec [`extern`](/fr/guide/calling-c/).
