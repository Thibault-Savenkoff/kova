// What Android changes in Linux's runtime (PLAN Part 4 "Android"). An app starts in Java
// (runtime/android/Main.java), which loads the program as a library and gives it a thread. System
// calls stay Linux's. Compiled with `--cfg android` (build.rs).
//
// The two sides talk through one Java method, `call`, in bytes: Java's own thread waits in
// `take` for what the program asks (a message: a letter, then its content), and `give` brings back
// what happened. The program never calls Java, so it needs almost nothing of JNI.

use core::ptr::null_mut;
use core::sync::atomic::Ordering::SeqCst;
use core::sync::atomic::AtomicU32;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::Locked;
use crate::http::Fail;
use crate::shared::{UiMenu, UiNode, UiQueue};

/// What `pthread_attr_t` holds: 56 bytes on 64-bit Android.
type ThreadOptions = [u64; 7];

unsafe extern "C" {
    fn __android_log_write(priority: i32, tag: *const u8, text: *const u8) -> i32;
    fn chdir(path: *const u8) -> i32;
    fn getenv(name: *const u8) -> *const u8;
    fn localtime_r(time: *const i64, out: *mut [i64; 8]) -> *mut u8;
    fn pthread_attr_init(options: *mut ThreadOptions) -> i32;
    fn pthread_attr_setstacksize(options: *mut ThreadOptions, size: usize) -> i32;
    fn pthread_create(thread: *mut usize, options: *const ThreadOptions, f: extern "C" fn(*mut u8) -> *mut u8, arg: *mut u8) -> i32;
    fn pthread_join(thread: usize, out: *mut *mut u8) -> i32;
}

/// Woken on every change of what it counts.
struct Signal(AtomicU32);

impl Signal {
    const fn new() -> Signal {
        Signal(AtomicU32::new(0))
    }
    fn seen(&self) -> u32 {
        self.0.load(SeqCst)
    }
    fn tell(&self) {
        self.0.fetch_add(1, SeqCst);
        super::wake(&self.0);
    }
    fn wait(&self, seen: u32) {
        super::wait(&self.0, seen);
    }
}

static LANGUAGE: Locked<Option<String>> = Locked::new(None);

// To Java.
static OUT: Locked<VecDeque<Vec<u8>>> = Locked::new(VecDeque::new());
static OUT_SIGNAL: Signal = Signal::new();

// From Java: events for `ui.show`, then the answer to the question being asked.
static EVENTS: Locked<UiQueue> = Locked::new(UiQueue::new());
static EVENT_SIGNAL: Signal = Signal::new();
static AREAS: Locked<[i32; 8]> = Locked::new([0; 8]);
static ROOMS: Locked<Vec<(usize, i32, i32)>> = Locked::new(Vec::new());
static REPLY: Locked<Option<Vec<u8>>> = Locked::new(None);
static REPLY_SIGNAL: Signal = Signal::new();
/// One question at a time: threads may ask together.
static ASKING: super::Lock = super::Lock::new();

/// Telling Java, not `print`'s business: the log is for `adb logcat`.
fn send(kind: u8, content: &[u8]) {
    let mut m = Vec::with_capacity(content.len() + 1);
    m.push(kind);
    m.extend_from_slice(content);
    OUT.with(|q| q.push_back(m));
    OUT_SIGNAL.tell();
}

/// Tells Java and waits for its answer.
fn ask(kind: u8, content: &[u8]) -> Vec<u8> {
    ASKING.lock();
    let seen = REPLY_SIGNAL.seen();
    send(kind, content);
    let mut seen = seen;
    let answer = loop {
        if let Some(r) = REPLY.with(|r| r.take()) {
            break r;
        }
        REPLY_SIGNAL.wait(seen);
        seen = REPLY_SIGNAL.seen();
    };
    ASKING.unlock();
    answer
}

/// The program, on a thread of its own: Android's first thread must keep drawing.
extern "C" fn program(_: *mut u8) -> *mut u8 {
    crate::run()
}

fn start(words: &[u8]) {
    let words = String::from_utf8_lossy(words).into_owned();
    let (folder, language) = words.split_once('\n').unwrap_or((&words, ""));
    unsafe {
        let folder = [folder.as_bytes(), &[0]].concat();
        chdir(folder.as_ptr());
        LANGUAGE.with(|l| *l = Some(String::from(language)));
        *crate::ARGS.get() = vec![String::from("app")];
        *super::PTHREAD_CREATE.get() = pthread_create as usize;
        *super::PTHREAD_JOIN.get() = pthread_join as usize;
        let mut options = [0; 7];
        pthread_attr_init(&mut options);
        pthread_attr_setstacksize(&mut options, super::STACK);
        let mut thread = 0;
        pthread_create(&mut thread, &options, program, null_mut());
    }
}

