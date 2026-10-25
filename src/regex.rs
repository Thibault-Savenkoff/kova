// Regular expressions (PLAN Part 4 "The library after Python, step 2"), shared by `kova run`
// (src/interp.rs) and the runtime of `kova build` (runtime/kova.rs). A pattern becomes a small
// program run by a Pike machine: every way of matching is followed at once, one character at a
// time, so the time grows with the text times the pattern, never more (no "catastrophic
// backtracking"). The first way, in the pattern's order, wins: `a|ab` finds `a` in `ab`.
// Only `core` and `alloc`.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use alloc::string::String;

/// Why a pattern is not one, and where (in characters from its start). src/types.rs turns it
/// into a message.
#[derive(Debug, PartialEq)]
pub enum Problem {
    /// `(` never closed.
    Unclosed,
    /// `)` never opened.
    Unopened,
    /// `[` never closed.
    UnclosedClass,
    /// `*`, `+`, `?` or `{…}` after nothing.
    NothingToRepeat,
    /// `\q`: not a known escape.
    UnknownEscape(char),
    /// `[z-a]`.
    BadRange(char, char),
    /// `{3,1}`, `{2000}` (1000 at most), or a `{` that is no repeat.
    BadRepeat,
    /// A `\` at the very end.
    TrailingBackslash,
}

/// The most a `{n,m}` may say.
const MOST: usize = 1000;

