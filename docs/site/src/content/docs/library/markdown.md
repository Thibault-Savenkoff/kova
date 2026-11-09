---
title: markdown
description: Reading Markdown as blocks, plain text or HTML.
sidebar:
  order: 27
---

```kova
import markdown
```

Markdown is what people write notes, readmes and messages in: `# Title`, `**bold**`, `- item`. `markdown` reads
it for you. It cuts a text into **blocks** that you show the way you want (in a window, a terminal, a page), or
writes it all as HTML.

```kova
import markdown

fn main {
    let text: String := """
# Shopping

Things to **buy** this week:

- bread
- `milk` & eggs
"""
    for block in markdown.blocks(text) {
        print(block)
    }
    print(markdown.html(text))
}
```

```text
heading 1: Shopping
paragraph: Things to **buy** this week:
bullet: bread
bullet: `milk` & eggs
<h1>Shopping</h1>
<p>Things to <strong>buy</strong> this week:</p>
<ul>
<li>bread</li>
<li><code>milk</code> &amp; eggs</li>
</ul>
```

## The blocks

| Block | Written |
|---|---|
| `.Heading(level, text)` | `# Title`, up to `######` |
| `.Paragraph(text)` | lines that follow each other, joined with a space |
| `.Bullet(text)` | `- item`, `* item`, `+ item` |
| `.Numbered(number, text)` | `3. item` |
| `.Quote(text)` | `> said` |
| `.Code(text)` | the lines between two lines of three backticks, as they are written |
| `.Rule` | `---`, `***` or `___` |
| `.Image(alt, address)` | a line that is only `![alt](address)` |

A `Block` shows as a line of text (`heading 1: Shopping`), so `print(block)` is enough to look at one.

## Functions

| Written | Gives |
|---|---|
| `markdown.blocks(text) -> List<Block>` | the blocks of `text`, in order |
| `markdown.plain(line) -> String` | `line` without its marks: `**bold** and [a link](x.com)` becomes `bold and a link` |
| `markdown.inline_html(line) -> String` | `line` as HTML: `**bold**` → `<strong>`, `*stressed*` → `<em>`, `` `code` `` → `<code>`, `[label](address)` → `<a href>` |
| `markdown.html(text) -> String` | the whole text as HTML, the items of a list in one `<ul>` or `<ol>` |

The text of a block keeps its marks inside a line, so you choose: `plain` to show it in a place that has no
bold, `inline_html` to keep them.

## What it reads

Headings, paragraphs, lists (one level), numbered lists, quotes, code between backticks lines, rules; inside a
line, `**bold**`, `*stressed*`, `` `code` ``, `~~struck~~` (the marks are taken off) and `[links](address)`. Tables,
images and lists inside lists are left as plain paragraphs.
