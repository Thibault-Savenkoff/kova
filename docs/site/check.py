#!/usr/bin/env python3
"""Checks that the library pages name every method and function Kova has, so a new one cannot be
left out of the documentation. Kova itself says what exists: its help lists the methods of each
type, and `types::LIBRARY` the functions. Run before every build (`npm run build`); needs `kova`.
"""
import os, pathlib, re, subprocess, sys, tempfile

here = pathlib.Path(__file__).resolve().parent
root = here.parent.parent
pages = "\n".join(p.read_text() for p in (here / "src/content/docs/library").glob("*.md"))
env = dict(os.environ, KOVA_LANG="en", NO_COLOR="1")

# (type, a declaration of `r` of that type)
TYPES = [
    ("String", 'let r: String := "a"'), ("Int", "let r: Int := 1"), ("Float", "let r: Float := 1.5"),
    ("Bool", "let r: Bool := true"), ("Char", "let r: Char := 'a'"), ("BigInt", "let r: BigInt := 5"),
    ("Date", "let r: Date := Date.year(2026) month(1) day(1)"), ("DateTime", "let r: DateTime := DateTime.now"),
    ("List", "let r: List<Int> := [1]"), ("Map", 'let r: Map<String, Int> := ["a" := 1]'),
    ("Set", "let r: Set<Int> := Set[1]"),
]

def methods(decl):
    with tempfile.TemporaryDirectory() as tmp:
        f = pathlib.Path(tmp) / "p.kova"
        f.write_text(f"fn main {{\n    {decl}\n    r.zzz\n}}\n")
        out = subprocess.run(["kova", "run", str(f)], capture_output=True, text=True, env=env, timeout=30).stderr
    m = re.search(r"help: methods of `.*?`: (.*)", out)
    return [n.split("(")[0] for n in re.findall(r"`([^`]+)`", m.group(1))] if m else []

missing = []
for ty, decl in TYPES:
    names = methods(decl)
    if not names:
        sys.exit(f"check.py: kova gave no methods for {ty}: is it installed?")
    for name in names:
        # `each workers(…)` is written `each workers(n)`.
        if not re.search(r"\." + re.escape(name.split(" ")[0]) + r"\b", pages):
            missing.append(f"{ty}.{name}")
table = re.search(r"pub const LIBRARY[^=]*=\s*&\[(.*?)\n    \];", (root / "src/types.rs").read_text(), re.S).group(1)
for name in re.findall(r'\("([^"(]+)', table):
    if not re.search(r"(?<![\w.])" + re.escape(name) + r"\b", pages):
        missing.append(name)
if missing:
    sys.exit("Not in docs/site/src/content/docs/library: " + ", ".join(sorted(set(missing))))
print("check.py: every method and function is documented")