/// The next message for Java; waits for one.
fn take() -> Vec<u8> {
    loop {
        let seen = OUT_SIGNAL.seen();
        if let Some(m) = OUT.with(|q| q.pop_front()) {
            return m;
        }
        OUT_SIGNAL.wait(seen);
    }
}

/// `E` answer, areas, rooms (`number width height`, by commas), what was typed, one by line; or
/// `R` the answer to a question.
fn give(b: &[u8]) {
    let Some((&kind, rest)) = b.split_first() else { return };
    if kind == b'R' {
        REPLY.with(|r| *r = Some(rest.to_vec()));
        return REPLY_SIGNAL.tell();
    }
    let text = String::from_utf8_lossy(rest).into_owned();
    let mut lines = text.splitn(4, '\n');
    let (answer, areas, rooms, words) = (lines.next(), lines.next(), lines.next(), lines.next());
    AREAS.with(|a| areas.unwrap_or("").split(' ').zip(a.iter_mut()).for_each(|(n, v)| *v = n.parse().unwrap_or(0)));
    let number = |n: Option<&str>| n.and_then(|n| n.parse().ok()).unwrap_or(0);
    ROOMS.with(|r| {
        r.clear();
        for room in rooms.unwrap_or("").split(',').filter(|s| !s.is_empty()) {
            let mut n = room.split(' ');
            r.push((number(n.next()) as usize, number(n.next()), number(n.next())));
        }
    });
    let answer = answer.and_then(|a| a.parse().ok()).unwrap_or(-1);
    EVENTS.with(|q| q.hear(answer, String::from(words.unwrap_or(""))));
    EVENT_SIGNAL.tell();
}

// The numbers of the JNI functions used, in the table every `JNIEnv` points to.
const GET_ARRAY_LENGTH: usize = 171;
const NEW_BYTE_ARRAY: usize = 176;
const GET_BYTE_ARRAY_REGION: usize = 200;
const SET_BYTE_ARRAY_REGION: usize = 208;

type Env = *const *const usize;

/// `static native byte[] call(int op, byte[] data)` of `dev.kova.Main`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_dev_kova_Main_call(env: Env, _class: usize, op: i32, data: usize) -> usize {
    unsafe {
        let function = |n: usize| *(*env).add(n);
        let mut b = vec![];
        if data != 0 {
            let length: extern "C" fn(Env, usize) -> i32 = core::mem::transmute(function(GET_ARRAY_LENGTH));
            b.resize(length(env, data) as usize, 0u8);
            let read: extern "C" fn(Env, usize, i32, i32, *mut u8) = core::mem::transmute(function(GET_BYTE_ARRAY_REGION));
            read(env, data, 0, b.len() as i32, b.as_mut_ptr());
        }
        match op {
            0 => start(&b),
            2 => give(&b),
            _ => {
                let m = take();
                let new: extern "C" fn(Env, i32) -> usize = core::mem::transmute(function(NEW_BYTE_ARRAY));
                let array = new(env, m.len() as i32);
                let write: extern "C" fn(Env, usize, i32, i32, *const u8) = core::mem::transmute(function(SET_BYTE_ARRAY_REGION));
                write(env, array, 0, m.len() as i32, m.as_ptr());
                return array;
            }
        }
    }
    0
}

pub fn language() -> Option<String> {
    LANGUAGE.with(|l| l.clone()).filter(|l| !l.is_empty())
}

pub fn var(name: &str) -> Option<String> {
    let asked = [name.as_bytes(), &[0]].concat();
    let found = unsafe { getenv(asked.as_ptr()) };
    (!found.is_null()).then(|| String::from_utf8_lossy(unsafe { crate::c_text(found) }).into_owned())
}

/// The local time at `utc`: Android keeps its time zones its own way, its C library reads them.
pub fn local_at(utc: i64) -> i64 {
    // `struct tm`: nine `int`s, then the offset from UTC in seconds.
    let mut tm = [0; 8];
    unsafe { localtime_r(&utc, &mut tm) };
    utc + tm[5]
}

// Android 8 kills an app calling `statx` or `renameat2` (too new for its list of allowed system
// calls), and its older versions never learn them: the older calls work everywhere.
#[cfg(target_arch = "x86_64")]
const NEWFSTATAT: usize = 262;
#[cfg(target_arch = "aarch64")]
const NEWFSTATAT: usize = 79;
#[cfg(target_arch = "x86_64")]
pub const RENAMEAT: usize = 264;
#[cfg(target_arch = "aarch64")]
pub const RENAMEAT: usize = 38;

