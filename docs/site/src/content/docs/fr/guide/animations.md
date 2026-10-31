---
title: Animations
description: Comment marche une animation, comment construire tes propres courbes, et comment faire bouger n'importe quoi dans le temps.
sidebar:
  order: 18
---

Une animation est une suite d'images fixes, montrées assez vite pour que l'œil voie un mouvement.
Kova n'a pas de moteur d'animation caché : un programme change un nombre dans le temps et redessine.
Ce chapitre construit ça à partir de rien, puis montre comment faire n'importe quel mouvement. Tout
ici est du Kova ordinaire, donc tout peut être changé.

Lis d'abord [Fenêtres](/fr/guide/windows/) : il explique l'état, `view` et `update`.

## Les trois questions

Toute animation répond aux trois mêmes questions :

1. **Combien de temps ?** Un nombre de pas, ou une durée en millisecondes.
2. **D'où à où ?** Deux valeurs : un départ et un but.
3. **De quelle façon ?** À vitesse constante, en ralentissant, en rebondissant : c'est la **courbe**.

Elles se rejoignent dans un seul calcul, fait à chaque image :

```text
valeur = départ + (but - départ) * progression
```

Avec `progression` à `0.0`, la valeur est le départ. À `1.0`, c'est le but. À `0.5`, c'est la moitié
du chemin. Tout le chapitre consiste à choisir `progression`.

## Une première animation, à la main

Cette balle traverse un canvas quand tu cliques. L'état est un seul nombre, le pas en cours :

```kova
import ui

enum Event {
    Start
    Tick
}

let steps: Int := 60

fn view(step: Int) -> ui.View {
    let t: Float := step.to_float / steps.to_float
    let x: Int := 40 + (320.0 * t).to_int
    return ui.column([
        ui.button("Go", Event.Start),
        ui.canvas(400, 100) { d: ui.Draw =>
            d.clear(ui.white)
            d.circle(x, 50) radius(20) color(ui.blue)
        },
    ]).every(16, Event.Tick)
}

fn update(step: var Int, e: Event) {
    match e {
        Start => step := 1
        Tick => {
            if step > 0 && step < steps {
                step := step + 1
            }
        }
    }
}

fn main {
    ui.run("Ball", 0, view, update)
}
```

```text
[Ball] column(button "Go", canvas 400x100 clear(#ffffff) circle(40 50 20 #2563eb 0)) every(16)
```

Voici ce qui se passe :

1. `.every(16, Event.Tick)` envoie `Tick` toutes les 16 millisecondes, environ 60 fois par seconde.
2. Chaque `Tick` ajoute 1 à `step`, jusqu'à atteindre `steps`.
3. Chaque changement de l'état reconstruit la vue, comme toujours.
4. `view` transforme le pas en `t`, de `0.0` à `1.0` : `step.to_float / steps.to_float`.
5. `40 + 320.0 * t` est le calcul ci-dessus : le départ est 40, le but 360.

Donc 60 pas de 16 millisecondes durent environ une seconde. Pour changer la durée, change `steps`.

La balle avance à vitesse constante, parce que `t` sert directement de progression. Ça fait
mécanique : rien dans le monde réel ne démarre ni ne s'arrête à pleine vitesse. Une courbe corrige ça.

## Ce qu'est une courbe

Une courbe est une fonction qui reçoit `t`, le **temps** écoulé, et donne la **progression**. Les deux
vont en général de `0.0` à `1.0`. Pour en utiliser une dans la balle, remplace `320.0 * t` par
`320.0 * curve(t)`.

Avant de mettre une courbe dans une fenêtre, regarde-la. Ce programme n'a pas besoin de fenêtre :

```kova
fn curve(t: Float) -> Float {
    return t * t
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t} -> {curve(t)}")
    }
}
```

```text
0.0 -> 0.0
0.25 -> 0.0625
0.5 -> 0.25
0.75 -> 0.5625
1.0 -> 1.0
```

Lis le tableau ainsi : au quart du temps, la balle a fait `0.0625` du chemin. À la moitié du temps,
seulement un quart du chemin. Elle démarre lentement, puis accélère. La raison est simple : multiplier
par lui-même un nombre plus petit que 1 le rend plus petit, et d'autant plus qu'il est petit.