#[derive(Clone, Debug)]
enum Node {
    Char(char),
    /// `.`: any character but a line end.
    Any,
    /// `[a-z_]`, `\d`: ranges, the class turned round if `true`.
    Class(Vec<(char, char)>, bool),
    /// `^`, `$`, `\b`, `\B`.
    Assert(Edge),
    Group(Box<Node>, Option<usize>),
    Repeat(Box<Node>, usize, Option<usize>, bool),
    Concat(Vec<Node>),
    Either(Vec<Node>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Edge {
    Start,
    End,
    Word,
    NotWord,
}

#[derive(Clone, Debug)]
enum Op {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Assert(Edge),
    /// Go on at both, the first one preferred.
    Split(usize, usize),
    Jump(usize),
    /// Remember the position in this capture slot.
    Save(usize),
    Match,
}

/// A pattern made ready to run.
pub struct Regex {
    ops: Vec<Op>,
    /// The groups `(…)`, not counting the whole match.
    pub groups: usize,
}

/// The pattern made ready, or why it is not one.
pub fn compile(pattern: &str) -> Result<Regex, (usize, Problem)> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut p = Parser { c: &chars, at: 0, groups: 0 };
    let node = p.either()?;
    if p.at < chars.len() {
        return Err((p.at, Problem::Unopened));
    }
    let mut ops = vec![Op::Save(0)];
    emit(&node, &mut ops);
    ops.push(Op::Save(1));
    ops.push(Op::Match);
    Ok(Regex { ops, groups: p.groups })
}

struct Parser<'a> {
    c: &'a [char],
    at: usize,
    groups: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.c.get(self.at).copied()
    }

    fn either(&mut self) -> Result<Node, (usize, Problem)> {
        let mut ways = vec![self.concat()?];
        while self.peek() == Some('|') {
            self.at += 1;
            ways.push(self.concat()?);
        }
        Ok(if ways.len() == 1 { ways.pop().unwrap() } else { Node::Either(ways) })
    }

    fn concat(&mut self) -> Result<Node, (usize, Problem)> {
        let mut items = vec![];
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let start = self.at;
            let atom = self.atom()?;
            items.push(self.repeat(atom, start)?);
        }
        Ok(Node::Concat(items))
    }

    fn atom(&mut self) -> Result<Node, (usize, Problem)> {
        let start = self.at;
        let c = self.c[self.at];
        self.at += 1;
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::Assert(Edge::Start),
            '$' => Node::Assert(Edge::End),
            '(' => {
                let number = if self.c[self.at..].starts_with(&['?', ':']) {
                    self.at += 2;
                    None
                } else {
                    self.groups += 1;
                    Some(self.groups)
                };
                let inside = self.either()?;
                if self.peek() != Some(')') {
                    return Err((start, Problem::Unclosed));
                }
                self.at += 1;
                Node::Group(Box::new(inside), number)
            }
            '[' => self.class(start)?,
            '*' | '+' | '?' => return Err((start, Problem::NothingToRepeat)),
            '{' => return Err((start, if self.counts().is_some() { Problem::NothingToRepeat } else { Problem::BadRepeat })),
            '\\' => match self.escape(start)? {
                Escaped::Char(c) => Node::Char(c),
                Escaped::Class(ranges, not) => Node::Class(ranges, not),
                Escaped::Edge(e) => Node::Assert(e),
            },
            c => Node::Char(c),
        })
    }

    /// `\d`, `\.`… after the `\` at `start`.
    fn escape(&mut self, start: usize) -> Result<Escaped, (usize, Problem)> {
        let Some(c) = self.peek() else { return Err((start, Problem::TrailingBackslash)) };
        self.at += 1;
        let digits = vec![('0', '9')];
        let word = vec![('0', '9'), ('A', 'Z'), ('_', '_'), ('a', 'z'), (WORD, WORD)];
        let space = vec![('\t', '\r'), (' ', ' '), (SPACE, SPACE)];
        Ok(match c {
            'd' => Escaped::Class(digits, false),
            'D' => Escaped::Class(digits, true),
            'w' => Escaped::Class(word, false),
            'W' => Escaped::Class(word, true),
            's' => Escaped::Class(space, false),
            'S' => Escaped::Class(space, true),
            'b' => Escaped::Edge(Edge::Word),
            'B' => Escaped::Edge(Edge::NotWord),
            'n' => Escaped::Char('\n'),
            't' => Escaped::Char('\t'),
            'r' => Escaped::Char('\r'),
            c if c.is_ascii_alphanumeric() => return Err((start, Problem::UnknownEscape(c))),
            c => Escaped::Char(c),
        })
    }

    /// `[a-z_]`, `[^,]` after the `[` at `start`.
    fn class(&mut self, start: usize) -> Result<Node, (usize, Problem)> {
        let not = self.peek() == Some('^');
        if not {
            self.at += 1;
        }
        let mut ranges = vec![];
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return Err((start, Problem::UnclosedClass)) };
            if c == ']' && !first {
                self.at += 1;
                break;
            }
            first = false;
            let at = self.at;
            self.at += 1;
            let low = if c == '\\' {
                match self.escape(at)? {
                    Escaped::Char(c) => c,
                    Escaped::Class(r, false) => {
                        ranges.extend(r);
                        continue;
                    }
                    // `[\D]`, `[\b]`: kept simple, refused.
                    _ => return Err((at, Problem::UnknownEscape(self.c[at + 1]))),
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.c.get(self.at + 1).is_some_and(|&n| n != ']') {
                self.at += 1;
                let high = self.c[self.at];
                self.at += 1;
                let high = if high == '\\' {
                    match self.escape(self.at - 1)? {
                        Escaped::Char(c) => c,
                        _ => return Err((at, Problem::BadRange(low, self.c[self.at - 1]))),
                    }
                } else {
                    high
                };
                if high < low {
                    return Err((at, Problem::BadRange(low, high)));
                }
                ranges.push((low, high));
            } else {
                ranges.push((low, low));
            }
        }
        Ok(Node::Class(ranges, not))
    }

    /// `{2}`, `{2,}`, `{2,5}` from the `{`: (least, most), the position past the `}`.
    fn counts(&self) -> Option<(usize, Option<usize>, usize)> {
        let mut at = self.at;
        let number = |at: &mut usize| {
            let from = *at;
            while self.c.get(*at).is_some_and(|c| c.is_ascii_digit()) {
                *at += 1;
            }
            (*at > from).then(|| self.c[from..*at].iter().fold(0usize, |n, d| n.saturating_mul(10).saturating_add(*d as usize - '0' as usize)))
        };
        let least = number(&mut at)?;
        let most = if self.c.get(at) == Some(&',') {
            at += 1;
            number(&mut at)
        } else {
            Some(least)
        };
        (self.c.get(at) == Some(&'}')).then_some((least, most, at + 1))
    }

    fn repeat(&mut self, mut atom: Node, start: usize) -> Result<Node, (usize, Problem)> {
        loop {
            let (least, most) = match self.peek() {
                Some('*') => (0, None),
                Some('+') => (1, None),
                Some('?') => (0, Some(1)),
                Some('{') => {
                    self.at += 1;
                    let Some((least, most, past)) = self.counts() else { return Err((self.at - 1, Problem::BadRepeat)) };
                    if most.is_some_and(|m| m < least) || least > MOST || most.is_some_and(|m| m > MOST) {
                        return Err((self.at - 1, Problem::BadRepeat));
                    }
                    self.at = past - 1;
                    (least, most)
                }
                _ => return Ok(atom),
            };
            self.at += 1;
            if matches!(atom, Node::Assert(_)) {
                return Err((start, Problem::NothingToRepeat));
            }
            let lazy = self.peek() == Some('?');
            if lazy {
                self.at += 1;
            }
            atom = Node::Repeat(Box::new(atom), least, most, lazy);
        }
    }
}

