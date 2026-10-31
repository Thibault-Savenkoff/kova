---
title: xml
description: Reading and writing XML.
sidebar:
  order: 20
---

```kova
import xml
```

`xml.parse(text) -> xml.Node or xml.Error` reads a document into a tree. Each `xml.Node` is an
element, and you build one like any struct to write XML.

```kova
import xml

fn main {
    let text: String := """
    <library>
        <book id="1"><title>Dune</title></book>
        <book id="2"><title>Emma</title></book>
    </library>
    """
    let root: xml.Node := xml.parse(text)?
    for book in root.all("book") {
        print("{book.attributes["id"] else "?"}: {book.first("title")?.text}")
    }
    let made: xml.Node := xml.Node { name := "note", attributes := ["to" := "Ada"], text := "1 < 2" }
    print(made.to_xml)
}
```

```text
1: Dune
2: Emma
<note to="Ada">1 &lt; 2</note>
```

| Member | What it gives |
| --- | --- |
| `n.name -> String` | the element's name. |
| `n.attributes -> Map<String, String>` | its attributes. |
| `n.children -> List<xml.Node>` | the elements inside it. |
| `n.text -> String` | the text directly inside it, all its pieces joined, without the spaces around. |
| `n.all(name) -> List<xml.Node>` | its children of that name. |
| `n.first(name) -> xml.Node?` | the first of them. |
| `n.to_xml -> String` | the element as text, with `<`, `>`, `&` and `"` written safely. |

`attributes`, `children` and `text` may be left out when you make a `Node`. A `xml.Error` holds `line`
and `message`, and `e.to_string` writes both. Comments, `<?…?>`, `<!DOCTYPE …>` and `<![CDATA[…]]>`
are read; entities other than the five of XML and the numbered ones are refused, and a `DOCTYPE` is
skipped without being followed, so a document cannot make the program read a file.
