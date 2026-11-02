// What the runtime asks of the browser (`kova build --target web`, PLAN Part 4 "WebAssembly"):
// a few functions the page's JavaScript gives (runtime/web.js), and an allocator over the module's
// memory. No files, threads or sockets here: `kova build` refuses the programs that use them.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::AtomicU32;

use crate::Global;
use crate::http::{Fail, Network, Serve};
use crate::shared::{UiMenu, UiNode};
use crate::view::{list, quoted};

#[link(wasm_import_module = "kova")]
unsafe extern "C" {
    #[link_name = "write"]
    fn host_write(fd: i32, text: *const u8, n: usize);
    /// Some bytes of the input, 0 at its end, -1 if none has come yet.
    #[link_name = "read"]
    fn host_read(into: *mut u8, n: usize) -> i32;
    /// The program leaves (see `leave`): the page starts it again after `s` seconds, or when
    /// some input has come.
    fn nap(s: f64);
    fn await_input();
    fn stop(code: i32) -> !;
    /// Milliseconds since 1970.
    fn time() -> f64;
    #[link_name = "chance"]
    fn host_chance(into: *mut u8, n: usize);
    /// Shows a view (JSON, src: `ui_show`): 0, or 1 without WebGL, 2 if a shader does not compile.
    fn ui_render(json: *const u8, n: usize) -> i32;
    /// What happened: as `ui.show` says it (a Float: it may be `UI_RESIZED`), and `ui.typed`'s text.
    fn ui_clicked() -> f64;
    fn ui_typed(into: *mut u8, n: usize) -> usize;
    fn ui_rooms() -> i32;
    fn ui_room(k: i32, which: i32) -> i32;
    fn ui_area(k: i32) -> i32;
    fn ui_ask(kind: i32, title: *const u8, n: usize, text: *const u8, m: usize) -> i32;
    fn play(wav: *const u8, n: usize);
    /// The size of a file only the host has, -1 if it has none, -2 if it is a folder; then
    /// `file_read` copies it.
    fn file_size(path: *const u8, n: usize) -> i32;
    fn file_read(into: *mut u8);
    fn pick_file();
    /// The size of the file picked (`file_read`; its name by `ui_typed`), or -1.
    fn picked() -> i32;
    fn download(name: *const u8, n: usize, data: *const u8, m: usize);
    fn fetch_start(json: *const u8, n: usize, body: *const u8, m: usize);
    /// The answer's size (`file_read`), -1 refused, -2 too late.
    fn fetched() -> i32;
    /// A variable's value into `into`, at most `n` bytes: its whole size, or -1 without it. The
    /// page's own language is the variable "language".
    fn variable(name: *const u8, length: usize, into: *mut u8, n: usize) -> i32;
}

pub const SLASH: char = '/';

fn asked(name: &str) -> Option<String> {
    let mut value = alloc::vec![0; 256];
    let n = unsafe { variable(name.as_ptr(), name.len(), value.as_mut_ptr(), value.len()) };
    if n < 0 {
        return None;
    }
    if n as usize > value.len() {
        value.resize(n as usize, 0);
        unsafe { variable(name.as_ptr(), name.len(), value.as_mut_ptr(), value.len()) };
    }
    value.truncate(n as usize);
    String::from_utf8(value).ok()
}

pub fn system_language() -> Option<String> {
    asked("language")
}

pub fn var(name: &str) -> Option<String> {
    asked(name)
}

pub fn exit(code: i32) -> ! {
    unsafe { stop(code) }
}

pub fn write(fd: usize, b: &[u8]) {
    unsafe { host_write(fd as i32, b.as_ptr(), b.len()) }
}

/// While the input is empty, the program leaves: called again when it enters back, this reads.
pub fn read(_fd: usize, b: &mut [u8]) -> usize {
    arrived();
    match unsafe { host_read(b.as_mut_ptr(), b.len()) } {
        -1 => {
            leave();
            unsafe { await_input() };
            0
        }
        n => n as usize,
    }
}

