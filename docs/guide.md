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

`{…}` inside a text puts a value in it.

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

    let seen: Set<String> := ["red", "blue", "red"]
    print(seen.length)
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
2
```

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
- Reading a file gives a `String or IoError`: Kova never hides that something can go wrong.

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
    }
    print(larger(3, 8))
    print(larger("pear", "apple"))
    print(Pair<Int> { first := 1, second := 2 } = Pair<Int> { first := 1, second := 2 })
}
```

```text
woof woof
meow meow
8
pear
true
```

- An interface lists methods; a struct that follows it writes them, or takes the default ones.
- `any Animal` holds any value that follows `Animal`.
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
fn main {
    let saved: Nothing or IoError := write_file("note.txt", "hello\n")
    match read_file("note.txt") {
        ok(text) => write(text)
        err(e) => print("cannot read: {e}")
    }
    let removed: Nothing or IoError := remove_file("note.txt")
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

`kova run` and `kova build` print exactly the same digits on every system: Kova has its own maths.

## 16. The web

```kova
fn main {
    match get("https://example.com/") {
        ok(page) => {
            print(page.status)
            print(page.header("Content-Type"))
            print(page.body.length)
        }
        err(e) => print("no answer: {e}")
    }
    let sent: Response or WebError := post("https://example.com/notes") body("hello")
}
```

A site's answer is always `ok(…)`, even a `404`: look at `status`. `err` means there was no answer at all:
`NoHost`, `Refused`, `Timeout`, `Broken`, `BadAnswer`, `TooManyRedirects` or `BadUrl`. Redirections are
followed (`page.url` says where it ended). A `Response` has `status`, `url`, `body` and `headers` (a
`Map<String, String>` with names in small letters); `header(name)` finds one whatever its capitals.

For anything else, build a `Request` step by step, then `send` it:

```kova
fn main {
    let asked: Request := Request.put("https://example.com/notes/1")
        .header("Accept") is("text/plain")
        .body("hello")
        .seconds(5)
        .no_redirections
    match send(asked) {
        ok(page) => print(page.status)
        err(e) => print("no answer: {e}")
    }
}
```

A `Request` starts with `Request.get`, `post`, `put`, `patch`, `delete` or `head`; each step gives a
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
`Book.from_json(page.body)`. In a Kova text, a brace is written `\{`.

## 18. A web server

```kova
struct Book {
    title: String
    pages: Int
}

fn main {
    serve(8080) { q: Request =>
        match q.path {
            "/" => Answer.text("Hello from Kova")
            "/book" => Answer.json(Book { title := "Dune", pages := 412 })
            "/search" => match q.query("author") {
                none => Answer.status(400)
                author => Answer.text("books by {author}")
            }
            _ => Answer.file("public" + q.path)
        }
    }
}
```

`serve(port)` answers each question with what its block gives back, one question at a time, until the
program stops (`exit(0)` in a case stops it). Open `http://localhost:8080/` to see it: only this
computer can ask (127.0.0.1). `serve_network(8080) { … }` and `serve_files_network("public") at(8080)`
let the other computers of the network ask too (the system may first ask you to allow it). The block receives a `Request`: `q.method`, `q.path` (decoded, without
the `?…`), `q.query(name)` and `q.header(name)` (both `String?`), `q.body`, `q.headers`.

An `Answer` is made by `Answer.text(t)`, `Answer.json(value)` (a struct, a List or a Map, as `to_json`
writes it), `Answer.status(404)` (the body says `404 Not Found`) or `Answer.file(path)` (its type from its
name: `.html`, `.css`, `.png`…; `404` if it does not exist or the path goes up with `..`). Add headers
with `.header("Cache-Control") is("no-store")`. `serve_files("public") at(8080)` serves a folder alone,
`index.html` for a folder's own address.

A port another program has stops the program (code 101). Kova itself answers a question it cannot read
(`400`), too long (`431`, `413` past 16 MB of body) or sent in pieces (`501`). It listens to every
connection at once and gives the block whichever question is whole first, so a browser's spare
connection that says nothing keeps no one waiting; the block itself answers one question at a time.

## 19. The tools

- `kova run`, `kova build`, `kova test`: run, build, test.
- `kova fmt file.kova`: lays the code out the standard way (the editor does it on save).
- The VSCode / VSCodium extension (`editors/vscode`): colors, errors while typing, completion, types on
  hover, go to definition, rename, ▶ 🔨 🧪 buttons, and a debugger 🐞 (breakpoints, variables, and the
  Debug Console evaluates what you type while stopped).
- Kova speaks English or French: `KOVA_LANG=fr kova run hello.kova`, or `header { lang := fr }` at the top
  of a program.