enum Escaped {
    Char(char),
    Class(Vec<(char, char)>, bool),
    Edge(Edge),
}

/// Stand for "any other letter or digit" and "any other space" in `\w` and `\s`: not
/// characters a text holds (Unicode's private-use area), checked by `in_class`.
const WORD: char = '\u{10FFFE}';
const SPACE: char = '\u{10FFFD}';

fn emit(node: &Node, ops: &mut Vec<Op>) {
    match node {
        Node::Char(c) => ops.push(Op::Char(*c)),
        Node::Any => ops.push(Op::Any),
        Node::Class(r, not) => ops.push(Op::Class(r.clone(), *not)),
        Node::Assert(e) => ops.push(Op::Assert(*e)),
        Node::Group(inside, None) => emit(inside, ops),
        Node::Group(inside, Some(n)) => {
            ops.push(Op::Save(2 * n));
            emit(inside, ops);
            ops.push(Op::Save(2 * n + 1));
        }
        Node::Concat(items) => items.iter().for_each(|n| emit(n, ops)),
        Node::Either(ways) => {
            // split L1, next; L1: way; jump end; next: split …
            let mut jumps = vec![];
            for (k, way) in ways.iter().enumerate() {
                if k + 1 < ways.len() {
                    let split = ops.len();
                    ops.push(Op::Split(split + 1, 0));
                    emit(way, ops);
                    jumps.push(ops.len());
                    ops.push(Op::Jump(0));
                    let next = ops.len();
                    ops[split] = Op::Split(split + 1, next);
                } else {
                    emit(way, ops);
                }
            }
            let end = ops.len();
            for j in jumps {
                ops[j] = Op::Jump(end);
            }
        }
        Node::Repeat(inside, least, most, lazy) => {
            for _ in 0..*least {
                emit(inside, ops);
            }
            let choice = |go: usize, skip: usize| if *lazy { Op::Split(skip, go) } else { Op::Split(go, skip) };
            match most {
                // x*: L: split body, end; body; jump L
                None => {
                    let split = ops.len();
                    ops.push(Op::Jump(0));
                    emit(inside, ops);
                    ops.push(Op::Jump(split));
                    let end = ops.len();
                    ops[split] = choice(split + 1, end);
                }
                // x? as many times as more is allowed, each one skipping to the end.
                Some(most) => {
                    let mut splits = vec![];
                    for _ in *least..*most {
                        splits.push(ops.len());
                        ops.push(Op::Jump(0));
                        emit(inside, ops);
                    }
                    let end = ops.len();
                    for s in splits {
                        ops[s] = choice(s + 1, end);
                    }
                }
            }
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn in_class(ranges: &[(char, char)], not: bool, c: char) -> bool {
    let found = ranges.iter().any(|&(low, high)| match low {
        WORD => is_word(c),
        SPACE => c.is_whitespace(),
        _ => low <= c && c <= high,
    });
    found != not
}

impl Regex {
    /// The first match starting at `from` or after (`whole`: only one covering all of `text`
    /// from `from`): the capture slots, two per group (start, end; `usize::MAX` if not reached),
    /// the whole match first. Positions in characters.
    pub fn search(&self, text: &[char], from: usize, whole: bool) -> Option<Vec<usize>> {
        let slots = 2 * (self.groups + 1);
        let mut now: Vec<(usize, Vec<usize>)> = vec![];
        let mut next: Vec<(usize, Vec<usize>)> = vec![];
        let mut seen = vec![usize::MAX; self.ops.len()];
        let mut found = None;
        for at in from..=text.len() {
            if found.is_none() && (at == from || !whole) {
                self.add(&mut now, &mut seen, 0, vec![usize::MAX; slots], text, at);
            }
            // Nothing left, and no new start to come.
            if now.is_empty() && (found.is_some() || whole) {
                break;
            }
            let c = text.get(at).copied();
            for (pc, caps) in now.drain(..) {
                let fits = match (&self.ops[pc], c) {
                    (Op::Char(want), Some(c)) => *want == c,
                    (Op::Any, Some(c)) => c != '\n',
                    (Op::Class(r, not), Some(c)) => in_class(r, *not, c),
                    (Op::Match, _) if whole && at != text.len() => false,
                    (Op::Match, _) => {
                        found = Some(caps);
                        // The ways after this one are less preferred: dropped.
                        break;
                    }
                    _ => false,
                };
                if fits {
                    self.add(&mut next, &mut seen, pc + 1, caps, text, at + 1);
                }
            }
            core::mem::swap(&mut now, &mut next);
            next.clear();
        }
        found
    }

    /// The thread at `pc` into `list`, following jumps, splits, saves and edges at `at`.
    fn add(&self, list: &mut Vec<(usize, Vec<usize>)>, seen: &mut [usize], pc: usize, caps: Vec<usize>, text: &[char], at: usize) {
        let mut stack = vec![(pc, caps)];
        while let Some((pc, mut caps)) = stack.pop() {
            if seen[pc] == at {
                continue;
            }
            seen[pc] = at;
            match &self.ops[pc] {
                Op::Jump(to) => stack.push((*to, caps)),
                // The second pushed first: the first is followed first.
                Op::Split(a, b) => {
                    stack.push((*b, caps.clone()));
                    stack.push((*a, caps));
                }
                Op::Save(n) => {
                    caps[*n] = at;
                    stack.push((pc + 1, caps));
                }
                Op::Assert(edge) => {
                    let before = at.checked_sub(1).and_then(|i| text.get(i)).is_some_and(|&c| is_word(c));
                    let after = text.get(at).is_some_and(|&c| is_word(c));
                    let holds = match edge {
                        Edge::Start => at == 0,
                        Edge::End => at == text.len(),
                        Edge::Word => before != after,
                        Edge::NotWord => before == after,
                    };
                    if holds {
                        stack.push((pc + 1, caps));
                    }
                }
                _ => list.push((pc, caps)),
            }
        }
    }

    /// Each match, left to right, not overlapping; empty ones left out.
    fn each(&self, text: &[char], mut f: impl FnMut(usize, usize, &[usize])) {
        let mut at = 0;
        while at <= text.len() {
            let Some(caps) = self.search(text, at, false) else { break };
            let (start, end) = (caps[0], caps[1]);
            if end > start {
                f(start, end, &caps);
            }
            at = if end > start { end } else { end + 1 };
        }
    }
}

/// `s.matches(p)`: the whole text fits the pattern.
pub fn matches(re: &Regex, text: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    re.search(&t, 0, true).is_some()
}

/// `s.find_all(p)`: the texts of every match.
pub fn find_all(re: &Regex, text: &str) -> Vec<String> {
    let t: Vec<char> = text.chars().collect();
    let mut found = vec![];
    re.each(&t, |start, end, _| found.push(t[start..end].iter().collect()));
    found
}

/// `s.replace_all(p) with(new)`.
pub fn replace_all(re: &Regex, text: &str, new: &str) -> String {
    let t: Vec<char> = text.chars().collect();
    let (mut out, mut kept) = (String::new(), 0);
    re.each(&t, |start, end, _| {
        out.extend(&t[kept..start]);
        out.push_str(new);
        kept = end;
    });
    out.extend(&t[kept..]);
    out
}

/// `s.groups(p)`: the texts of the groups of the first match (an empty text for a group the
/// match did not go through), or `None`.
pub fn groups(re: &Regex, text: &str) -> Option<Vec<String>> {
    let t: Vec<char> = text.chars().collect();
    let caps = re.search(&t, 0, false)?;
    Some((1..=re.groups).map(|g| match (caps[2 * g], caps[2 * g + 1]) {
        (start, end) if start != usize::MAX && end != usize::MAX => t[start..end].iter().collect(),
        _ => String::new(),
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(p: &str, t: &str) -> Vec<String> {
        find_all(&compile(p).unwrap(), t)
    }

    #[test]
    fn matching() {
        assert_eq!(all(r"\d+", "a12b345"), ["12", "345"]);
        assert_eq!(all(r"\bp\w+", "12 pommes, 7 poires"), ["pommes", "poires"]);
        assert_eq!(all("a|ab", "ab"), ["a"]);
        assert_eq!(all("ab|a", "ab"), ["ab"]);
        assert_eq!(all("a+?", "aaa"), ["a", "a", "a"]);
        assert_eq!(all("<.+>", "<a><b>"), ["<a><b>"]);
        assert_eq!(all("<.+?>", "<a><b>"), ["<a>", "<b>"]);
        assert_eq!(all(r"\bété\b", "l'été, étés"), ["été"]);
        assert_eq!(all("x{2,3}", "xxxxxxx"), ["xxx", "xxx"]);
        assert_eq!(all("[^,]+", "a,b,,c"), ["a", "b", "c"]);
        assert_eq!(all(r"[\d.-]+", "-3.5 et 2"), ["-3.5", "2"]);
        assert!(matches(&compile("[a-z.]+@[a-z.]+").unwrap(), "ana@ex.fr"));
        assert!(!matches(&compile("[a-z]+").unwrap(), "abc1"));
        assert!(matches(&compile("(a|ab)c").unwrap(), "abc"));
        assert!(matches(&compile("(a*)*b").unwrap(), "aaab"));
        assert_eq!(replace_all(&compile(r"\s+").unwrap(), "a  b \t c", " "), "a b c");
        assert_eq!(groups(&compile(r"(\d+)-(\d+)(x)?").unwrap(), "le 12-34"), Some(vec!["12".into(), "34".into(), String::new()]));
        assert_eq!(groups(&compile(r"(\d)").unwrap(), "abc"), None);
        // No blow-up: 30 `a?` then 30 `a` on 30 `a` stays fast.
        let p = "a?".repeat(30) + &"a".repeat(30);
        assert!(matches(&compile(&p).unwrap(), &"a".repeat(30)));
    }

    #[test]
    fn problems() {
        let e = |p: &str| compile(p).err().unwrap();
        assert_eq!(e("(ab"), (0, Problem::Unclosed));
        assert_eq!(e("ab)"), (2, Problem::Unopened));
        assert_eq!(e("[ab"), (0, Problem::UnclosedClass));
        assert_eq!(e("*a"), (0, Problem::NothingToRepeat));
        assert_eq!(e(r"a\q"), (1, Problem::UnknownEscape('q')));
        assert_eq!(e("[z-a]"), (1, Problem::BadRange('z', 'a')));
        assert_eq!(e("a{3,1}"), (1, Problem::BadRepeat));
        assert_eq!(e("a{"), (1, Problem::BadRepeat));
        assert_eq!(e("a\\"), (1, Problem::TrailingBackslash));
    }
}
