#!/usr/bin/env python3
"""Writes the reference chapters of the documentation, from Kova itself so that they stay exact:

- library.md: every method of every built-in type, found by asking the compiler (its help lists
  the methods; a wrong call tells the number and the types of the arguments and the result), and
  every function of the modules (`types::LIBRARY`);
- language.md: the keywords and symbols (`src/lexer.rs`);
- errors.md: every message of `locales/en.kova`.
Run by build_site.py.
"""
import pathlib, re, subprocess, tempfile, os

root = pathlib.Path(__file__).resolve().parent.parent
env = dict(os.environ, KOVA_LANG="en", NO_COLOR="1")
tmp = pathlib.Path(tempfile.mkdtemp())

# (type shown, a declaration of `r` of that type)
TYPES = [
    ("String", 'let r: String := "a"'), ("Int", "let r: Int := 1"), ("Float", "let r: Float := 1.5"),
    ("Bool", "let r: Bool := true"), ("Char", "let r: Char := 'a'"), ("BigInt", "let r: BigInt := 5"),
    ("Date", "let r: Date := Date.year(2026) month(1) day(1)"),
    ("DateTime", "let r: DateTime := DateTime.now"),
    ("List<Int>", "let r: List<Int> := [1]"), ("Map<String, Int>", 'let r: Map<String, Int> := ["a" := 1]'),
    ("Set<Int>", "let r: Set<Int> := Set[1]"), ("Int?", "let r: Int? := 1"),
    ("String or IoError", 'let r: String or files.IoError := ok("a")'),
]

def run(body, imports=""):
    f = tmp / "p.kova"
    f.write_text(f"{imports}fn main {{\n    {body}\n}}\n")
    return subprocess.run(["kova", "run", str(f)], capture_output=True, text=True, env=env, timeout=30).stderr

def methods(decl, imports):
    out = run(f"{decl}\n    r.zzz", imports)
    m = re.search(r"help: methods of `.*?`: (.*)", out)
    return re.findall(r"`([^`]+)`", m.group(1)) if m else []

def signature(decl, imports, name):
    call = name.split("(")[0]
    if "(…)" in name:  # a keyword message: its parts are not probed
        return name, ""
    out = run(f"{decl}\n    r.{call}(" + ", ".join(["true"] * 9) + ")", imports)
    n = re.search(r"takes (\d+) argument", out)
    n = int(n.group(1)) if n else 0
    args = ", ".join(["true"] * n)
    out = run(f"{decl}\n    let x: Bool := r.{call}" + (f"({args})" if n else ""), imports)
    params = re.findall(r"`([^`]+)` expected, `Bool` given", out)
    result = re.findall(r"`Bool` expected, `([^`]+)` given", out)
    p = "(" + ", ".join(params if len(params) == n else ["?"] * n) + ")" if n else ""
    # No type clash on the result: a `Bool` (unless the call failed for another reason: a block).
    if not result and (params or n == 0) and "`Bool` expected" not in out:
        result = ["Bool"]
    return f"{call}{p}", result[0] if result else ""

def library():
    lines = ["# Library reference", "",
             "Every method of the built-in types, then the functions of the modules. A method is called "
             "`value.name(arguments)`; with no arguments, `value.name`. Generated from the compiler by "
             "`docs/reference.py`.", ""]
    for ty, decl in TYPES:
        imports = "import files\n" if "files." in decl else ""
        names = methods(decl, imports)
        if not names:
            continue
        lines += [f"## `{ty}`", "", "| Method | Result |", "| --- | --- |"]
        for n in names:
            sig, res = signature(decl, imports, n)
            lines.append(f"| `{sig}` | {'`' + res + '`' if res else ''} |")
        lines.append("")
    src = (root / "src/types.rs").read_text()
    table = re.search(r"pub const LIBRARY[^=]*=\s*&\[(.*?)\n    \];", src, re.S).group(1)
    lines += ["## Functions", "", "Without a module name, a function is always there; the others need `import module`.", "",
              "| Function | Signature |", "| --- | --- |"]
    for name, sig in re.findall(r'\("([^"]+)", "([^"]+)"\)', table):
        lines.append(f"| `{name}` | `{sig.removeprefix('fn ')}` |")
    return "\n".join(lines) + "\n"

def language():
    lex = (root / "src/lexer.rs").read_text()
    words = re.findall(r'"([a-z_]+)"', re.search(r"KEYWORDS.*?= &\[(.*?)\];", lex, re.S).group(1))
    syms = re.findall(r'"([^"]+)"', re.search(r"SYMBOLS.*?= &\[(.*?)\];", lex, re.S).group(1))
    return f"""# Language reference

## Keywords

These words are reserved: no variable, function, parameter or field can take them as a name. After a
dot, a keyword is a name like another (`files.copy(a) to(b)`).

{", ".join(f"`{w}`" for w in words)}

`lang`, `extern`, `mode`, `skip`, `is` and `as` mean something only where they are expected; they may be names elsewhere.

## Symbols

{" ".join(f"`{s}`" for s in syms)}

| Symbols | Meaning |
| --- | --- |
| `:=` | gives a value: `let x: Int := 1`, `x := 2` (`=` only compares) |
| `+= -= *= /=` | changes a variable in place |
| `= != < <= > >=` | comparisons (`=` is equality, never assignment) |
| `and or !` | logic (`&&` and `||` are also read) |
| `..` `..=` | a range: end excluded / included |
| `->` | a function's result type |
| `=>` | the result of a `match` case or of a block |
| `?` | a missing value (`Int?`) or the early exit of an error (`x?`) |

## Names

A variable, a function, a field or a parameter starts with a lowercase letter; a type (`struct`,
`enum`, `interface`) with a capital. A type is written wherever a name is introduced, unless the
value written on the right shows it (`let rex := Dog {{ … }}`). A function without parameters has no
`()`: `fn main`, `rex.sound`, `x.to_float`.

## Ownership in four rules

1. A value has one owner. Giving it away (`let b := a`, passing it to `move`, `return`) empties the
   first name.
2. A call borrows what it is given, read-only, unless the call says otherwise at the place: `f(var a)`
   lets `f` change `a`, `f(move a)` gives `a` away.
3. A borrow never leaves its block: a reference is never stored or returned, so there are no lifetimes.
4. `copy struct` values are copied instead of moved; `.clone` copies any value on purpose.

Chapter 7 of the guide shows each rule with a program.
"""

def errors():
    text = (root / "locales/en.kova").read_text()
    lines = ["# Messages", "",
             "Every message Kova can show, as written in `locales/en.kova` (`{name}` stands for what the "
             "program has there). Run `kova` in French with `KOVA_LANG=fr`.", "",
             "| Key | Message |", "| --- | --- |"]
    for key, msg in re.findall(r'^\s{4}([a-z0-9_]+) := "(.*)"$', text.split("lang fr")[0], re.M):
        msg = msg.replace("\\\"", '"').replace("|", "\\|")
        lines.append(f"| `{key}` | {msg} |")
    return "\n".join(lines) + "\n"

def write(dest):
    (dest / "library.md").write_text(library())
    (dest / "language.md").write_text(language())
    (dest / "errors.md").write_text(errors())

if __name__ == "__main__":
    write(pathlib.Path("."))
