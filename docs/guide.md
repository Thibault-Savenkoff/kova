# A tour of Kova

This guide shows Kova through small programs. Every program here is checked by Kova's own tests: what is
written under it is exactly what it prints.

## 1. Hello

```kova
fn main {
    print("Hello, Kova!")
}
```

```text
Hello, Kova!
```

Save it as `hello.kova`, then:

- `kova run hello.kova` runs it at once (in Kova's interpreter);
- `kova build hello.kova` makes a native executable, `hello` (`hello.exe` on Windows), that runs anywhere
  without Kova. Add `--target linux`, `windows` or `mac` (every Mac) to make one for another system, or
  `linux-arm`, `windows-arm`, `mac-arm`, `mac-intel` for one processor.

## 2. Values and variables

```kova
fn main {
    let name: String := "Ada"
    var age: Int := 36
    age := age + 1
    age += 1
    print("{name} is {age}")
    print(age = 38)
    let pi: Float := 3.14159
    print(pi * 2.0)
}
```

```text
Ada is 38
true
6.28318
```

- `let` makes a value that never changes; `var` one that may.
- `:=` gives a value; `=` compares (there is no `==`).
- Types are written: `age: Int`. They may be left out only when the type's name is already on the right
  (`let rex := Dog { ... }`, below).
- The basic types: `Int` (64 bits), `Float`, `Bool`, `Char` (`'a'`), `String`, and `BigInt` for numbers
  without a limit. An `Int` that overflows stops the program with an error, it never wraps around.

## 3. Functions

```kova
fn square(x: Int) -> Int {
    return x * x
}

fn seven -> Int {
    return 7
}

fn main {
    print(square(seven))
    print(square(3).to_float / 2.0)
}
```

```text
49
4.5
```

A function without parameters has no `()`, at its declaration and when it is called: `seven`, `x.to_float`.

## 4. Choices and loops

```kova
fn sign(n: Int) -> String {
    return if n < 0 { "negative" } else if n = 0 { "zero" } else { "positive" }
}

fn main {
    print(sign(-4))
    for i in 0..3 {
        write(i)
    }
    print("")
    from i := 1 to 3 {
        write(i)
    }
    print("")
    var n: Int := 1
    while n < 100 {
        n := n * 3
    }
    print(n)
}
```

```text
negative
012
123
243
```

`if` gives a value. `0..3` stops before 3; `0..=3` and `from i := 1 to 3` include the end. `write` prints
without going to the next line.

## 5. Texts

```kova
fn main {
    let words: List<String> := "the quick brown fox".words
    print(words.length)
    print(words[1].to_upper)
    print("{words[0]}-{words[3]}")
    print("kova".repeat(2))
    print("a,b,c".split(",").join(" + "))
    print("2026-10-23".to_date)
}
```

```text
4
QUICK
the-fox
kovakova
a + b + c
2026-10-23
```

`{…}` inside a text puts a value in it. A raw text, between `"""`, keeps everything as written:
quotes, braces and `\` (`"""{"title": "Dune"}"""`). On several lines, it starts on the line after
its `"""`, and the closing `"""`, alone on its line, gives the indentation taken off every line.

## 6. Structs and methods

```kova
struct Dog {
    name: String
    age: Int

    fn describe(self) -> String {
        return "{self.name} ({self.age})"
    }

    fn birthday(var self) {
        self.age += 1
    }
}

fn main {
    var rex := Dog { name := "Rex", age := 3 }
    rex.birthday
    print(rex.describe)
    print(rex)
}
```

```text
Rex (4)
Dog { name: "Rex", age: 4 }
```

A method reads its value through `self`. To change it, it says so with `var self`.

## 7. Who owns what

Kova frees memory itself, without a garbage collector: every value has one owner, and is freed when its
owner goes away. A function **borrows** what it is given, unless the call says otherwise:

```kova
fn shout(s: String) -> String {
    return s.to_upper
}

fn add_one(l: var List<Int>) {
    l.add(1)
}

fn count(l: move List<Int>) -> Int {
    return l.length
}

fn main {
    let word: String := "hi"
    print(shout(word))
    print(word)
    var l: List<Int> := [5]
    add_one(var l)
    print(l)
    print(count(move l))
}
```

```text
HI
hi
[5, 1]
2
```

- `shout(word)` only borrows `word`: it is still there after.
- `add_one(var l)` may change `l`; the `var` is written at the call too, so nothing changes in secret.
- `count(move l)` takes `l`: using `l` after that line is an error, found before the program runs.
- `let b := a` gives `a` to `b`. To keep both, write `let b := a.clone`.

A borrow never outlives the line or the block it is in, so Kova needs no "lifetimes".

## 8. Lists, Maps and Sets

```kova
fn main {
    var scores: List<Int> := [12, 5, 18]
    scores.add(7)
    print(scores.sorted)
    print(scores[1..3])
    for s in var scores {
        s := s * 10
    }
    print(scores)

    var ages: Map<String, Int> := ["Ana" := 31, "Bo" := 25]
    ages["Cy"] := 40
    print(ages["Bo"])
    print(ages["Zed"])
    for name, age in ages {
        print("{name}: {age}")
    }

    let seen: Set<String> := Set["red", "blue", "red"]
    print(seen)
}
```

```text
[5, 7, 12, 18]
[5, 18]
[120, 50, 180, 70]
25
none
Ana: 31
Bo: 25
Cy: 40
Set["red", "blue"]
```

- `[…]` is a List, `[k := v]` a Map, `Set[…]` a Set (each value once, in the order added).
- `for s in var scores` changes each element; `for s in move scores` takes them (the List is used up).
- `ages["Zed"]` is an `Int?`: there may be no value (see section 10).

## 9. Enums and match

```kova
enum Shape {
    Circle(radius: Float)
    Rect(width: Float, height: Float)
    Dot
}

fn area(s: Shape) -> Float {
    return match s {
        Circle(r) => 3.0 * r * r
        Rect(w, h) => w * h
        Dot => 0.0
    }
}

fn size(n: Int) -> String {
    return match n {
        0 => "none"
        1..=9 => "small"
        _ => "big"
    }
}

fn main {
    let shapes: List<Shape> := [Shape.Circle(1.0), Shape.Rect(2.0, 3.0), Shape.Dot]
    for s in shapes {
        print(area(s))
    }
    print(size(4))
    let today := Date.year(2026) month(10) day(23)
    match today.weekday {
        Saturday, Sunday => print("weekend")
        _ => print("work")
    }
}
```

```text
3.0
6.0
0.0
small
work
```

A `match` must handle every case: forgetting one is an error, and the editor offers to add them.

## 10. Missing values and errors

A value that may be missing has a type ending in `?`; an operation that may fail gives a `T or E`:

```kova
fn half(n: Int) -> Int or String {
    if n % 2 != 0 {
        return err("{n} is odd")
    }
    return ok(n / 2)
}

fn quarter(n: Int) -> Int or String {
    let h: Int := half(n)?
    return half(h)
}

fn main {
    let age: Int? := "42".to_int
    if age != none {
        print(age + 1)
    }
    print("forty".to_int)
    match quarter(12) {
        ok(q) => print(q)
        err(e) => print("error: {e}")
    }
    match quarter(6) {
        ok(q) => print(q)
        err(e) => print("error: {e}")
    }
}
```

```text
43
none
3
error: 3 is odd
```

- Inside `if age != none`, `age` is an `Int`.
- `?` gives the error back to the caller at once.
- Reading a file gives a `String or files.IoError`: Kova never hides that something can go wrong.
- When the function returns its own error enum, `?` puts the error into the case holding one:

```kova
import files

enum AppError {
    Empty
    Io(error: files.IoError)
}

fn load(path: String) -> String or AppError {
    let text: String := files.read(path)?
    if text = "" {
        return err(AppError.Empty)
    }
    return ok(text)
}

fn main {
    match load("missing.txt") {
        ok(text) => print(text)
        err(e) => print(e)
    }
}
```

```text
AppError.Io(error: NotFound("missing.txt"))
```

## 11. Interfaces and generics

```kova
interface Animal {
    fn sound(self) -> String

    fn twice(self) -> String {
        return "{self.sound} {self.sound}"
    }
}

struct Dog : Animal {
    fn sound(self) -> String {
        return "woof"
    }
}

struct Cat : Animal {
    fn sound(self) -> String {
        return "meow"
    }
}

struct Pair<T> : Equal {
    first: T
    second: T
}

fn larger<T: Comparable>(a: T, b: T) -> T {
    if a < b {
        return b.clone
    }
    return a.clone
}

fn main {
    let zoo: List<any Animal> := [Dog {}, Cat {}]
    for a in zoo {
        print(a.twice)
        if a is Cat {
            print("a cat")
        }
    }
    print(larger(3, 8))
    print(larger("pear", "apple"))
    print(Pair<Int> { first := 1, second := 2 } = Pair<Int> { first := 1, second := 2 })
}
```

```text
woof woof
meow meow
a cat
8
pear
true
```

- An interface lists methods; a struct that follows it writes them, or takes the default ones.
- `any Animal` holds any value that follows `Animal`. `a is Cat` tells if it holds a `Cat`;
  inside `if a is Cat { … }`, `a` is that `Cat`.
- `T: Comparable` says what a generic type must allow. The built-in interfaces: `Equal` (`=`),
  `Comparable` (`<`), `Display` (its text, for `print` and `{…}`).

## 12. Blocks

```kova
fn main {
    let scores: List<Int> := [12, 5, 18, 7]
    print(scores.filter { x: Int => x > 10 })
    print(scores.map { x: Int => x * 2 })
    print(scores.fold(0) { total: Int, x: Int => total + x })
    print(scores.any { x: Int => x > 15 })
    var count: Int := 0
    scores.each { x: Int =>
        count += 1
    }
    print(count)
}
```

```text
[12, 18]
[24, 10, 36, 14]
42
true
4
```

A block is a piece of code given to a function, which runs it. It can read and change the variables around it.

```kova
fn main {
    let names: List<String> := ["ada", "alan", "grace"]
    let shout: String := "!"
    print(names.map workers(4) { name: String => name.to_upper + shout })
}
```

```text
["ADA!", "ALAN!", "GRACE!"]
```

`map workers(4) { … }` and `each workers(4) { … }` share the elements between 4 threads, so slow work
(waiting for a site, a long computation) is done several at a time. `map` still gives its results in
the List's order; `each` runs its elements in an order that changes from one run to the next (two
`print`s may come out swapped), and `random` draws differently. As with a server's `workers(…)`
(section 18), the block may read the values around it but not change them. `kova run` takes one
element at a time: only the built program uses threads.

## 13. Contracts and tests

```kova
fn divide(a: Int, b: Int) -> Int
    require b != 0 else "you cannot divide by zero"
    ensure result * b <= a
{
    return a / b
}

test "divide rounds down" {
    check divide(7, 2) = 3
}

fn main {
    print(divide(9, 2))
}
```

```text
4
```

- `require` says what a function needs, `ensure` what it promises, `check` what must be true. If one fails,
  the program stops with a message showing the values involved and where it was called from.
- `kova test file.kova` runs the `test` blocks; `kova run` and `kova build` leave them out.
- `kova build --boost` leaves the contracts out of the executable, for speed.

### Run or build

```kova
fn banner
    mode build else skip
{
    print("=== built ===")
}

fn workers_count -> Int
    mode build else return 1
{
    return 8
}

fn main {
    banner
    print(workers_count)
}
```

```text
1
```

Built, the same program prints `=== built ===` and `8`. `mode build` (or `mode run`) under a signature
says what a call of the function does in the other mode, where everyone reading the code sees it:

- `else skip`: nothing, the program goes on (only for a function that returns nothing);
- `else return value`: the call gives this value (`else return none` for a `T?`);
- `else "message"`: the program stops at the call with this message, like a `require` that fails;
- no `else`: the function runs in both modes, giving the same results. It is there for speed:
  `kova run` then compiles the whole program before running it (still the `kova run` version of it:
  the `else`s of `mode build` act), which it does in a few milliseconds.

In the other mode, the function's contracts are left out too, and its arguments are still computed
(a `move` argument is given away all the same). `kova test` counts as `kova run`.

## 14. Several files

```kova
// main.kova
import geometry as geo

fn main {
    let p := geo.Point { x := 3.0, y := 4.0 }
    print(geo.length(p))
}
```

```kova
// geometry.kova, next to main.kova
public:

struct Point {
public:
    x: Float
    y: Float
}

fn length(p: Point) -> Float {
    return (p.x * p.x + p.y * p.y).sqrt
}
```

```text
5.0
```

`kova run main.kova` reads `geometry.kova` too. What a file holds is private to it unless written under
`public:` (and a struct's fields too, inside it); `private:` hides again what follows.

## 15. Files, time and chance

```kova
import files

fn main {
    let saved: Nothing or files.IoError := files.write("note.txt", "hello\n")
    match files.read("note.txt") {
        ok(text) => write(text)
        err(e) => print("cannot read: {e}")
    }
    let removed: Nothing or files.IoError := files.remove("note.txt")
    print(2.0.sqrt)
    print(Date.year(2024) month(1) day(31).plus_months(1))
    let n: Int := random(1..7)
    print(n >= 1 && n < 7)
}
```

```text
hello
1.4142135623730951
2024-02-29
true
```

Files are in Kova's own module `files`, reached after `import files` (no file to add): `files.read`,
`files.write`, `files.append`, `files.exists`, `files.list` (a folder's names, sorted), `files.make_dir` and
`files.remove`. `import files as fs` gives it another name. `kova run` and `kova build` print exactly the
same digits on every system: Kova has its own maths.

## 16. The web

```kova
import web

fn main {
    match web.get("https://example.com/") {
        ok(page) => {
            print(page.status)
            print(page.header("Content-Type"))
            print(page.body.length)
        }
        err(e) => print("no answer: {e}")
    }
    let sent: web.Response or web.WebError := web.post("https://example.com/notes") body("hello")
}
```

The web is Kova's module `web` (`import web`), like `files`. A site's answer is always `ok(…)`, even a `404`: look at `status`. `err` means there was no answer at all:
`NoHost`, `Refused`, `Timeout`, `Broken`, `BadAnswer`, `TooManyRedirects` or `BadUrl`. Redirections are
followed (`page.url` says where it ended). A `web.Response` has `status`, `url`, `body` and `headers` (a
`Map<String, String>` with names in small letters); `header(name)` finds one whatever its capitals.

For anything else, build a `web.Request` step by step, then `web.send` it:

```kova
import web

fn main {
    let asked: web.Request := web.Request.put("https://example.com/notes/1")
        .header("Accept") is("text/plain")
        .body("hello")
        .seconds(5)
        .no_redirections
    match web.send(asked) {
        ok(page) => print(page.status)
        err(e) => print("no answer: {e}")
    }
}
```

A `web.Request` starts with `web.Request.get`, `post`, `put`, `patch`, `delete` or `head`; each step gives a
changed copy. Without `seconds`, Kova waits 30 seconds at most for each step (connecting, each piece
of the answer); `seconds(0)` waits forever. A header Kova writes itself (`Host`, `Content-Length`…)
or holding a line break gives `BadHeader`.

`https://` uses each system's own encryption, so security fixes come with the system's updates;
certificates are always checked (`BadCertificate`). On Linux, a program using the web asks for the
system's C library, and `https://` for OpenSSL (`NoHttps` without it); the others stay standalone.

## 17. JSON

```kova
struct Author {
    name: String
    born: Int?
}

struct Book {
    title: String
    tags: List<String>
    author: Author
}

fn main {
    let b := Book { title := "Dune", tags := ["sf"], author := Author { name := "Herbert", born := 1920 } }
    let text: String := b.to_json
    print(text)
    match Book.from_json(text) {
        ok(book) => print(book.author.name)
        err(e) => print("not a book: {e}")
    }
    print(Book.from_json("\{\"title\": \"Dune\"\}"))
}
```

```text
{"title":"Dune","tags":["sf"],"author":{"name":"Herbert","born":1920}}
Herbert
err(Missing("tags"))
```

Every struct has `T.from_json(text)` and `value.to_json`, checked against its fields: the compiler
refuses a field JSON cannot hold. JSON holds `Int`, `Float`, `Bool`, `String`, `T?`, `List<T>`,
`Map<String, T>` and structs made of them. A field the struct does not have is ignored; a `T?` that is
missing or `null` is `none`. A failure says where: `NotJson(line)`, `Missing(field)` or
`WrongKind(field)`, the field written as a path (`author.name`, `tags[2]`). A text whose root is an
array is read by `List<Book>.from_json(text)` and written by `books.to_json`; an object whose keys are
not known before, by `Map<String, Book>.from_json(text)` and `shelf.to_json`. With the web:
`Book.from_json(page.body)`. In a Kova text, a brace is written `\{`, or the text is raw:
`"""{"title": "Dune"}"""`.

## 18. A web server

```kova
import web

struct Book {
    title: String
    pages: Int
}

fn main {
    web.serve(8080) { q: web.Request =>
        match q.path {
            "/" => web.Answer.text("Hello from Kova")
            "/book" => web.Answer.json(Book { title := "Dune", pages := 412 })
            "/search" => match q.query("author") {
                none => web.Answer.status(400)
                author => web.Answer.text("books by {author}")
            }
            _ => web.Answer.file("public" + q.path)
        }
    }
}
```

`web.serve(port)` answers each question with what its block gives back, one question at a time, until the
program stops (`exit(0)` in a case stops it). Open `http://localhost:8080/` to see it: only this
computer can ask (127.0.0.1). `web.serve_network(8080) { … }` and `web.serve_files_network("public") at(8080)`
let the other computers of the network ask too (the system may first ask you to allow it).

When it starts, the server writes where to open it, then a line for each question it answers
(`14:30:01  192.168.1.20  GET /books  200`), as Python's `http.server` does; they go to the error
output, apart from what the program prints. `web.serve(8080) log(web.Log.Start) { … }` writes only where
to open it, `log(web.Log.Off)` nothing (also after `serve_files(…) at(…)`). The block receives a `web.Request`: `q.method`, `q.path` (decoded, without
the `?…`), `q.query(name)` and `q.header(name)` (both `String?`), `q.body`, `q.headers`.

A `web.Answer` is made by `web.Answer.text(t)`, `web.Answer.json(value)` (a struct, a List or a Map, as
`to_json` writes it), `web.Answer.status(404)` (the body says `404 Not Found`) or `web.Answer.file(path)` (its type from its
name: `.html`, `.css`, `.png`…; `404` if it does not exist or the path goes up with `..`). Add headers
with `.header("Cache-Control") is("no-store")`. `web.serve_files("public") at(8080)` serves a folder alone,
`index.html` for a folder's own address.

A port another program has stops the program (code 101). Kova itself answers a question it cannot read
(`400`), too long (`431`, `413` past 16 MB of body) or sent in pieces (`501`). It listens to every
connection at once and gives the block whichever question is whole first, so a browser's spare
connection that says nothing keeps no one waiting; the block itself answers one question at a time.

`web.serve(8080) workers(8) { q: web.Request => … }` answers up to 8 questions at the same time, each on its own
thread, so a slow answer keeps no one else waiting (also with `log(…)`, before or after). The block may
read the values around it but not change them: `count += 1` inside is refused, since two questions
answered at once would change the same `count`. Each thread starts with its own copy of what the block
reads. `kova run` still answers one question at a time: only the built program answers several at once
(so a block that waits for another question of the same server only gets its answer when built).

## 19. The tools

- `kova run`, `kova build`, `kova test`: run, build, test.
- `kova fmt file.kova`: lays the code out the standard way (the editor does it on save).
- The VSCode / VSCodium extension (`editors/vscode`): colors, errors while typing, completion, types on
  hover, go to definition, rename, ▶ 🔨 🧪 buttons, and a debugger 🐞 (breakpoints, variables, and the
  Debug Console evaluates what you type while stopped).
- Kova speaks English or French: `KOVA_LANG=fr kova run hello.kova`, or `header { lang := fr }` at the top
  of a program.