Plus de multiplications donnent un effet plus fort : `t * t * t`, qui s'écrit aussi `t.pow(3)`,
démarre plus lentement et finit plus vite.

## Construire des courbes à partir d'autres

### Inverser une courbe

Pour ralentir à la fin, joue le même mouvement à l'envers. Remplace `t` par `1.0 - t` pour inverser
le temps, et enlève le résultat de `1.0` pour inverser la distance :

```kova
fn speed_up(t: Float) -> Float {
    return t * t
}

fn slow_down(t: Float) -> Float {
    return 1.0 - speed_up(1.0 - t)
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t} -> {slow_down(t)}")
    }
}
```

```text
0.0 -> 0.0
0.25 -> 0.4375
0.5 -> 0.75
0.75 -> 0.9375
1.0 -> 1.0
```

Ça marche avec n'importe quelle courbe : `1.0 - c(1.0 - t)` est `c` jouée à l'envers.

### Joindre deux courbes

Pour un départ et une arrivée tous les deux lents, accélère pendant la première moitié et ralentis
pendant la seconde. Chaque moitié est serrée dans la moitié du temps et la moitié du chemin :

```kova
fn speed_up(t: Float) -> Float {
    return t * t
}

fn slow_down(t: Float) -> Float {
    return 1.0 - speed_up(1.0 - t)
}

fn smooth(t: Float) -> Float {
    if t < 0.5 {
        return speed_up(t * 2.0) / 2.0
    }
    return 0.5 + slow_down(t * 2.0 - 1.0) / 2.0
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t} -> {smooth(t)}")
    }
}
```

```text
0.0 -> 0.0
0.25 -> 0.125
0.5 -> 0.5
0.75 -> 0.875
1.0 -> 1.0
```

`t * 2.0` fait aller la première moitié de `0.0` à `1.0`, et `/ 2.0` ramène le résultat dans la
première moitié du chemin. `ui.animate` utilise par défaut une courbe de ce genre.

## Les rebonds

### Dépasser le but

La progression n'est pas limitée à `1.0`. À `1.1`, le calcul donne une valeur 10 % au-delà du but.
Donc une courbe qui monte au-dessus de `1.0` avant de redescendre fait aller la fenêtre ou la balle
trop loin, puis revenir :

```kova
fn past(t: Float, strength: Float) -> Float {
    let back: Float := t - 1.0
    return 1.0 + (strength + 1.0) * back.pow(3) + strength * back.pow(2)
}

fn main {
    for strength in [1.0, 1.7, 3.0] {
        var highest: Float := 0.0
        for i in 0..=100 {
            highest := highest.max(past(i.to_float / 100.0, strength))
        }
        print("strength {strength}: goes up to {highest}")
    }
}
```

```text
strength 1.0: goes up to 1.037026
strength 1.7: goes up to 1.0998424
strength 3.0: goes up to 1.25
```

`back` va de `-1.0` à `0.0`, donc la courbe commence à `0.0` et finit à `1.0`. `strength` décide
jusqu'où elle monte : ci-dessus, `1.7` donne environ 10 %, et `3.0` donne 25 %.

Dépasser le but ne se voit pas quand le but est tout l'écran, parce que la fenêtre sort de l'écran.
Dans ce cas, le rebond doit rester à l'intérieur.

### Rester à l'intérieur

Une balle qui tombe touche le sol, remonte, le touche encore, et chaque rebond est plus petit. Trois
ingrédients font cette courbe :

1. `(t * Float.pi * 3.0).cos` est une vague, qui monte et descend.
2. `.abs` retourne vers le haut la moitié basse de la vague : le résultat est une suite d'arcs, entre
   `0.0` et `1.0`.
3. `(1.0 - t).pow(3)` va de `1.0` à `0.0` : il rend chaque arc plus petit que le précédent.

```kova
fn arcs(t: Float) -> Float {
    return (t * Float.pi * 3.0).cos.abs
}

fn fade(t: Float) -> Float {
    return (1.0 - t).pow(3)
}

fn bounce(t: Float) -> Float {
    return 1.0 - arcs(t) * fade(t)
}

fn main {
    for i in 0..=6 {
        let t: Float := i.to_float / 6.0
        print("{bounce(t)}")
    }
}
```

