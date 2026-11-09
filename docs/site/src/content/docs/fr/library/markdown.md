---
title: markdown
description: Lire du Markdown en blocs, en texte simple ou en HTML.
sidebar:
  order: 27
---

```kova
import markdown
```

Le Markdown est ce dans quoi on écrit des notes, des « lisez-moi » et des messages : `# Titre`, `**gras**`,
`- élément`. `markdown` le lit pour toi. Il découpe un texte en **blocs** que tu montres comme tu veux (dans une
fenêtre, un terminal, une page), ou l'écrit en entier en HTML.

```kova
import markdown

fn main {
    let text: String := """
# Courses

Choses à **acheter** cette semaine :

- pain
- `lait` & œufs
"""
    for block in markdown.blocks(text) {
        print(block)
    }
    print(markdown.html(text))
}
```

```text
heading 1: Courses
paragraph: Choses à **acheter** cette semaine :
bullet: pain
bullet: `lait` & œufs
<h1>Courses</h1>
<p>Choses à <strong>acheter</strong> cette semaine :</p>
<ul>
<li>pain</li>
<li><code>lait</code> &amp; œufs</li>
</ul>
```

## Les blocs

| Bloc | S'écrit |
|---|---|
| `.Heading(level, text)` | `# Titre`, jusqu'à `######` |
| `.Paragraph(text)` | des lignes qui se suivent, jointes par une espace |
| `.Bullet(text)` | `- élément`, `* élément`, `+ élément` |
| `.Numbered(number, text)` | `3. élément` |
| `.Quote(text)` | `> dit` |
| `.Code(text)` | les lignes entre deux lignes de trois accents graves, telles qu'elles sont écrites |
| `.Rule` | `---`, `***` ou `___` |
| `.Image(alt, address)` | une ligne qui est seulement `![alt](address)` |

Un `Block` s'affiche comme une ligne de texte (`heading 1: Courses`) : `print(block)` suffit pour en regarder un.

## Les fonctions

| Écrit | Donne |
|---|---|
| `markdown.blocks(text) -> List<Block>` | les blocs de `text`, dans l'ordre |
| `markdown.plain(line) -> String` | `line` sans ses marques : `**gras** et [un lien](x.com)` devient `gras et un lien` |
| `markdown.inline_html(line) -> String` | `line` en HTML : `**gras**` → `<strong>`, `*accentué*` → `<em>`, `` `code` `` → `<code>`, `[texte](adresse)` → `<a href>` |
| `markdown.html(text) -> String` | tout le texte en HTML, les éléments d'une liste dans un seul `<ul>` ou `<ol>` |

Le texte d'un bloc garde ses marques à l'intérieur d'une ligne, donc tu choisis : `plain` pour l'afficher là où il
n'y a pas de gras, `inline_html` pour les garder.

## Ce qu'il lit

Les titres, les paragraphes, les listes (sur un niveau), les listes numérotées, les citations, le code entre deux
lignes d'accents graves, les filets ; dans une ligne, `**gras**`, `*accentué*`, `` `code` ``, `~~barré~~` (les
marques sont retirées) et les `[liens](adresse)`. Les tableaux, les images et les listes dans des listes restent
de simples paragraphes.
