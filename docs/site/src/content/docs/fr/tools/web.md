---
title: Kova dans une page web
description: kova build --target web fait un seul fichier .html qui lance ton programme dans n'importe quel navigateur.
sidebar:
  order: 2
---

Un programme Kova peut tourner dans un navigateur. Rien à installer, aucun serveur à lancer :

```console
$ kova build compteur.kova --target web
✓ compteur.html (Web, 141 Ko) en 13 ms
```

`compteur.html` est un seul fichier. Ouvre-le d'un double clic, envoie-le à quelqu'un ou dépose-le
chez n'importe quel hébergeur : le programme est dedans, compilé en WebAssembly, et il tourne sur la
machine du visiteur.

Le même programme se compile pour Linux, Windows, macOS et le web. Ce qui change, c'est ce que la
page lui donne à la place d'un système.

## Ce qu'un programme trouve dans une page

| Dans le programme | Dans la page |
|---|---|
| `print`, `write` | du texte dans la page ; les erreurs en rouge |
| `read_line` | un champ apparaît ; le programme attend Entrée |
| `system.sleep` | le programme attend, la page reste vivante |
| `import ui` | de vrais éléments de page : boutons, champs, listes, tableaux |
| `ui.canvas` | un `<canvas>` |
| `ui.gpu` | WebGL 2 : les shaders écrits pour OpenGL 3.3 sont adaptés à la compilation |
| `import sound` | le navigateur joue le WAV |
| `import files` | des fichiers dans la mémoire de la page |
| `web.get`, `web.post`, `web.send` | le `fetch` du navigateur |

Tout le reste (textes, List, Map, JSON, expressions régulières, dates, hachages, `secure`…) marche
comme partout. Les nombres sont identiques au dernier chiffre : c'est le même code qui les calcule.

## Attendre sans bloquer

Une page web a une règle : rien ne doit la bloquer, sinon elle se fige. Pourtant `system.sleep(1.0)`
attend, et `read_line` aussi. Kova les garde simples à écrire : quand une fonction intégrée doit
attendre, les fonctions qui y mènent notent où elles en sont et rendent la main, la page continue,
puis elles reviennent à la même ligne. Tu écris du code ordinaire.

Un long calcul sans aucune attente occupe quand même la page jusqu'au bout, et ce qu'il affiche
n'apparaît qu'à la fin. Pour montrer l'avancement, laisse un instant à la page : `system.sleep(0.0)`
ne suffit pas, une attente doit dépasser 0, donc `system.sleep(0.001)` de temps en temps.

## Fenêtres

`import ui` ne demande aucun changement. `ui.run` montre la vue dans la page, et le titre devient
celui de l'onglet.

- `frame(false)` enlève la marge autour du contenu. `width`, `height`, `ui.place` et `ui.minimize`
  ne font rien : une page ne déplace pas le navigateur. `ui.full_screen` le demande au navigateur.
- `ui.fill` prend la place de la fenêtre du navigateur, et un redimensionnement redessine.
- `ui.image` et `d.image` prennent une adresse relative à la page : mets l'image à côté du `.html`.

## Fichiers

Une page n'a pas de disque. `import files` travaille sur des fichiers gardés dans la mémoire de la
page : un programme peut les écrire puis les relire, créer des dossiers et les lister. Ils
disparaissent avec la page.

Deux portes mènent dehors :

- `ui.open_file` demande un fichier à l'utilisateur. Il arrive dans la mémoire de la page, et le
  chemin rendu s'ouvre avec `files.read` comme d'habitude.
- `ui.save_file` donne un nom. Écrire sous ce nom télécharge le fichier sur la machine de
  l'utilisateur.

`ui.open_folder` répond `none` : on ne peut pas donner un dossier à une page.

## Interroger d'autres sites

`web.get` passe par le navigateur, donc sa règle s'applique : un site ne répond à une page venue
d'une autre adresse que s'il l'autorise (CORS). S'il ne l'autorise pas, ou si la machine ne répond
pas, le résultat est `err(Refused(…))` : le navigateur ne distingue pas les deux. Ton propre
serveur, celui qui sert la page, répond toujours.

## Ce qu'une page ne peut pas faire

Ces choses sont refusées à la compilation, avec une ligne qui dit laquelle :

| Refusé | Pourquoi |
|---|---|
| `extern` | pas de bibliothèque C dans une page |
| `import threads`, `workers(n)` | pas encore dans une page |
| `web.serve`, `import net` | une page ne peut ni écouter, ni ouvrir des sockets bruts |
| `system.run` | une page ne peut pas lancer de programmes |

## Jusqu'où va la récursion

La pile est celle du navigateur, bien plus petite que celle d'un programme sur un système : environ
6 000 appels imbriqués dans une fonction simple, moins dans une fonction à nombreuses variables, et
cela varie selon le navigateur. Au-delà, le programme s'arrête avec une erreur et le code 101. Un
parcours profond qui marche sur un système peut demander une boucle ici.

## Taille et vitesse

`hello` pèse 141 Ko dans une page, et seul ce que le programme utilise est gardé. Mesurés sous node,
les programmes tournent environ 1,3 à 4 fois moins vite que le même programme compilé pour le système.

Pour mettre le module dans ta propre page, `-o programme.wasm` l'écrit seul. Il lui faut le
JavaScript que contient le `.html` pour tourner.
