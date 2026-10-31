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
        .Plus => app.clicks := app.clicks + 1
        .Minus => app.clicks := app.clicks - 1
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

La fenêtre s'ouvre à la taille de son contenu. Pour choisir sa taille de départ, ajoute
`width(…)` et `height(…)`, ensemble ou seuls : `ui.run("Counter", App { clicks := 0 }, view, update) width(400)`.
Elle ne devient jamais plus petite que son contenu, et la personne qui s'en sert peut la
redimensionner.

Rien d'autre ne change jamais la fenêtre. Il n'y a pas d'étiquette à retrouver pour la changer, pas
de bouton à activer depuis ailleurs : ce qui est à l'écran est toujours `view(état)`. Une fenêtre
qui montre quelque chose de faux, c'est une `view` à relire, ou un `update` à corriger, jamais un
changement oublié quelque part.

## Construire une vue

| Fonction | Montre |
| --- | --- |
| `ui.text(s)` | une ligne de texte |
| `ui.button(étiquette, événement)` | un bouton ; un clic envoie `événement` à `update` |
| `ui.input(valeur) { s: String => événement }` | un champ de texte qui contient `valeur` ; chaque changement envoie l'événement du bloc à `update` |
| `ui.column([…])` | des vues l'une sous l'autre |
| `ui.row([…])` | des vues côte à côte |
| `ui.column([…]) align(center)` | les vues de la colonne centrées dans la fenêtre (`left`, `center`, `right`) |
| `ui.row([…]) align(center)` | les vues de la ligne centrées dans sa hauteur (`top`, `center`, `bottom`) |

L'événement d'un bouton est du type que prend `update`, en général une enum qui liste ce qui peut
arriver. Il peut porter des valeurs : `Named(name: String)`, `Remove(index: Int)`.

Sans `align`, tout part d'en haut à gauche. Pour centrer une ligne de boutons dans la fenêtre,
mets-la dans une colonne centrée : `ui.column([ui.row([…])]) align(center)`.

Le compilateur vérifie les formes : `view` prend l'état et donne un `ui.View`, `update` prend l'état
en `var` et un événement, et l'événement de chaque bouton a le bon type.

## Donner du style à une vue

Une vue suit l'apparence du système tant qu'on ne dit pas autre chose. Les méthodes de style
s'enchaînent dans n'importe quel ordre, et chacune donne une nouvelle vue :

```kova
import ui

struct App {
    name: String
}

enum Event {
    Renamed(name: String)
}

fn view(app: App) -> ui.View {
    let wrong: Bool := app.name.length > 8
    return ui.column([
        ui.text("Votre nom").size(20).bold,
        ui.input(app.name) { s: String => Event.Renamed(s.clone) }.color(if wrong { ui.red } else { ui.black }),
        ui.text("Huit lettres au maximum").size(11).color(ui.gray).margin(4),
    ])
}

fn update(app: var App, e: Event) {
    match e {
        .Renamed(name) => app.name := name.clone
    }
}

fn main {
    ui.run("Style", App { name := "" }, view, update)
}
```

Ici, le texte du champ passe au rouge tant que le nom est trop long. Le champ reste le même : la
personne qui écrit garde son curseur.

Les couleurs sont `ui.red`, `ui.blue` et les autres, ou n'importe quelle couleur avec
`ui.rgb(37, 99, 235)`. Sur une colonne ou une ligne, `color`, `size` et `bold` vont à toutes ses
vues ; le style propre d'une vue l'emporte. `margin` ajoute des pixels vides autour de la vue où il
est écrit.

Le texte d'un bouton garde la couleur du système : Windows ne laisse pas un bouton natif la changer,
et une fenêtre doit se comporter pareil partout. `size` et `bold` y fonctionnent.

Une image vient d'un fichier, et `.size` donne sa largeur, la hauteur suit :

```kova
ui.image("logo.png").size(64)
```

Si le fichier n'existe pas, le programme s'arrête et le nomme, plutôt que de laisser un trou dans la
fenêtre. Un chemin relatif part du dossier où le programme a été lancé.