```text
0.0
1.0
0.7037037037037036
1.0
0.9629629629629629
1.0
1.0
```

À chaque point où les arcs arrivent à `0.0`, la courbe donne `1.0` : le but est touché. Entre ces
points, elle retombe. Comme `arcs(t) * fade(t)` n'est jamais négatif, la courbe ne dépasse jamais
`1.0`.

Chaque nombre est un réglage :

| Pour changer | Change |
| --- | --- |
| combien de fois le but est touché | `3.0` : le nombre d'arcs |
| à quelle vitesse les rebonds s'éteignent | le `3` de `pow(3)` : plus haut s'éteint plus vite |
| la durée totale | le temps donné à l'animation |

Sans `.abs`, la vague garde ses deux moitiés, et la courbe passe des deux côtés du but, comme un
ressort :

```kova
fn spring(t: Float) -> Float {
    return 1.0 - (t * Float.pi * 5.0).cos * (1.0 - t).pow(3)
}

fn main {
    for i in 0..=5 {
        print("{spring(i.to_float / 5.0)}")
    }
}
```

```text
0.0
1.512
0.784
1.064
0.992
1.0
```

Prends le temps de jouer avec ces nombres : un tableau affiché par un petit programme montre tout de
suite ce que change un réglage, avant même d'ouvrir une fenêtre.

## Déplacer la fenêtre

`ui.animate` pose les trois mêmes questions : `time` pour la durée, la place de la fenêtre et une
`ui.Area` pour le départ et le but, et `curve` pour la façon. Il s'occupe des pas et de l'horloge, et
reconstruit la vue à chaque image, donc un canvas `ui.fill` suit la fenêtre.

`curve` prend un bloc, ou le nom d'une fonction. Une courbe construite plus haut s'utilise donc en une
ligne :

```kova
import ui

enum Event {
    Grow
}

fn bounce(t: Float) -> Float {
    let arcs: Float := (t * Float.pi * 3.0).cos.abs
    return 1.0 - arcs * (1.0 - t).pow(3)
}

fn view(n: Int) -> ui.View {
    return ui.button("Grow", Event.Grow)
}

fn update(n: var Int, e: Event) {
    match e {
        Grow => ui.animate(ui.screen) time(1200) curve(bounce)
    }
}

fn main {
    ui.run("Grow", 0, view, update)
}
```

```text
[Grow] button "Grow"
```

Le but est n'importe quelle `ui.Area`, donc c'est là que se décide le mouvement. Une taille gardée au
centre de l'écran :

```kova
fn centered(width: Int, height: Int) -> ui.Area {
    let s: ui.Area := ui.screen
    return ui.Area { x := s.x + (s.width - width) / 2, y := s.y + (s.height - height) / 2, width, height }
}
```

Un glissement vers la droite, sans changer la taille de la fenêtre :

```kova
let w: ui.Area := ui.window
ui.animate(ui.Area { x := w.x + 300, y := w.y, width := w.width, height := w.height }) time(500)
```

Le programme continue d'entendre les clics pendant une animation, et un nouveau `ui.animate` remplace
celui en cours, en partant de là où est la fenêtre. Un bouton qui agrandit et réduit devient donc
simple : il n'a qu'à choisir son but.