// ---- Waiting ----
// A page cannot block. The built-in that must wait calls `leave` and returns: every Kova function
// on the stack then saves its locals and returns too (src/wasm.rs `resumable`), down to JavaScript,
// which waits for real. Then it calls the module's start again: each function restores its locals
// and calls the next one again, down to the same built-in, which finds itself `arrived`.

static PHASE: Global<i32> = Global::new(0);
const LEAVING: i32 = 1;
const COMING_BACK: i32 = 2;

fn leave() {
    crate::flush();
    *PHASE.get() = LEAVING;
}

/// The program is back at the built-in that left: running as usual from here. Gives whether it was.
fn arrived() -> bool {
    let back = *PHASE.get() == COMING_BACK;
    *PHASE.get() = 0;
    back
}

pub fn rewind() {
    *PHASE.get() = COMING_BACK;
}

pub fn unwinding() -> bool {
    *PHASE.get() == LEAVING
}

pub fn rewinding() -> bool {
    *PHASE.get() == COMING_BACK
}

pub fn is_terminal(_fd: usize) -> bool {
    false
}

pub fn cores() -> i64 {
    1
}

pub fn thread_id() -> i64 {
    1
}

pub fn catch_stop() {}

/// In nanoseconds since 1970.
pub fn now() -> u64 {
    unsafe { (time() * 1e6) as u64 }
}

pub fn sleep(s: f64) {
    if !arrived() {
        leave();
        unsafe { nap(s) };
    }
}

pub fn pid() -> u64 {
    1
}

pub fn chance(b: &mut [u8]) {
    unsafe { host_chance(b.as_mut_ptr(), b.len()) }
}

// ---- Files ----
// A page has no disk: files live in its memory, from the start of the page to its end, and what
// the user opens with `ui.open_file` comes in there. What is not there may be given by the host
// (the tests read real files, `host.file`).

const MISSING: i64 = 1;
const A_FOLDER: i64 = 3;

/// By path, without `./`; `None` a folder.
static FILES: Global<BTreeMap<String, Option<Vec<u8>>>> = Global::new(BTreeMap::new());

fn cleaned(path: &str) -> String {
    let mut p = path;
    while let Some(rest) = p.strip_prefix("./") {
        p = rest;
    }
    String::from(p.trim_end_matches('/'))
}

/// A file only the host has, kept for good once read.
fn known(path: &str) -> bool {
    let files = FILES.get();
    if files.contains_key(path) {
        return true;
    }
    let n = unsafe { file_size(path.as_ptr(), path.len()) };
    if n == -2 {
        files.insert(path.into(), None);
        return true;
    }
    if n < 0 {
        return false;
    }
    let mut b = alloc::vec![0; n as usize];
    unsafe { file_read(b.as_mut_ptr()) };
    files.insert(path.into(), Some(b));
    true
}

fn parent_there(path: &str) -> bool {
    match path.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => FILES.get().get(parent).is_some_and(Option::is_none),
        _ => true,
    }
}

pub fn kind(path: &str) -> Result<bool, i64> {
    let p = cleaned(path);
    if p.is_empty() {
        return Ok(true);
    }
    if known(&p) { Ok(FILES.get()[&p].is_none()) } else { Err(MISSING) }
}

pub fn info(path: &str, _follow: bool) -> Result<crate::Info, i64> {
    let folder = kind(path)?;
    let size = FILES.get().get(&cleaned(path)).and_then(|f| f.as_ref()).map_or(0, |f| f.len() as i64);
    Ok(crate::Info { folder, link: false, size, modified: (now() / 1_000_000_000) as i64 })
}

pub fn rename(a: &str, b: &str) -> Result<(), i64> {
    let (a, b) = (cleaned(a), cleaned(b));
    if !known(&a) {
        return Err(MISSING);
    }
    let moved = FILES.get().remove(&a).unwrap();
    FILES.get().insert(b, moved);
    Ok(())
}

