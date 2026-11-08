---
title: ios
description: Tout UIKit sur un iPhone ou un iPad, en envoyant des messages à des objets natifs.
sidebar:
  order: 26
---

```kova
import ios
```

`ui` te donne les contrôles dont la plupart des programmes ont besoin, dessinés par le système. `ios` sert
au reste : toutes les classes de tous les frameworks d'un iPhone ou d'un iPad, atteintes comme Smalltalk
atteint ses objets. Tu nommes une classe, tu envoies des messages par leur sélecteur, et ce qui revient
est un autre objet. Rien n'est enveloppé à l'avance, donc rien ne manque.

Cela ne marche que sur un iPhone ou un iPad (`kova build f.kova --target ios`). Un programme qui l'importe et
tourne ailleurs s'arrête avec un message qui le dit.

## Objets et messages

```kova
import ios

fn main {
    let view: ios.Object := ios.root.child("view")
    let label: ios.Object := ios.new("UILabel")
    label.send("setText:", [ios.text("Bonjour de Kova")])
    label.send("setFrame:", [ios.rect(20.0, 200.0, 300.0, 40.0)])
    view.send("addSubview:", [label.value])
}
```

Le sélecteur est celui de la documentation d'Apple, deux-points compris. Chaque argument s'écrit pour ce que le
sélecteur veut, et Kova lit les types du sélecteur pour le lui remettre : un `Int` devient l'entier ou le booléen
demandé, un `Float` un `CGFloat`, un texte un `NSString`.

| Fonction | Ce qu'elle fait |
|---|---|
| `ios.class(name)` | la classe nommée `name`, ou `none` |
| `ios.get(name)` | la classe elle-même, pour lui envoyer des messages de classe (le programme s'arrête s'il n'y en a pas) |
| `ios.new(name)` | un nouvel objet de la classe `name` (le programme s'arrête s'il n'y en a pas) |
| `ios.root` | le contrôleur de vue de la fenêtre ; `root.child("view")` est l'écran |
| `o.send(selector, args)` | envoie un message, donne un `Value` |
| `o.call(selector)` | pareil sans argument |
| `o.child(selector)` | pareil, pour un message qui rend un objet |
| `o.value` | l'objet écrit comme argument |
| `o.release` | laisse partir l'objet |
| `ios.text(s)`, `ios.number(n)`, `ios.decimal(x)`, `ios.flag(b)`, `ios.selector(name)` | des arguments |
| `ios.rect(x, y, width, height)` | un `CGRect` ; `.Numbers([…])` écrit un point, une taille ou des marges |
| `ios.handler(n)` | un objet qui te prévient quand on l'appelle |
| `ios.wait` | attend qu'un handler soit appelé et donne son numéro |

Un `Value` est `.Nothing`, `.Int`, `.Float`, `.Bool`, `.Text`, `.Object`, `.Selector` ou `.Numbers`. Ce qu'un
message rend est lu dans sa signature, et `.object`, `.int` et `.text` arrêtent le programme avec un message
clair s'il a rendu autre chose.

## Entendre un contrôle

Un contrôle appelle une cible et une action quand quelque chose arrive. `ios.handler(7)` est une cible qui fait
une seule chose : faire donner 7 à `ios.wait`.

```kova
let tap: ios.Object := ios.handler(7)
button.send("addTarget:action:forControlEvents:", [tap.value, ios.selector("invoke:"), ios.number(64)])
while true {
    let n: Int := ios.wait
    // n vaut 7 : le bouton a été touché
}
```

## Des vues lisibles

Envoyer des messages par leur sélecteur atteint tout, mais les vues usuelles ont des méthodes qui se lisent.
Chacune garde l'objet natif qu'elle représente (`.object`) : ce qu'une méthode ne couvre pas reste à un
`send` près.

```kova
import ios

fn main {
    let screen: ios.View := ios.screen
    screen.set_background(ios.Color.white)

    let title: ios.Label := ios.Label.new("Bonjour")
    title.set_size(28.0)
    title.set_color(ios.Color.blue)
    title.view.set_frame(20.0, 80.0, 300.0, 40.0)
    screen.add(title.view)

    let button: ios.Button := ios.Button.new("Saluer")
    button.view.set_frame(20.0, 140.0, 120.0, 44.0)
    button.on_tap(1)
    screen.add(button.view)

    while true {
        if ios.wait = 1 {
            title.set_text("Touché")
        }
    }
}
```

| Type | Fabriqué avec | Méthodes |
|---|---|---|
| `ios.View` | `ios.View.new`, `ios.screen`, ou `.view` de n'importe quel type ci-dessous | `set_frame(x, y, width, height)`, `set_background(color)`, `set_alpha(a)`, `set_hidden(b)`, `set_corner(radius)`, `add(view)`, `remove`, `width`, `height`, `release` |
| `ios.Label` | `ios.Label.new(words)` | `set_text`, `text`, `set_color`, `set_size(points)`, `set_centered(b)`, `set_lines(n)`, `view` |
| `ios.Button` | `ios.Button.new(title)` | `set_title`, `on_tap(n)`, `view` |
| `ios.TextField` | `ios.TextField.new(hint)` | `text`, `set_text`, `on_change(n)`, `view` |
| `ios.Switch` | `ios.Switch.new` | `is_on`, `set_on(b)`, `on_change(n)`, `view` |
| `ios.Color` | `ios.Color.rgb(r, g, b)` (chacun de 0.0 à 1.0), `blue`, `red`, `green`, `gray`, `white`, `black`, `clear` | donné à `set_background`, `set_color` |

`on_tap(n)`, comme `on_change(n)`, fait que `ios.wait` donne `n` à chaque fois que cela arrive.

## À savoir

- Chaque message est envoyé sur le fil principal, comme UIKit le veut, donc le programme peut aller à son
  rythme.
- Un objet que tu fabriques reste en vie jusqu'à ce que tu le `release` ; les objets qu'un message rend sont
  gardés pour toi aussi. Un programme qui fabrique des milliers de vues doit relâcher celles qu'il abandonne.
- Une exception Objective-C (un mauvais sélecteur pour l'objet, une vue ajoutée deux fois) n'est pas attrapée :
  l'app écrit `error: NSInvalidArgumentException: Can't add self as subview` et s'arrête avec le code 101,
  comme en Swift. Kova vérifie que l'objet répond au sélecteur et que le nombre d'arguments est
  juste avant d'envoyer.
- `ui` et `ios` marchent ensemble : `ui.run` construit ses vues dans la même fenêtre, et `ios.root` les atteint.