Sous Linux, `ui.animate` ne fait rien pour l'instant, comme `ui.place` : voir
[Déplacer et redimensionner la fenêtre toi-même](/fr/guide/windows/#déplacer-et-redimensionner-la-fenêtre-toi-même).

## Quand `ui.animate` ne suffit pas

`ui.animate` déplace `x`, `y`, `width` et `height` ensemble, en ligne droite, avec une seule courbe.
Pour tout le reste, le programme fait lui-même chaque image avec une horloge, comme pour la balle.

### Faire les choses l'une après l'autre

L'astuce est de couper le temps en parts. `part` donne la progression d'un morceau de l'animation :
`0.0` avant son début, `1.0` après sa fin :

```kova
fn part(t: Float, begin: Float, end: Float) -> Float {
    return ((t - begin) / (end - begin)).max(0.0).min(1.0)
}

fn main {
    for i in 0..=4 {
        let t: Float := i.to_float / 4.0
        print("{t}: first {part(t, 0.0, 0.5)}, then {part(t, 0.5, 1.0)}")
    }
}
```

```text
0.0: first 0.0, then 0.0
0.25: first 0.5, then 0.0
0.5: first 1.0, then 0.0
0.75: first 1.0, then 0.5
1.0: first 1.0, then 1.0
```

Cette fenêtre grandit en largeur pendant la première moitié, puis en hauteur pendant la seconde :

```kova
import ui

enum Event {
    Grow
    Tick
}

let steps: Int := 60

fn part(t: Float, begin: Float, end: Float) -> Float {
    return ((t - begin) / (end - begin)).max(0.0).min(1.0)
}

fn between(a: Int, b: Int, e: Float) -> Int {
    return a + ((b - a).to_float * e).to_int
}

fn view(step: Int) -> ui.View {
    return ui.button("Grow", Event.Grow).every(16, Event.Tick)
}

fn update(step: var Int, e: Event) {
    match e {
        Grow => step := 1
        Tick => {
            if step > 0 && step <= steps {
                let t: Float := step.to_float / steps.to_float
                let s: ui.Area := ui.screen
                let width: Int := between(320, s.width, part(t, 0.0, 0.5))
                let height: Int := between(200, s.height, part(t, 0.5, 1.0))
                ui.place(s.x, s.y) size(width, height)
                step := step + 1
            }
        }
    }
}

fn main {
    ui.run("Grow", 0, view, update)
}
```

```text
[Grow] button "Grow" every(16)
```

Les parts peuvent se chevaucher : `part(t, 0.0, 0.7)` et `part(t, 0.3, 1.0)` donnent une largeur qui
commence avant et une hauteur qui finit après. Et chaque part peut passer par sa propre courbe :
`smooth(part(t, 0.0, 0.5))`.

### Un trajet qui n'est pas une ligne

Chaque coordonnée a son propre calcul. Ici `x` traverse l'écran à vitesse constante pendant que `y`
suit un sinus, qui monte et descend une fois entre `0.0` et `Float.pi`. La fenêtre dessine une arche :

```kova
let t: Float := step.to_float / steps.to_float
let s: ui.Area := ui.screen
let x: Int := s.x + ((s.width - 400).to_float * t).to_int
let y: Int := s.y + 300 - (300.0 * (t * Float.pi).sin).to_int
ui.place(x, y) size(400, 300)
```

### Animer autre chose

`progression` peut piloter n'importe quel nombre : une taille, une épaisseur, une position dans un
canvas, ou une couleur, en changeant chacune de ses trois parts :

```kova
fn between(a: Int, b: Int, e: Float) -> Int {
    return a + ((b - a).to_float * e).to_int
}

fn mixed(e: Float) -> ui.Color {
    let safe: Float := e.max(0.0).min(1.0)
    return ui.rgb(between(37, 220, safe), between(99, 38, safe), between(235, 38, safe))
}
```

`e.max(0.0).min(1.0)` est là parce qu'un rebond peut donner plus que `1.0`, et `ui.rgb` arrête le
programme sur une valeur au-dessus de 255. Une position peut dépasser son but, pas une couleur.

### Arrêter l'horloge

Une horloge qui continue de tourner reconstruit la vue 60 fois par seconde pour rien. Ne la demande
que pendant l'animation :

```kova
fn view(step: Int) -> ui.View {
    let page: ui.View := ui.button("Go", Event.Start)
    if step > 0 && step < steps {
        return page.every(16, Event.Tick)
    }
    return page
}
```

### Des durées exactes

Compter les pas suppose que chaque image prend 16 millisecondes. Une vue plus longue à construire
allonge l'animation, sans sauter d'image. `ui.animate` se comporte de la même façon.

Quand la durée compte plus que chaque image, mesure-la. `now` donne les secondes depuis 1970, avec
une fraction. Garde le départ dans l'état, et calcule `t` depuis le temps réellement écoulé :

```kova
struct App {
    started: Float
    running: Bool
}

fn update(app: var App, e: Event) {
    match e {
        Start => {
            app.started := now
            app.running := true
        }
        Tick => {
            if app.running {
                let t: Float := ((now - app.started) / 0.8).min(1.0)
                ui.place(100, 100) size(between(320, 800, smooth(t)), 400)
                app.running := t < 1.0
            }
        }
    }
}
```

`.min(1.0)` arrête `t` à la fin, et cette animation dure 0,8 seconde quelle que soit la lenteur de
l'ordinateur : un ordinateur plus lent montre moins d'images.

## Plusieurs choses à la fois

Chaque objet a besoin de son propre timing, pas de sa propre horloge. Un seul `t` pilote tout, et
`part` donne à chaque objet son morceau. Ici cinq balles partent l'une après l'autre. Chacune démarre
0,1 après la précédente, et prend 0,6 du temps :

```kova
import ui

enum Event {
    Start
    Tick
}

let steps: Int := 90

fn part(t: Float, begin: Float, end: Float) -> Float {
    return ((t - begin) / (end - begin)).max(0.0).min(1.0)
}

fn slow_down(t: Float) -> Float {
    return t * (2.0 - t)
}

fn view(step: Int) -> ui.View {
    let t: Float := step.to_float / steps.to_float
    return ui.column([
        ui.button("Go", Event.Start),
        ui.canvas(400, 220) { d: ui.Draw =>
            d.clear(ui.white)
            for i in 0..5 {
                let begin: Float := i.to_float * 0.1
                let e: Float := slow_down(part(t, begin, begin + 0.6))
                d.circle(40 + (320.0 * e).to_int, 30 + i * 40) radius(15) color(ui.blue)
            }
        },
    ]).every(16, Event.Tick)
}

fn update(step: var Int, e: Event) {
    match e {
        Start => step := 1
        Tick => {
            if step > 0 && step < steps {
                step := step + 1
            }
        }
    }
}

fn main {
    ui.run("Balls", 0, view, update)
}
```

```text
[Balls] column(button "Go", canvas 400x220 clear(#ffffff) circle(40 30 15 #2563eb 0) circle(40 70 15 #2563eb 0) circle(40 110 15 #2563eb 0) circle(40 150 15 #2563eb 0) circle(40 190 15 #2563eb 0)) every(16)
```

Changer `0.1` change l'écart entre les balles, et `0.6` la durée de chacune. Vérifie que la dernière
finit à temps : elle commence à `4 * 0.1`, et `0.4 + 0.6` fait `1.0`. Une somme au-dessus de `1.0`
veut dire que la dernière balle est coupée avant d'arriver.

## Un mouvement sans courbe : la physique

Une courbe connaît tout le mouvement à l'avance. Ce n'est pas ce qu'il faut pour quelque chose qui
réagit : une balle qu'on relance en plein vol, ou qui heurte un mur. La physique marche autrement :
l'état garde où est l'objet **et à quelle vitesse il va**, et chaque image calcule la suivante à
partir de ça.

Deux lignes suffisent, toujours dans cet ordre :

1. La vitesse change : `speed := speed + gravity`.
2. La position change : `position := position + speed`.

Cette balle tombe, et perd une partie de sa vitesse à chaque fois qu'elle touche le sol :

```kova
import ui

enum Event {
    Drop
    Tick
}

struct Ball {
    y: Float
    speed: Float
}

let floor_at: Float := 170.0
let gravity: Float := 0.5
let kept: Float := 0.7

fn view(ball: Ball) -> ui.View {
    return ui.column([
        ui.button("Drop", Event.Drop),
        ui.canvas(200, 200) { d: ui.Draw =>
            d.clear(ui.white)
            d.circle(100, ball.y.to_int + 15) radius(15) color(ui.red)
        },
    ]).every(16, Event.Tick)
}

fn update(ball: var Ball, e: Event) {
    match e {
        Drop => {
            ball.y := 0.0
            ball.speed := 0.0
        }
        Tick => {
            ball.speed := ball.speed + gravity
            ball.y := ball.y + ball.speed
            if ball.y > floor_at {
                ball.y := floor_at
                ball.speed := -ball.speed * kept
            }
        }
    }
}

fn main {
    ui.run("Ball", Ball { y := floor_at, speed := 0.0 }, view, update)
}
```

```text
[Ball] column(button "Drop", canvas 200x200 clear(#ffffff) circle(100 185 15 #dc2626 0)) every(16)
```

`gravity` est ce que la vitesse gagne à chaque image. `kept` est ce qui reste après le sol : `1.0`
rebondit pour toujours, `0.0` ne rebondit pas du tout. Le frottement est une ligne de plus,
`speed := speed * 0.98`, et un ressort est une vitesse tirée vers le but,
`speed := speed + (goal - position) * 0.1`, avec du frottement pour le calmer.

Rien ici ne dépend du temps restant : appuyer sur `Drop` pendant un rebond repart simplement de
l'état. En revanche, la physique n'a pas de durée. Le mouvement finit quand la vitesse s'est éteinte,
donc il se règle en essayant, et le même petit programme sans fenêtre le montre :

```kova
fn main {
    var y: Float := 0.0
    var speed: Float := 0.0
    var highest: Float := 170.0
    var going_up: Bool := false
    for frame in 0..400 {
        speed := speed + 0.5
        y := y + speed
        if y > 170.0 {
            y := 170.0
            speed := -speed * 0.7
            going_up := true
        }
        if going_up && speed > 0.0 {
            print("frame {frame}: rises to {170.0 - y}")
            going_up := false
        }
    }
}
```

```text
frame 44: rises to 77.89999999999989
frame 74: rises to 35.48999999999987
frame 94: rises to 13.850999999999885
frame 107: rises to 6.136199999999917
frame 116: rises to 2.6364399999999932
frame 122: rises to 0.34086899999996945
```

## Construire une animation complexe

Ne commence pas par le code. Une animation complexe est une suite d'animations simples, donc
construis-la dans cet ordre :

1. **Écris la chronologie en mots.** « Le menu s'ouvre en 0,4 s ; à partir de 0,2 s, chaque ligne
   glisse, une toutes les 0,05 s ; la dernière rebondit. » Une phrase que tu ne sais pas écrire est un
   mouvement que tu n'as pas décidé.
2. **Transforme chaque moment en part de `t`.** Divise les temps par le total. Avec 1 seconde en tout,
   le menu est `part(t, 0.0, 0.4)`, la première ligne `part(t, 0.2, 0.5)`.
3. **Choisis ce que chaque part déplace**, avec `valeur = départ + (but - départ) * progression`, une
   ligne pour chacune.
4. **Choisis une courbe pour chaque part**, en commençant sans aucune. Règle d'abord le timing à
   vitesse constante : les courbes cachent les erreurs de timing.
5. **Ralentis tout pour regarder.** Multiplie `steps` par 5 pendant que tu travailles : ce qui se passe
   en 16 millisecondes ne se juge pas.

Pour que ça reste lisible, mets le calcul de chaque objet dans une fonction de `t`, comme
`fn menu_height(t: Float) -> Int`. `view` ne fait plus que dessiner, et chaque fonction peut s'afficher
en tableau sans ouvrir de fenêtre. Un état avec un `enum` pour la phase (`Closed`, `Opening`, `Open`,
`Closing`) dit quelle animation tourne et quels clics comptent : un clic sur un menu en train de
s'ouvrir peut lancer `Closing` depuis là où est le menu, si l'état garde la valeur actuelle comme
prochain départ.

Pour un mouvement qui dépend de ce qui arrive, un clic ou une collision, utilise la physique. Pour un
mouvement qui doit durer un temps précis et finir à un endroit précis, utilise une courbe. Les deux
peuvent vivre dans le même programme : chaque objet choisit le sien.

## En bref

- `valeur = départ + (but - départ) * progression`, à chaque image.
- Une courbe transforme le temps écoulé en progression. Affiche ses valeurs pour la voir.
- `1.0 - c(1.0 - t)` inverse une courbe. Diviser le temps en joint deux.
- Au-dessus de `1.0`, la valeur dépasse son but. `cos`, `abs` et un affaiblissement font des rebonds
  qui restent à l'intérieur.
- `ui.animate` pour le déplacement en ligne droite d'une fenêtre, avec ta courbe. Une horloge et
  `ui.place` pour tout le reste.