## Les champs de texte

Un champ montre une valeur de l'état, et chaque changement fait par l'utilisateur, lettre par
lettre, devient un événement : le bloc reçoit le texte que contient maintenant le champ et donne
l'événement.

```kova
import ui

enum Event {
    Renamed(name: String)
}

struct App {
    name: String
}

fn view(app: App) -> ui.View {
    return ui.column([
        ui.input(app.name) { s: String => Event.Renamed(s.clone) },
        ui.text("Hello {app.name}"),
    ])
}

fn update(app: var App, e: Event) {
    match e {
        .Renamed(name) => app.name := name.clone
    }
}

fn main {
    ui.run("Greeting", App { name := "" }, view, update)
}
```

```text
[Greeting] column(input "", text "Hello ")
```

En [mode test](#tester-une-fenêtre), il affiche la vue ci-dessus. Dans une fenêtre, « Hello … » suit le champ pendant qu'on tape. Le bloc prête `s` ([La possession](/fr/guide/ownership/)),
donc l'événement en prend une copie : `s.clone`.

Le champ montre toujours l'état. Si `update` ne prend pas un changement (un champ d'âge qui refuse
une lettre, par exemple), le champ revient à ce que contient l'état, donc l'écran ne montre jamais
une valeur que le programme n'a pas.

## Les widgets qui gardent une valeur

Une case à cocher, un curseur, une liste déroulante, une liste ou un tableau montrent chacun une
valeur, et cette valeur vit dans ton état, comme le texte d'un champ. Le widget la reçoit, et un bloc
transforme ce qu'a fait l'utilisateur en événement :

```kova
import ui

enum Event {
    Dark(on: Bool)
    Volume(v: Int)
    Size(i: Int)
}

struct App {
    dark: Bool
    volume: Int
    size: Int
}

fn view(app: App) -> ui.View {
    return ui.column([
        ui.checkbox("Sombre", app.dark) { on: Bool => Event.Dark(on) },
        ui.slider(app.volume, 0, 10) { v: Int => Event.Volume(v) },
        ui.choice(["Petit", "Moyen", "Grand"], app.size) { i: Int => Event.Size(i) },
        ui.text("Volume {app.volume}"),
    ])
}

fn update(app: var App, e: Event) {
    match e {
        .Dark(on) => app.dark := on
        .Volume(v) => app.volume := v
        .Size(i) => app.size := i
    }
}

fn main {
    ui.run("Réglages", App { dark := false, volume := 3, size := 0 }, view, update)
}
```

Bouger le curseur ne change rien tout seul côté Kova : l'événement arrive à `update`, l'état change,
et `view` dessine le curseur à sa nouvelle place. Un programme peut donc refuser une valeur, ou lier
deux widgets, simplement par ce que fait `update`.

`ui.choice`, `ui.list`, `ui.table` et `ui.tabs` travaillent avec une position comptée à partir de 0,
pas avec le texte de la ligne : deux lignes peuvent se lire pareil, et une position reste juste quand
les textes sont traduits. `ui.list` et `ui.table` prennent `-1` pour « rien de sélectionné ».

`ui.tabs` n'est que la rangée d'onglets. La page montrée dessous, c'est à toi de l'écrire, et cela
tient en un `if` dans `view` :

```kova
let page: ui.View := if app.tab = 0 { settings(app) } else { about(app) }
return ui.column([ui.tabs(["Réglages", "À propos"], app.tab) { i: Int => Event.Tab(i) }, page])
```

## Les menus

Une ligne de menu est un bouton placé ailleurs : elle envoie un événement à `update`. `menus` se met
sur la vue entière, la plus extérieure :

```kova
return ui.column([content]).menus([
    ui.menu("Fichier", [ui.item("Ouvrir", Event.Open), ui.separator, ui.item("Quitter", Event.Quit)]),
])
```

Sur Mac, la barre est en haut de l'écran, et contient toujours le menu de l'application (avec
Quitter) et un menu Édition, pour que copier et coller marchent dans les champs de texte.

## Les fenêtres de dialogue