pub fn read_file(path: &str) -> Result<Vec<u8>, i64> {
    let p = cleaned(path);
    if !known(&p) {
        return Err(MISSING);
    }
    FILES.get()[&p].clone().ok_or(A_FOLDER)
}

pub fn write_file(path: &str, b: &[u8], append: bool) -> Result<(), i64> {
    let p = cleaned(path);
    if !parent_there(&p) {
        return Err(MISSING);
    }
    if kind(&p) == Ok(true) {
        return Err(A_FOLDER);
    }
    let held = FILES.get().entry(p).or_insert_with(|| Some(Vec::new())).as_mut().unwrap();
    if !append {
        held.clear();
    }
    held.extend_from_slice(b);
    // A file named by `ui.save_file` goes to the user's downloads.
    if SAVING.get().as_deref() == Some(path) {
        unsafe { download(path.as_ptr(), path.len(), held.as_ptr(), held.len()) };
    }
    Ok(())
}

/// One folder; Err(0) if something is already there.
pub fn make_dir(path: &str) -> Result<(), i64> {
    let p = cleaned(path);
    if known(&p) {
        return Err(0);
    }
    FILES.get().insert(p, None);
    Ok(())
}

pub fn remove_file(path: &str) -> Result<(), i64> {
    let p = cleaned(path);
    match kind(&p)? {
        true => Err(A_FOLDER),
        false => {
            FILES.get().remove(&p);
            Ok(())
        }
    }
}

pub fn list_dir(path: &str) -> Result<Vec<String>, i64> {
    let p = cleaned(path);
    if !kind(&p)? {
        return Err(4);
    }
    let start = if p.is_empty() { p } else { format!("{p}/") };
    let inside = FILES.get().keys().filter_map(|k| k.strip_prefix(start.as_str())).filter(|k| !k.contains('/'));
    Ok(inside.map(String::from).collect())
}

/// The name `ui.save_file` gave.
static SAVING: Global<Option<String>> = Global::new(None);

pub fn run(_program: &str, _args: &[&str]) -> Result<(Vec<u8>, Vec<u8>, i64), i64> {
    Err(2)
}

// ---- Memory ----
// As on Linux (runtime/linux.rs): powers of two with a list of freed blocks each, cut from pieces
// the module's memory grows by; here every size goes through the lists, a page cannot give any
// back. ponytail: a freed block stays its size for good.

const SIZES: usize = 28;
const PIECE: usize = 1 << 20;

struct Blocks {
    freed: [*mut u8; SIZES],
    next: usize,
    end: usize,
}

static BLOCKS: Global<Blocks> = Global::new(Blocks { freed: [core::ptr::null_mut(); SIZES], next: 0, end: 0 });

fn size_of(n: usize) -> usize {
    n.max(16).next_power_of_two().trailing_zeros() as usize - 4
}

fn grown(n: usize) -> usize {
    let at = core::arch::wasm32::memory_grow(0, n.div_ceil(1 << 16));
    if at == usize::MAX { 0 } else { at << 16 }
}

pub struct Memory;

unsafe impl GlobalAlloc for Memory {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let k = size_of(l.size());
        let b = BLOCKS.get();
        let block = b.freed[k];
        if !block.is_null() {
            b.freed[k] = unsafe { *(block as *mut *mut u8) };
            return block;
        }
        let size = 16usize << k;
        if b.next + size > b.end {
            if size >= PIECE {
                return grown(size) as *mut u8;
            }
            b.next = grown(PIECE);
            if b.next == 0 {
                return core::ptr::null_mut();
            }
            b.end = b.next + PIECE;
        }
        b.next += size;
        (b.next - size) as *mut u8
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let k = size_of(l.size());
        let b = BLOCKS.get();
        unsafe { *(p as *mut *mut u8) = b.freed[k] };
        b.freed[k] = p;
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if size_of(l.size()) == size_of(n) {
            return p;
        }
        unsafe {
            let q = self.alloc(Layout::from_size_align_unchecked(n, l.align()));
            if !q.is_null() {
                core::ptr::copy_nonoverlapping(p, q, l.size().min(n));
                self.dealloc(p, l);
            }
            q
        }
    }
}

