---
title: xml
description: Lire et écrire du XML.
sidebar:
  order: 20
---

```kova
import xml
```

`xml.parse(text) -> xml.Node or xml.Error` lit un document dans un arbre. Chaque `xml.Node` est un
élément, et tu en construis un comme n'importe quelle struct pour écrire du XML.

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

| Membre | Ce qu'il donne |
| --- | --- |
| `n.name -> String` | le nom de l'élément. |
| `n.attributes -> Map<String, String>` | ses attributs. |
| `n.children -> List<xml.Node>` | les éléments qu'il contient. |
| `n.text -> String` | le texte directement dedans, tous ses morceaux réunis, sans les espaces autour. |
| `n.all(name) -> List<xml.Node>` | ses enfants de ce nom. |
| `n.first(name) -> xml.Node?` | le premier d'entre eux. |
| `n.to_xml -> String` | l'élément en texte, avec `<`, `>`, `&` et `"` écrits sans danger. |

`attributes`, `children` et `text` peuvent être omis quand tu crées un `Node`. Un `xml.Error` contient
`line` et `message`, et `e.to_string` écrit les deux. Les commentaires, `<?…?>`, `<!DOCTYPE …>` et
`<![CDATA[…]]>` sont lus ; les entités autres que les cinq de XML et les numérotées sont refusées, et
un `DOCTYPE` est sauté sans être suivi, donc un document ne peut pas faire lire un fichier au
programme.