`ui.open_file`, `ui.save_file` et `ui.open_folder` montrent la fenêtre du système et rendent le
chemin, ou `none` si l'utilisateur annule. `ui.message` et `ui.confirm` posent une question. Toutes
attendent la réponse, alors `update` se lit comme une phrase :

```kova
fn update(app: var App, e: Event) {
    match e {
        .Open => match ui.open_file("Ouvrir une note") {
            none => {}
            path => app.path := path.clone
        }
        .Quit => if ui.confirm("Quitter", "Partir sans enregistrer ?") {
            ui.close
        }
    }
}
```

En mode `KOVA_UI_SCRIPT`, aucune fenêtre ne s'ouvre : chaque dialogue affiche ce qu'il demanderait,
puis lit sa réponse sur la ligne suivante de l'entrée.

## Dessiner

Les éléments natifs conviennent aux formulaires. Pour un jeu, un graphique ou une apparence bien à
toi, `ui.canvas` est une zone où tu dessines toi-même. C'est une vue comme les autres : elle se
place dans une colonne à côté de textes et de boutons, et elle suit la même règle. L'état dit ce
qu'il y a, `view` le dessine, `update` le change.

```kova
import ui

enum Event {
    Pressed(x: Int, y: Int)
    Tick
}

struct App {
    x: Int
    y: Int
    ticks: Int
}

fn view(app: App) -> ui.View {
    return ui.canvas(300, 200) { d: ui.Draw =>
        d.clear(ui.white)
        d.circle(app.x, app.y) radius(20) color(ui.blue)
        d.text("{app.ticks} ticks", 10, 10) color(ui.gray)
    }
    .on_mouse { m: ui.Mouse => Event.Pressed(m.x, m.y) }
    .every(1000, Event.Tick)
}

fn update(app: var App, e: Event) {
    match e {
        .Pressed(x, y) => {
            app.x := x
            app.y := y
        }
        .Tick => app.ticks := app.ticks + 1
    }
}

fn main {
    ui.run("Dot", App { x := 150, y := 100, ticks := 0 }, view, update)
}
```

```text
[Dot] canvas 300x200 clear(#ffffff) circle(150 100 20 #2563eb 0) text(10 10 16 #6b7280 "0 ticks") every(1000)
```

Le bloc s'exécute à chaque construction de la vue : un `for` ou un `if` à l'intérieur dessine ce que
contient l'état. Il n'y a rien à effacer ni à déplacer à la main. Tu changes l'état, et le dessin
suivant le montre.

Les positions sont en pixels depuis le coin en haut à gauche du canvas. `d.width` et `d.height`
donnent sa taille.

`ui.fill` à la place d'un nombre donne au canvas toute la place disponible, pour qu'il suive la
fenêtre : `ui.canvas(ui.fill, 36)` est une barre aussi large que la fenêtre, `ui.canvas(ui.fill, ui.fill)`
prend tout ce que les autres vues laissent. `d.width` et `d.height` donnent alors sa vraie taille,
et il est redessiné à chaque changement.

```kova
d.clear(ui.white)
d.rect(10, 20) size(100, 50) color(ui.red)
d.rect(10, 20) size(100, 50) outline(2)
d.circle(150, 80) radius(30) color(ui.blue)
d.line(0, 0) to(100, 100) thickness(3)
d.text("Score", 10, 90) size(20) color(ui.gray)
```

Sans `color`, c'est noir. `outline(n)` ne trace que le bord, épais de `n` pixels, à l'intérieur de
la forme ; sans lui, la forme est remplie. Un canvas commence blanc.

### Souris, clavier, horloge

Trois méthodes font écouter un canvas. Chacune transforme ce qui s'est passé en un de tes
événements :

| Écrit | Envoie son événement |
| --- | --- |
| `.on_mouse { m: ui.Mouse => … }` | quand un bouton est pressé ou relâché, et quand la souris bouge. |
| `.on_key { k: ui.Key => … }` | à chaque touche pressée, tant que le canvas a le clavier. |
| `.every(ms, event)` | toutes les `ms` millisecondes. |