// ---- One thread: locks have nobody to keep out ----

pub struct Lock;

impl Lock {
    pub const fn new() -> Lock {
        Lock
    }
    pub fn lock(&self) {}
    pub fn unlock(&self) {}
}

unsafe impl Sync for Lock {}

pub fn not_alone() {}
pub fn wait(_word: &AtomicU32, _seen: u32) {}
pub fn wait_for(_word: &AtomicU32, _seen: u32, _ns: u64) {}
pub fn wake(_word: &AtomicU32) {}

pub struct Thread;

pub fn spawn(_f: extern "C" fn(i64), _arg: i64) -> Option<Thread> {
    None
}

pub fn join(_t: Thread) {}

pub fn library(_name: &str) -> Option<usize> {
    None
}

pub fn symbol(_library: usize, _name: &str) -> Option<usize> {
    None
}

// ---- Windows: a view becomes elements of the page (runtime/web.js) ----

/// What the page last gave back, into `typed`.
fn answer(typed: &mut String) -> i64 {
    let mut b = alloc::vec![0; 1 << 12];
    let clicked = unsafe { ui_clicked() } as i64;
    let n = unsafe { ui_typed(b.as_mut_ptr(), b.len()) } as usize;
    if n > b.len() {
        b.resize(n, 0);
        unsafe { ui_typed(b.as_mut_ptr(), b.len()) };
    }
    b.truncate(n);
    *typed = String::from_utf8_lossy(&b).into_owned();
    for k in 0..unsafe { ui_rooms() } {
        crate::ui_room(k as usize, unsafe { ui_room(k, 0) }, unsafe { ui_room(k, 1) });
    }
    clicked
}

/// `ui.show`, back from waiting: what happened.
pub fn ui_heard(typed: &mut String) -> Option<i64> {
    arrived().then(|| answer(typed))
}

/// Shows the view, then the program leaves until something happens.
pub fn ui_show(title: &str, view: &UiNode, menus: &[UiMenu], width: i32, height: i32, acts: &[i64], _typed: &mut String) -> i64 {
    let o = crate::view::message(title, view, menus, width, height, acts);
    match unsafe { ui_render(o.as_ptr(), o.len()) } {
        0 => {}
        1 => crate::gl_stop(crate::GPU_MISSING, b""),
        _ => {
            let mut log = String::new();
            answer(&mut log);
            crate::gl_stop(crate::GPU_MISSING + 1, log.as_bytes());
        }
    }
    leave();
    0
}

/// The window's then the screen's x, y, width and height.
pub fn ui_areas() -> Option<[i32; 8]> {
    let mut areas = [0; 8];
    areas.iter_mut().enumerate().for_each(|(k, a)| *a = unsafe { ui_area(k as i32) });
    Some(areas)
}

/// `ui.message` and `ui.confirm` answer at once (the browser's own dialogs). `ui.open_file` waits
/// for the user's file, which comes into the page's memory; `ui.save_file` gives a name, and
/// writing to that name downloads the file. No folder can be picked.
pub fn ui_dialog(kind: i64, title: &str, text: &str) -> Option<String> {
    match kind {
        0 if !arrived() => {
            leave();
            unsafe { pick_file() };
            None
        }
        0 => {
            let n = unsafe { picked() };
            if n < 0 {
                return None;
            }
            let mut b = alloc::vec![0; n as usize];
            unsafe { file_read(b.as_mut_ptr()) };
            let mut name = String::new();
            answer(&mut name);
            FILES.get().insert(name.clone(), Some(b));
            Some(name)
        }
        1 => {
            let name = String::from(if text.is_empty() { "kova.txt" } else { text });
            *SAVING.get() = Some(name.clone());
            Some(name)
        }
        2 => None,
        _ => (unsafe { ui_ask(kind as i32, title.as_ptr(), title.len(), text.as_ptr(), text.len()) } != 0).then(|| "yes".into()),
    }
}