/// A file's kind, size and modification time, or the system's error (negative).
pub fn stat(path: *const u8, flags: usize) -> Result<(u16, i64, i64), isize> {
    let mut st = [0u8; 256];
    let asked = unsafe { super::call(NEWFSTATAT, super::AT_FDCWD, path as usize, st.as_mut_ptr() as usize, flags, 0, 0) };
    if asked != 0 {
        return Err(asked);
    }
    #[cfg(target_arch = "x86_64")]
    let mode = u32::from_ne_bytes(st[24..28].try_into().unwrap());
    #[cfg(target_arch = "aarch64")]
    let mode = u32::from_ne_bytes(st[16..20].try_into().unwrap());
    let number = |at: usize| i64::from_ne_bytes(st[at..at + 8].try_into().unwrap());
    Ok((mode as u16 & 0o170000, number(48), number(88)))
}

/// `print`: for the app's console, and one line of the system's log for each line of `b`
/// (`adb logcat -s kova`).
pub fn print(b: &[u8]) {
    send(b'P', b);
    let mut line = [0u8; 1000];
    for piece in b.trim_ascii_end().split(|&c| c == b'\n') {
        for part in piece.chunks(line.len() - 1).chain(piece.is_empty().then_some(&[][..])) {
            line[..part.len()].copy_from_slice(part);
            line[part.len()] = 0;
            unsafe { __android_log_write(4, c"kova".as_ptr() as *const u8, line.as_ptr()) };
        }
    }
}

/// `read_line`: the app's console asks.
pub fn read_input(b: &mut [u8]) -> usize {
    let mut line = ask(b'R', b"");
    if line.first() == Some(&0) {
        return 0; // the app closes
    }
    line.push(b'\n');
    let n = line.len().min(b.len());
    b[..n].copy_from_slice(&line[..n]);
    n
}

/// The app stops: Java decides when (a console stays on the screen). This thread is over.
pub fn exit(code: i32) -> ! {
    send(b'X', format!("{code}").as_bytes());
    let parked = AtomicU32::new(0);
    loop {
        super::wait(&parked, 0);
    }
}

pub fn ui_show(title: &str, view: &UiNode, menus: &[UiMenu], width: i32, height: i32, acts: &[i64], typed: &mut String) -> i64 {
    send(b'V', crate::view::message(title, view, menus, width, height, acts).as_bytes());
    loop {
        let seen = EVENT_SIGNAL.seen();
        if let Some((answer, words)) = EVENTS.with(|q| q.next()) {
            ROOMS.with(|r| r.iter().for_each(|&(k, w, h)| {
                crate::ui_room(k, w, h);
            }));
            if answer == SHADER_FAILED {
                crate::gl_stop(crate::GPU_MISSING + 1, words.as_bytes());
            }
            *typed = words;
            return answer;
        }
        EVENT_SIGNAL.wait(seen);
    }
}

/// An event saying a shader does not compile, with the compiler's words.
const SHADER_FAILED: i64 = -1_000_000_000;

/// The window's then the screen's x, y, width and height.
pub fn ui_areas() -> Option<[i32; 8]> {
    Some(AREAS.with(|a| *a))
}

/// Java's answer: nothing, or a letter then the text.
pub fn ui_dialog(kind: i64, title: &str, text: &str) -> Option<String> {
    let r = ask(b'A', format!("{kind}\n{title}\n{text}").as_bytes());
    let (&first, rest) = r.split_first()?;
    (first == b'Y').then(|| String::from_utf8_lossy(rest).into_owned())
}

pub fn sound(whole: &[u8], _wav: &crate::shared::Wav) -> bool {
    ask(b'S', whole);
    true
}


/// HTTP and HTTPS go through Java: Android keeps `libssl` from apps, and gives them its own
/// certificates and proxy settings there.
pub fn fetch(r: &crate::http::Request) -> Option<Result<crate::http::Answer, Fail>> {
    if crate::http::parse_url(r.url).is_none() {
        return Some(Err(Fail::BadUrl));
    }
    let mut o = String::from("{\"url\":");
    crate::view::quoted(&mut o, r.url);
    o.push_str(",\"method\":");
    crate::view::quoted(&mut o, r.method);
    o.push_str(&format!(",\"follow\":{},\"ms\":{},\"headers\":", r.follow, r.ms));
    crate::view::list(&mut o, &r.headers, |o, (name, value)| {
        o.push('[');
        crate::view::quoted(o, name);
        o.push(',');
        crate::view::quoted(o, value);
        o.push(']');
    });
    o.push('}');
    let mut question = o.into_bytes();
    question.push(0);
    question.extend_from_slice(r.body.as_ref());
    let b = ask(b'W', &question);
    Some(match b.split_first() {
        Some((b'+', rest)) => Ok(crate::view::answer_of(rest)),
        Some((b'c', _)) => Err(Fail::BadCertificate),
        Some((b'h', _)) => Err(Fail::NoHost),
        Some((b't', _)) => Err(Fail::Timeout),
        Some((b'r', _)) => Err(Fail::TooManyRedirects),
        _ => Err(Fail::Refused),
    })
}