`m.x` et `m.y` donnent la position dans le canvas. `m.pressed`, `m.released` et `m.moved` disent ce
qui s'est passé, `m.right` que c'est le bouton droit. Un mouvement avec un bouton enfoncé reste un
`m.moved` : pour glisser-déposer, il faut donc une ligne dans ton état :

```kova
.Mouse(x, y, pressed, released) => {
    if pressed {
        app.dragging := true
    }
    if released {
        app.dragging := false
    }
    if app.dragging {
        app.x := x
    }
}
```

`k.name` est ce que la touche écrit (`"a"`, `"A"`, `"é"`, `"1"`), ou un mot pour les autres, le même
sur tous les systèmes : `Left`, `Right`, `Up`, `Down`, `Space`, `Enter`, `Escape`, `Tab`,
`Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `F1` à `F12`.

`.every` marche aussi sur n'importe quelle autre vue, par exemple pour rafraîchir une horloge. Pour
une animation, 16 millisecondes donnent environ 60 images par seconde. Un programme plus lent que
son horloge saute des tics plutôt que de prendre du retard, et seul le dernier mouvement de souris
attend, mais un clic n'est jamais perdu.

```kova
import ui

enum Event {
    Tick
    Key(name: String)
}

struct App {
    x: Int
    speed: Int
}

fn view(app: App) -> ui.View {
    return ui.canvas(300, 60) { d: ui.Draw =>
        d.circle(app.x, 30) radius(10) color(ui.red)
    }
    .on_key { k: ui.Key => Event.Key(k.name.clone) }
    .every(16, Event.Tick)
}

fn update(app: var App, e: Event) {
    match e {
        .Tick => app.x := app.x + app.speed
        .Key(name) => match name {
            "Left" => app.speed := -2
            "Right" => app.speed := 2
            _ => {}
        }
    }
}

