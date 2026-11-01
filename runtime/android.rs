// What Android changes in Linux's runtime (PLAN Part 4 "Android"): the system starts an app itself
// and loads the program as a library, so there is no `_start`, and text goes to the system's log.
// System calls stay Linux's. Compiled with `--cfg android` (build.rs).

use core::ptr::null_mut;

use alloc::string::String;
use alloc::vec;

use crate::Global;

/// Android's `ANativeActivity` (`android/native_activity.h`), which it gives to `ANativeActivity_onCreate`.
#[repr(C)]
pub struct Activity {
    callbacks: *mut u8,
    vm: *mut u8,
    env: *mut u8,
    clazz: *mut u8,
    /// The app's own folder: its working folder.
    internal_data_path: *const u8,
    external_data_path: *const u8,
    sdk_version: i32,
    instance: *mut u8,
    asset_manager: *mut u8,
    obb_path: *const u8,
}

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
    fn AConfiguration_new() -> *mut u8;
    fn AConfiguration_fromAssetManager(config: *mut u8, assets: *mut u8);
    fn AConfiguration_getLanguage(config: *mut u8, out: *mut u8);
}

/// The phone's language, from `ANativeActivity_onCreate`.
static LANGUAGE: Global<[u8; 2]> = Global::new([0; 2]);

/// The program runs on a thread of its own: Android's first thread must keep answering it.
extern "C" fn program(_: *mut u8) -> *mut u8 {
    crate::run()
}

/// Android calls this on its first thread when the app opens.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ANativeActivity_onCreate(activity: *mut Activity, _saved: *mut u8, _size: usize) {
    unsafe {
        chdir((*activity).internal_data_path);
        let config = AConfiguration_new();
        AConfiguration_fromAssetManager(config, (*activity).asset_manager);
        AConfiguration_getLanguage(config, LANGUAGE.get() as *const _ as *mut u8);
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

pub fn language() -> Option<String> {
    let l = LANGUAGE.get();
    (l[0] != 0).then(|| String::from_utf8_lossy(&l[..]).into_owned())
}

pub fn var(name: &str) -> Option<String> {
    let mut asked = alloc::vec::Vec::from(name.as_bytes());
    asked.push(0);
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

/// One line of the system's log for each line of `b` (`adb logcat -s kova`).
pub fn log(b: &[u8]) {
    let mut line = [0u8; 1000];
    for piece in b.trim_ascii_end().split(|&c| c == b'\n') {
        for part in piece.chunks(line.len() - 1).chain(piece.is_empty().then_some(&[][..])) {
            line[..part.len()].copy_from_slice(part);
            line[part.len()] = 0;
            unsafe { __android_log_write(4, c"kova".as_ptr() as *const u8, line.as_ptr()) };
        }
    }
}
