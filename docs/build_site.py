#!/usr/bin/env python3
"""Makes the documentation website with mdBook (cargo install mdbook --locked).

docs/guide.md stays the one source (the tests run its examples): this cuts it into one chapter per
section in docs/book/src (generated), then `mdbook build` writes the site to docs/book/book.
"""
import pathlib, re, shutil, subprocess

book = pathlib.Path(__file__).parent / "book"
src = book / "src"
shutil.rmtree(src, ignore_errors=True)
src.mkdir(parents=True)
guide = (book.parent / "guide.md").read_text(encoding="utf-8")
intro, *sections = re.split(r"^(?=## \d+\. )", guide, flags=re.M)
(src / "README.md").write_text(intro.replace("# A tour of Kova", "# Introduction").strip() + "\n", encoding="utf-8")
summary = ["# Summary", "", "[Introduction](README.md)", ""]
for s in sections:
    n, title = re.match(r"## (\d+)\. (.*)", s).groups()
    name = f"{int(n):02d}-{re.sub('[^a-z0-9]+', '-', title.lower()).strip('-')}.md"
    (src / name).write_text(re.sub(r"^## \d+\. ", "# ", s, count=1).replace("\n### ", "\n## "), encoding="utf-8")
    summary.append(f"- [{n}. {title}]({name})")
(src / "SUMMARY.md").write_text("\n".join(summary) + "\n", encoding="utf-8")
subprocess.run(["mdbook", "build", str(book)], check=True)