fn main {
    ui.run("Ball", App { x := 150, speed := 0 }, view, update)
}
```

```text
[Ball] canvas 300x60 circle(150 30 10 #dc2626 0) every(16)
```

Le canvas reçoit le clavier à l'ouverture de la fenêtre et à chaque clic sur lui ; un clic dans un
champ de texte le lui reprend.

En [mode test](#tester-une-fenêtre), un canvas affiche ce qu'il dessine, et `k=press 10 20 1`,
`k=release 10 20 1`, `k=move 10 20 0` ou `k=key Left` joue un geste sur l'écouteur `k`. Les
écouteurs se comptent avec les champs de texte, dans l'ordre d'écriture de la vue ; pour un canvas,
la souris avant le clavier si `.on_mouse` est écrit en premier. `1` est le bouton gauche, `2` le
droit. Les horloges se comptent avec les boutons.

## Dessiner avec la carte graphique

`ui.canvas` demande au système de dessiner les formes une par une : c'est simple et ça suffit à la
plupart des fenêtres. Pour des milliers de formes, pour la 3D ou pour des effets calculés sur chaque
pixel, `ui.gpu` confie le travail à la carte graphique. Il y a plus à apprendre, car tu écris deux
petits programmes pour la carte elle-même. On les appelle des *shaders*, et ils s'écrivent en GLSL,
le langage d'OpenGL, pas en Kova :

- le **vertex shader** tourne une fois par coin de triangle, et dit où ce coin arrive. La fenêtre va
  de -1 à 1 dans les deux sens, quelle que soit sa taille en pixels, avec 1 en haut.
- le **fragment shader** tourne une fois par pixel du triangle, et dit sa couleur. Ce que le vertex
  shader écrit dans une variable `out` lui arrive mélangé entre les trois coins : c'est ainsi que le
  triangle ci-dessous passe en douceur du rouge au vert puis au bleu.

```kova
import ui

let vertex_code: String := """
#version 330 core
layout(location = 0) in vec2 place;
layout(location = 1) in vec3 shade;
uniform float turn;
out vec3 kept;
void main() {
    float c = cos(turn), s = sin(turn);
    gl_Position = vec4(c * place.x - s * place.y, s * place.x + c * place.y, 0.0, 1.0);
    kept = shade;
}
"""

let fragment_code: String := """
#version 330 core
in vec3 kept;
out vec4 color;
void main() { color = vec4(kept, 1.0); }
"""

enum Event {
    Tick
}

fn view(turn: Float) -> ui.View {
    let area: ui.View := ui.gpu(400, 400) { g: ui.Gpu =>
        g.clear(ui.black)
        g.shader(vertex_code, fragment_code)
        g.uniform("turn", [turn])
        g.triangles([
            0.0, 0.8, 1.0, 0.0, 0.0,
            -0.7, -0.5, 0.0, 1.0, 0.0,
            0.7, -0.5, 0.0, 0.0, 1.0,
        ]) parts([2, 3])
    }
    return area.every(16, Event.Tick)
}

fn update(turn: var Float, e: Event) {
    turn += 0.02
}

fn main {
    ui.run("Triangle", 0.0, view, update)
}
```

La liste donnée à `g.triangles` est à plat : `parts([2, 3])` dit que chaque sommet fait 2 nombres,
puis 3, que les shaders lisent comme l'attribut 0 (`place`) et l'attribut 1 (`shade`). Trois sommets
font un triangle. Sans `parts`, un sommet fait 2 nombres.

`g.uniform` donne aux shaders une valeur qui reste la même pendant tout le dessin, ici l'angle. Il
prend de 1 à 4 nombres (`float`, `vec2`, `vec3`, `vec4`), ou 16 pour une `mat4`, colonne par colonne.
Un programme en 3D calcule sa matrice en Kova, et multiplie chaque sommet par elle dans le vertex
shader.

Le reste marche comme un canvas. `view` repeint toute l'image, alors une animation est une horloge
qui change l'état. `.on_mouse`, `.on_key` et `ui.fill` marchent pareil. Chaque image commence noire,
avec la profondeur remise à zéro, et ce qui est plus près cache ce qui est plus loin : pour la 3D,
donne un troisième nombre à chaque sommet. Une couleur dont l'alpha est sous 1 se mélange à ce qui
est derrière elle.

`g.texture(width, height, pixels)` donne une image aux shaders, à raison de 4 octets par pixel (rouge,
vert, bleu, alpha), la première ligne d'abord. Ils la lisent par un `uniform sampler2D`, qui n'a pas
besoin de `g.uniform`.

Un shader est compilé quand la fenêtre le dessine pour la première fois. S'il contient une erreur, le
programme s'arrête avec ce qu'a dit le pilote graphique :

```console
erreur : un shader ne compile pas : 0:4(23): error: `oops' undeclared
```

`ui.gpu` demande OpenGL 3.3, que presque tous les ordinateurs ont depuis 2010, et les shaders
commencent par `#version 330 core`. Apple n'améliore plus OpenGL, mais tous les Mac le font encore
tourner. Les sommets voyagent en texte à chaque image : cela tient bien jusqu'à quelques dizaines de
milliers, pas jusqu'à des millions.

## Une fenêtre avec ton propre cadre

`frame(false)` sur `ui.run` retire ce que le système dessine autour de la fenêtre : barre de titre,
boutons, bordure. Tout l'intérieur est à toi, donc un canvas peut servir de barre de titre. Quatre
fonctions, appelées depuis `update`, remplacent ce qui a disparu :

| Écrit | Fait |
| --- | --- |
| `ui.close` | ferme la fenêtre ; `ui.run` se termine et le programme continue. |
| `ui.minimize` | envoie la fenêtre dans la barre des tâches ou le Dock. |
| `ui.maximize` | agrandit la fenêtre au maximum, à côté de la barre de menus et du Dock ou de la barre des tâches, comme le Finder, ou revient en arrière si c'est déjà le cas. |
| `ui.full_screen` | prend tout l'écran, sans rien autour, ou revient en arrière. Sur Mac, c'est le vrai plein écran, comme avec le bouton vert. |
| `ui.drag` | le système déplace la fenêtre avec la souris, jusqu'au relâchement du bouton. |

```kova
import ui

enum Event {
    Pressed(x: Int, y: Int, pressed: Bool)
    Closed
}

fn view(n: Int) -> ui.View {
    return ui.column([
        ui.canvas(300, 32) { d: ui.Draw =>
            d.clear(ui.rgb(40, 44, 52))
            d.text("Mon programme", 10, 8) color(ui.white)
        }
        .on_mouse { m: ui.Mouse => Event.Pressed(m.x, m.y, m.pressed) },
        ui.button("Fermer", Event.Closed),
    ])
}

fn update(n: var Int, e: Event) {
    match e {
        .Pressed(x, y, pressed) => {
            if pressed {
                ui.drag
            }
        }
        .Closed => ui.close
    }
}

fn main {
    ui.run("Bare", 0, view, update) frame(false)
}
```

```text
[Bare] column(canvas 300x32 clear(#282c34) text(10 8 16 #ffffff "Mon programme"), button "Fermer")
(no frame)
```

`ui.drag` se place juste après une pression, tant que le bouton est enfoncé : il part de cette
pression et ne fait rien s'il n'y en a pas eu. Le relâchement va ensuite au système, donc
`m.released` n'arrive pas.

Les 16 pixels que Kova laisse autour du contenu disparaissent aussi, pour qu'une barre de titre
touche les bords de la fenêtre. Mets `.margin` sur ce qui a besoin de place.

Sans cadre, la fenêtre ne se redimensionne pas par ses bords : donne-lui sa taille avec `width` et
`height`, et propose `ui.maximize` si elle doit grandir.


## Déplacer et redimensionner la fenêtre toi-même

`ui.window` dit où est la fenêtre, `ui.screen` ce que l'écran lui laisse, sans la barre de menus, le
Dock ou la barre des tâches. Les deux donnent un `ui.Area`, avec `x`, `y`, `width` et `height`.
`ui.place(x, y) size(width, height)` met la fenêtre à cet endroit, tout de suite.

`ui.animate` amène la fenêtre sur une zone, en douceur. Tu l'appelles depuis `update`, Kova fait le
reste : `ui.animate(ui.screen) time(1000)` remplit l'écran en une seconde.

`time` est en millisecondes, 300 si tu ne le mets pas. Sans courbe, la fenêtre démarre lentement,
accélère, puis arrive lentement, sans rebond.

Le but est n'importe quelle `ui.Area` : `ui.Area { x := 100, y := 100, width := 640, height := 480 }`.
Le programme continue d'entendre les clics pendant l'animation, et un nouveau `ui.animate` remplace
celui en cours.

Tu peux donner ta propre courbe, et construire n'importe quel autre mouvement avec une horloge et
`ui.place` : [Animations](/fr/guide/animations/) explique comment, de la première image aux rebonds.

Sous Linux, `ui.place` ne fait rien pour l'instant : Wayland ne laisse aucun programme choisir la
place de sa fenêtre, et GTK 4 ne sait pas redimensionner une fenêtre ouverte. `ui.window` donne 0 pour
`x` et `y`, et `ui.screen` tout l'écran, panneaux compris.

## Tester une fenêtre

Un test ne peut pas regarder une fenêtre. Avec `KOVA_UI_SCRIPT=1`, aucune fenêtre ne s'ouvre :
chaque vue est affichée en texte, et le numéro du bouton à cliquer (compté à partir de 0, dans
l'ordre où ils sont écrits) est lu sur l'entrée, ou `k=texte` pour taper `texte` dans le champ `k`
(les champs sont comptés à part, eux aussi à partir de 0) ; la fin de l'entrée ferme la fenêtre. Sans
entrée :

```text
[Counter] column(text "0 clicks", row(button "+1", button "-1"))
closed
```

Avec l'entrée `0`, puis `0`, puis `1`, il montre 1 clic, 2, puis 1 à nouveau, puis se ferme. Un
test peut ainsi faire passer un programme à fenêtre par tous ses cas, sur n'importe quelle machine,
même sans écran.

## La suite

Pour un élément que `import ui` n'a pas encore, un programme peut appeler la
bibliothèque du système elle-même avec [`extern`](/fr/guide/calling-c/).
