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
| `ui.text(s: String) -> ui.View` | une ligne de texte. |
| `ui.button(label: String, event: E) -> ui.View` | un bouton ; un clic envoie `event` à `update`. |
| `ui.column(items: List<ui.View>) -> ui.View` | les vues l'une sous l'autre. |
| `ui.row(items: List<ui.View>) -> ui.View` | les vues côte à côte. |

Tout est placé en partant d'en haut à gauche, sur chaque système.

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
| `KOVA_UI_SCRIPT=1` | pas de fenêtre : chaque vue est affichée en texte, et le bouton à cliquer (son numéro à partir de 0, dans l'ordre où ils sont écrits) est lu sur l'entrée ; la fin de l'entrée ferme la fenêtre. Pour les tests. |
| `KOVA_UI_HIDDEN=1` | la fenêtre existe mais n'est jamais montrée (Windows, macOS), pour un programme qui la pilote. |

Sans `width` ni `height`, la fenêtre s'ouvre à la taille de son contenu, avec au moins 320 pixels de large. Elle n'est jamais plus petite que son contenu, quelle que soit la taille demandée, et qui s'en sert peut la redimensionner : un clic ne remet jamais la première taille, il fait seulement grandir la fenêtre quand le contenu a besoin de plus de place.

Un programme `ui` tourne en code natif, même sous `kova run`.
