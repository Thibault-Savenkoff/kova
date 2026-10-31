// What the runtime asks of the browser (`kova build --target web`, PLAN Part 4 "WebAssembly"):
// a few functions the page's JavaScript gives (runtime/web.js), and an allocator over the module's
// memory. No files, threads or sockets here: `kova build` refuses the programs that use them.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::AtomicU32;

use crate::Global;
use crate::http::{Fail, Network, Serve};

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

// ---- No files in a page: every path answers "denied" (`IoError.Denied`) ----

const DENIED: i64 = 2;

pub fn kind(_path: &str) -> Result<bool, i64> {
    Err(DENIED)
}

pub fn info(_path: &str, _follow: bool) -> Result<crate::Info, i64> {
    Err(DENIED)
}

pub fn rename(_a: &str, _b: &str) -> Result<(), i64> {
    Err(DENIED)
}

pub fn read_file(_path: &str) -> Result<Vec<u8>, i64> {
    Err(DENIED)
}

pub fn write_file(_path: &str, _b: &[u8], _append: bool) -> Result<(), i64> {
    Err(DENIED)
}

pub fn make_dir(_path: &str) -> Result<(), i64> {
    Err(DENIED)
}

pub fn remove_file(_path: &str) -> Result<(), i64> {
    Err(DENIED)
}

pub fn list_dir(_path: &str) -> Result<Vec<String>, i64> {
    Err(DENIED)
}

pub fn run(_program: &str, _args: &[&str]) -> Result<(Vec<u8>, Vec<u8>, i64), i64> {
    Err(DENIED)
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

pub fn ui_show(_title: &str, _view: &crate::shared::UiNode, _menus: &[crate::shared::UiMenu], _width: i32, _height: i32, _acts: &[i64], _typed: &mut String) -> i64 {
    -1
}

pub fn ui_areas() -> Option<[i32; 8]> {
    None
}

pub fn ui_dialog(_kind: i64, _title: &str, _text: &str) -> Option<String> {
    None
}

pub fn sound(_whole: &[u8], _wav: &crate::shared::Wav) -> bool {
    false
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

