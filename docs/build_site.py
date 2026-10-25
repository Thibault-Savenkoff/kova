#!/usr/bin/env python3
"""Makes docs/site/index.html, the documentation website, from docs/guide.md.

One self-contained page: the guide's text is inside it and drawn in the browser by `marked`
(loaded from cdnjs). Run again after editing the guide; open the file or put docs/site/ on any
static host (GitHub Pages: publish that folder).
"""
import json, pathlib

here = pathlib.Path(__file__).parent
guide = (here / "guide.md").read_text(encoding="utf-8")
page = (here / "site_template.html").read_text(encoding="utf-8")
out = here / "site" / "index.html"
out.parent.mkdir(exist_ok=True)
out.write_text(page.replace("__GUIDE__", json.dumps(guide).replace("</", "<\\/")), encoding="utf-8")
print(f"{out} ({out.stat().st_size // 1024} KB)")
