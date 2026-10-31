---
title: ui
description: Des fenêtres natives à partir d'un état, d'une vue et d'une mise à jour.
sidebar:
  order: 14
---

```kova
import ui
```

Le chapitre [Fenêtres](/fr/guide/windows/) du guide explique comment se construit un programme à
fenêtre. Cette page liste ce qu'a `ui`.

| Écrit | Fait |
| --- | --- |
| `ui.run(title, state, view, update)` | montre une fenêtre de titre `title`, dessinée par `view(state)`, jusqu'à ce qu'elle soit fermée. À chaque clic, `update(var state, event)` tourne, et la fenêtre est redessinée. |
| `ui.run(…) width(w) height(h)` | pareil, la fenêtre s'ouvrant avec `w` pixels de large et `h` de haut ; chaque partie peut être omise. |
| `ui.run(…) frame(false)` | la fenêtre sans le cadre du système ([ton propre cadre](/fr/guide/windows/#une-fenêtre-avec-ton-propre-cadre)). |
| `ui.close`, `ui.minimize`, `ui.maximize`, `ui.full_screen`, `ui.drag` | depuis `update` : ferme la fenêtre, la réduit, l'agrandit ou la rétablit, la met en plein écran ou l'en sort, la déplace avec la souris après une pression. |
| `ui.window -> ui.Area`, `ui.screen -> ui.Area` | où est la fenêtre, ce que l'écran lui laisse : `x`, `y`, `width`, `height`. |
| `ui.place(x, y) size(width, height)` | depuis `update` : met la fenêtre à cet endroit ; ne fait rien sous Linux pour l'instant. |
| `ui.animate(area) time(ms) curve { t: Float => … }` | depuis `update` : amène la fenêtre sur `area` en douceur, en 300 ms sans `time`, avec un départ et une arrivée lents sans `curve` ; ne fait rien sous Linux pour l'instant. |
| `ui.text(s: String) -> ui.View` | une ligne de texte. |
| `ui.button(label: String, event: E) -> ui.View` | un bouton ; un clic envoie `event` à `update`. |
| `ui.input(value: String) { s: String => E } -> ui.View` | un champ de texte qui contient `value` ; à chaque changement fait par l'utilisateur, l'événement du bloc, fait à partir du texte que contient maintenant le champ, va à `update` ([Les champs de texte](/fr/guide/windows/#les-champs-de-texte)). |
| `ui.image(path: String) -> ui.View` | un fichier image : PNG ou JPEG sur chaque système. Un chemin qui n'existe pas arrête le programme en le nommant. `.size(w)` donne sa largeur en pixels, la hauteur suit. |
| `ui.canvas(width: Int, height: Int) { d: ui.Draw => … } -> ui.View` | une zone de dessin ([Dessiner](/fr/guide/windows/#dessiner)). `ui.fill` à la place d'une taille prend toute la place disponible. |
| `ui.column(items: List<ui.View>) -> ui.View` | les vues l'une sous l'autre. |
| `ui.row(items: List<ui.View>) -> ui.View` | les vues côte à côte. |
| `ui.column(…) align(left\|center\|right)` | la colonne prend toute la largeur qu'on lui donne et y place ses vues. |
| `ui.row(…) align(top\|center\|bottom)` | les vues en haut, au milieu ou en bas de la ligne. |
| `vue.color(c: ui.Color) -> ui.View` | la couleur d'un texte ou d'un champ. Un bouton garde la couleur du système. |
| `vue.size(pixels: Int) -> ui.View` | la hauteur du texte, pour un texte, un bouton ou un champ. |
| `vue.bold -> ui.View` | le texte en gras. |
| `vue.margin(pixels: Int) -> ui.View` | des pixels vides tout autour de la vue. |
| `vue.on_mouse { m: ui.Mouse => E } -> ui.View` | sur un canvas : l'événement du bloc à chaque pression, relâchement et mouvement de la souris. |
| `vue.on_key { k: ui.Key => E } -> ui.View` | sur un canvas : l'événement du bloc à chaque touche pressée. |
| `vue.every(ms: Int, event: E) -> ui.View` | envoie `event` toutes les `ms` millisecondes, sur n'importe quelle vue. |
| `ui.rgb(r: Int, g: Int, b: Int) -> ui.Color` | n'importe quelle couleur, chaque composante allant de 0 à 255. |
| `ui.red`, `ui.green`, `ui.blue`, `ui.orange`, `ui.yellow`, `ui.purple`, `ui.gray`, `ui.black`, `ui.white` | des couleurs toutes prêtes. |

## Dessiner sur un canvas

| Écrit | Fait |
| --- | --- |
| `d.clear(c: ui.Color)` | peint tout le canvas. |
| `d.rect(x, y) size(w, h) color(c) outline(n)` | un rectangle, depuis son coin en haut à gauche. |
| `d.circle(x, y) radius(r) color(c) outline(n)` | un cercle, depuis son centre. |
| `d.line(x, y) to(x2, y2) color(c) thickness(n)` | une ligne, épaisse de 1 pixel par défaut. |
| `d.text(s, x, y) size(n) color(c)` | un texte, depuis son coin en haut à gauche, haut de 16 pixels par défaut. |
| `d.width`, `d.height` | la taille du canvas. |

Tous les nombres sont des `Int`, en pixels. `color` vaut noir par défaut. `outline(n)` ne trace que
le bord ; sans lui, la forme est remplie.

| Champ | C'est |
| --- | --- |
| `m.x`, `m.y` | la position de la souris dans le canvas. |
| `m.pressed`, `m.released`, `m.moved` | ce qui vient de se passer. |
| `m.right` | c'est le bouton droit. |
| `k.name` | ce que la touche écrit, ou `Left`, `Right`, `Up`, `Down`, `Space`, `Enter`, `Escape`, `Tab`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `F1` à `F12`. |

## Placement et style

Tout est placé en partant d'en haut à gauche, sur chaque système, sauf si `align` dit autre chose.

Les méthodes de style s'enchaînent dans n'importe quel ordre : `ui.text("Total").size(20).bold.color(ui.red)`.
Sur une colonne ou une ligne, `color`, `size` et `bold` vont à toutes ses vues, et le style propre
d'une vue l'emporte. `margin` reste sur la vue où il est écrit.

Le programme écrit les deux fonctions que prend `ui.run` :

- `fn view(app: App) -> ui.View`, où `App` est le type de l'état ;
- `fn update(app: var App, e: Event)`, où `Event` est le type des événements des boutons.

Leurs noms sont libres, et un programme a un seul `ui.run`. Le compilateur vérifie les deux
formes, et que l'événement de chaque bouton est un `Event`.

| Système | Fenêtres faites avec | Besoin |
| --- | --- | --- |
| Linux | GTK 4 | GTK 4, présent sur la plupart des bureaux |
| Windows | Win32 | rien |
| macOS | AppKit | rien |

## Variables d'environnement

| Variable | Effet |
| --- | --- |
| `KOVA_UI_SCRIPT=1` | pas de fenêtre : chaque vue est affichée en texte, et le bouton à cliquer (son numéro à partir de 0, dans l'ordre où ils sont écrits), ou `k=texte` pour taper dans le champ `k`, ou un geste comme `k=press 10 20 1` ou `k=key Left` pour un canvas, est lu sur l'entrée ; la fin de l'entrée ferme la fenêtre. Pour les tests. |
| `KOVA_UI_HIDDEN=1` | la fenêtre existe mais n'est jamais montrée (Windows, macOS), pour un programme qui la pilote. |

Sans `width` ni `height`, la fenêtre s'ouvre à la taille de son contenu, avec au moins 320 pixels de large. Elle n'est jamais plus petite que son contenu, quelle que soit la taille demandée, et qui s'en sert peut la redimensionner : un clic ne remet jamais la première taille, il fait seulement grandir la fenêtre quand le contenu a besoin de plus de place.

Un programme `ui` tourne en code natif, même sous `kova run`.