// ---- The web: `fetch`, so a site must allow other origins (CORS) ----

/// `None` while the program leaves to wait.
pub fn fetch(r: &crate::http::Request) -> Option<Result<crate::http::Answer, Fail>> {
    if !arrived() {
        if crate::http::parse_url(r.url).is_none() {
            return Some(Err(Fail::BadUrl));
        }
        let mut o = String::from("{\"url\":");
        quoted(&mut o, r.url);
        o.push_str(",\"method\":");
        quoted(&mut o, r.method);
        o.push_str(&format!(",\"follow\":{},\"ms\":{},\"headers\":", r.follow, r.ms));
        list(&mut o, &r.headers, |o, (name, value)| {
            o.push('[');
            quoted(o, name);
            o.push(',');
            quoted(o, value);
            o.push(']');
        });
        o.push('}');
        leave();
        unsafe { fetch_start(o.as_ptr(), o.len(), r.body.as_ptr(), r.body.len()) };
        return None;
    }
    let n = unsafe { fetched() };
    if n < 0 {
        return Some(Err(if n == -2 { Fail::Timeout } else { Fail::Refused }));
    }
    let mut b = alloc::vec![0; n as usize];
    unsafe { file_read(b.as_mut_ptr()) };
    Some(Ok(crate::view::answer_of(&b)))
}

pub fn sound(whole: &[u8], _wav: &crate::shared::Wav) -> bool {
    if !arrived() {
        leave();
        unsafe { play(whole.as_ptr(), whole.len()) };
    }
    true
}

// ---- No sockets from a page ----

pub struct Net;

impl Network for Net {
    type Link = usize;
    fn open(&mut self, _host: &str, _port: u16, _secure: bool, _ms: u64) -> Result<usize, Fail> {
        Err(Fail::Refused)
    }
    fn send(&mut self, _link: &mut usize, _data: &[u8]) -> Result<(), Fail> {
        Err(Fail::Broken)
    }
    fn receive(&mut self, _link: &mut usize, _buffer: &mut [u8]) -> Result<usize, Fail> {
        Err(Fail::Broken)
    }
    fn wake(&mut self, _link: &usize) {}
    fn close(&mut self, _link: usize) {}
}

impl Serve for Net {
    type Listener = ();
    type Datagram = ();
    fn listen(&mut self, _port: u16, _everyone: bool) -> Option<()> {
        None
    }
    fn accept(&mut self, _listener: &mut (), _ms: u64) -> Option<(usize, [u8; 4])> {
        None
    }
    fn ready(&mut self, _listener: &(), links: &[usize]) -> Vec<bool> {
        alloc::vec![false; links.len() + 1]
    }
    fn addresses(&mut self) -> Vec<(String, [u8; 4])> {
        Vec::new()
    }
    fn stop(&mut self, _listener: ()) {}
    fn port(&mut self, _listener: &()) -> u16 {
        0
    }
    fn datagram_port(&mut self, _socket: &()) -> u16 {
        0
    }
    fn udp(&mut self, _port: u16, _everyone: bool, _ms: u64) -> Option<()> {
        None
    }
    fn send_to(&mut self, _socket: &(), _host: &str, _port: u16, _data: &[u8]) -> Result<(), Fail> {
        Err(Fail::Broken)
    }
    fn receive_from(&mut self, _socket: &(), _buffer: &mut [u8]) -> Result<(usize, [u8; 4], u16), Fail> {
        Err(Fail::Broken)
    }
    fn close_datagram(&mut self, _socket: ()) {}
}

