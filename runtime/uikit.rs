// `ui.show` on iOS (PLAN Part 4 "iOS", step 2): UIKit through the Objective-C runtime, as
// `macos.rs` does AppKit. A child module of `macos.rs`, built only with `--cfg ios`.
//
// UIKit owns the main thread (`UIApplicationMain` never returns), so the Kova program runs on a
// second thread, started once the app has a window. `show` (the program's thread) hands the view
// to the main thread with `dispatch_sync_f`, which draws it, then sleeps until a tap, a keystroke
// or a clock puts an event in `HEARD`. What is drawn: texts, buttons, fields, check boxes, sliders,
// choices, images, lists (as rows of buttons), canvases (2D drawing and touches), dialogs, sound,
// columns and rows with their style and margins, tables with their headers, `ui.gpu` (OpenGL ES).

use super::{library, symbol, Id, Insets, Kit, Rect, Size};
use crate::shared::{UiAlign, UiNode, UiQueue, UiStyle};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

/// What the user did, until the program asks.
static HEARD: crate::Locked<UiQueue> = crate::Locked::new(UiQueue::new());
/// Changes at each event: the program's thread sleeps on it.
static RING: AtomicU32 = AtomicU32::new(0);

fn hear(answer: i64, text: String) {
    HEARD.with(|q| q.hear(answer, text));
    RING.fetch_add(1, SeqCst);
    super::wake(&RING);
}

/// What the main thread keeps of the window.
struct Screen {
    kit: Kit,
    /// The menus now shown (a row of buttons above the content).
    menus: Vec<crate::shared::UiMenu>,
    /// The object that hears the buttons and fields (`KovaUiTarget`).
    target: Id,
    /// The view of the window's controller.
    root: Id,
    /// The scroll view that fills its safe area: the content is in it.
    scroll: Id,
    /// The tab bar of `ui.tabs`, pinned to the bottom of the screen as iOS does (null for none).
    bar: Id,
    /// The window's view controller, which shows the dialogs.
    controller: Id,
    /// The stack view now shown.
    content: Id,
    /// The pushed page: its controller, its scroll view, its content (all null when there is none), and the event its back button sends.
    pushed: Id,
    page_scroll: Id,
    page_content: Id,
    back: usize,
    shape: String,
    /// Each text, button and field now shown, with the label that goes with it (0: none).
    made: Vec<(Id, Id)>,
    clocks: Vec<(i32, usize)>,
    timers: Vec<Id>,
}

/// Only the main thread reaches it.
static SCREEN: crate::Global<Option<Screen>> = crate::Global::new(None);
/// Each canvas now on screen (the main thread's).
static CANVASES: crate::Global<Vec<(Id, crate::shared::UiCanvas)>> = crate::Global::new(Vec::new());
/// The next text is built without its width cap (`.no_wrap`).
static WIDE: crate::Global<bool> = crate::Global::new(false);
/// The scroll view, whose size a `ui.fill` canvas follows.
static SCROLL: crate::Global<Id> = crate::Global::new(core::ptr::null_mut());
/// The window of the app, kept for the delegate.
static WINDOW: crate::Global<Id> = crate::Global::new(core::ptr::null_mut());

fn objc_kit() -> Option<Kit> {
    let objc = library("/usr/lib/libobjc.A.dylib")?;
    let f = |name: &str| symbol(objc, name).unwrap_or(0);
    let send = f("objc_msgSend");
    unsafe {
        use core::mem::transmute as t;
        Some(Kit {
            class: t(f("objc_getClass")),
            selector: t(f("sel_registerName")),
            send: t(send),
            send_int: t(send),
            send_id: t(send),
            send_two: t(send),
            send_int_of: t(send),
            send_text: t(send),
            send_float: t(send),
            send_size: t(send),
            send_insets: t(send),
            fitting: t(send),
            window_init: t(send),
            event: t(send),
            push: t(f("objc_autoreleasePoolPush")),
            pop: t(f("objc_autoreleasePoolPop")),
            window: core::ptr::null_mut(),
            target: core::ptr::null_mut(),
            canvas: core::ptr::null_mut(),
        })
    }
}

/// A message with other arguments than `Kit` has a prototype for.
macro_rules! msg {
    ($kit:expr, $ty:ty) => {
        unsafe { core::mem::transmute::<usize, $ty>($kit.send as usize) }
    };
}

// ---- import ios ----

/// The class of `ios.handler` objects, and each one made with the number it reports.
static HANDLER_CLASS: crate::Global<Id> = crate::Global::new(core::ptr::null_mut());
static HANDLERS: crate::Global<Vec<(Id, i64)>> = crate::Global::new(Vec::new());

extern "C" fn handler_invoke(me: Id, _message: Id, _sender: Id) {
    if let Some(&(_, tag)) = HANDLERS.get().iter().find(|e| e.0 == me) {
        hear(tag, String::new());
    }
}

/// The arguments of a message as `ios.kova` writes them: `n;` `i3;` `f1.5;` `b1;` `o1234;` `d1 2 3 4;` and
/// `s5:hello` / `l9:setText:` (a text and a selector, with their length in bytes).
enum Arg {
    Nothing,
    Int(i64),
    Float(f64),
    Object(Id),
    Selector(String),
    Text(String),
    Numbers(Vec<f64>),
}

fn ios_args(mut b: &[u8]) -> Option<Vec<Arg>> {
    let mut all = Vec::new();
    while let Some((&kind, rest)) = b.split_first() {
        b = rest;
        if matches!(kind, b's' | b'l') {
            let colon = b.iter().position(|&c| c == b':')?;
            let n: usize = core::str::from_utf8(&b[..colon]).ok()?.parse().ok()?;
            let text = String::from_utf8_lossy(b.get(colon + 1..colon + 1 + n)?).into_owned();
            b = &b[colon + 1 + n..];
            all.push(if kind == b's' { Arg::Text(text) } else { Arg::Selector(text) });
            continue;
        }
        let end = b.iter().position(|&c| c == b';')?;
        let word = core::str::from_utf8(&b[..end]).ok()?;
        b = &b[end + 1..];
        all.push(match kind {
            b'n' => Arg::Nothing,
            b'i' => Arg::Int(word.parse().ok()?),
            b'b' => Arg::Int((word == "1") as i64),
            b'o' => Arg::Object(word.parse::<i64>().ok()? as Id),
            b'f' => Arg::Float(crate::shared::parse_float(word)?),
            b'd' => Arg::Numbers(word.split(' ').filter(|w| !w.is_empty()).map(|w| crate::shared::parse_float(w)).collect::<Option<Vec<_>>>()?),
            _ => return None,
        });
    }
    Some(all)
}

/// The fields of a C structure's encoding, `{CGRect={CGPoint=dd}{CGSize=dd}}` giving `dddd`.
fn struct_fields(encoding: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut named = false;
    for &c in encoding {
        match c {
            b'{' => {
                depth += 1;
                named = false;
            }
            b'}' => depth -= 1,
            b'=' if depth > 0 && !named => named = true,
            c if depth > 0 && named => out.push(c),
            _ => {}
        }
    }
    out
}

/// Sends the message `name` to `object` with `args`, through `NSInvocation`: whatever the selector's types are.
/// The answer as `ios.kova` reads it: `n`, `i3`, `f1.5`, `b1`, `sHello`, `o1234`, `d1 2 3 4`, or `!` and a complaint.
fn ios_send(kit: &Kit, object: Id, name: &str, args: &[u8]) -> String {
    use alloc::format;
    if object.is_null() {
        return format!("!sending {name} to nil");
    }
    let Some(args) = ios_args(args) else { return format!("!the arguments of {name} are badly written") };
    let sel = kit.sel(name);
    let answers = (kit.send_two)(object, kit.sel("respondsToSelector:"), sel) as usize & 0xff != 0;
    if !answers {
        return format!("!the object does not answer {name}");
    }
    let sig = (kit.send_two)(object, kit.sel("methodSignatureForSelector:"), sel);
    let wanted = (kit.send_int)(sig, kit.sel("numberOfArguments")) as usize;
    if wanted != args.len() + 2 {
        return format!("!{name} takes {} arguments, not {}", wanted.saturating_sub(2), args.len());
    }
    let inv = (kit.send_two)(kit.class("NSInvocation"), kit.sel("invocationWithMethodSignature:"), sig);
    (kit.send_two)(inv, kit.sel("setTarget:"), object);
    (kit.send_two)(inv, kit.sel("setSelector:"), sel);
    let type_at = |i: usize| -> Vec<u8> {
        let p = msg!(kit, extern "C" fn(Id, Id, u64) -> Id)(sig, kit.sel("getArgumentTypeAtIndex:"), i as u64) as *const u8;
        let mut t = unsafe { crate::c_text(p) }.to_vec();
        while t.first().is_some_and(|c| b"rnNoORV".contains(c)) {
            t.remove(0);
        }
        t
    };
    for (k, arg) in args.iter().enumerate() {
        let t = type_at(k + 2);
        let mut buffer = [0u8; 64];
        let set = |buffer: &mut [u8; 64], bytes: &[u8]| buffer[..bytes.len()].copy_from_slice(bytes);
        match (t.first().copied().unwrap_or(b'?'), arg) {
            (b'f', Arg::Float(x)) => set(&mut buffer, &(*x as f32).to_le_bytes()),
            (b'f', Arg::Int(n)) => set(&mut buffer, &(*n as f32).to_le_bytes()),
            (b'd', Arg::Float(x)) => set(&mut buffer, &x.to_le_bytes()),
            (b'd', Arg::Int(n)) => set(&mut buffer, &(*n as f64).to_le_bytes()),
            (b'c' | b'C' | b'B' | b's' | b'S' | b'i' | b'I' | b'l' | b'L' | b'q' | b'Q' | b'*' | b'^', Arg::Int(n)) => set(&mut buffer, &n.to_le_bytes()),
            (b'c' | b'C' | b'B' | b'q' | b'Q', Arg::Float(x)) => set(&mut buffer, &(*x as i64).to_le_bytes()),
            (b'@' | b'#', Arg::Nothing) => {}
            (b'@' | b'#', Arg::Object(o)) => set(&mut buffer, &(*o as usize).to_le_bytes()),
            (b'@', Arg::Text(s)) => set(&mut buffer, &(kit.text(s) as usize).to_le_bytes()),
            (b'@', Arg::Int(n)) => set(&mut buffer, &(msg!(kit, extern "C" fn(Id, Id, i64) -> Id)(kit.class("NSNumber"), kit.sel("numberWithLongLong:"), *n) as usize).to_le_bytes()),
            (b':', Arg::Selector(s)) => set(&mut buffer, &(kit.sel(s) as usize).to_le_bytes()),
            (b'{', Arg::Numbers(xs)) => {
                let fields = struct_fields(&t);
                if fields.len() != xs.len() || fields.len() * 8 > 64 {
                    return format!("!argument {} of {name} is a structure of {} numbers, not {}", k + 1, fields.len(), xs.len());
                }
                for (j, (&f, x)) in fields.iter().zip(xs).enumerate() {
                    match f {
                        b'd' => buffer[j * 8..j * 8 + 8].copy_from_slice(&x.to_le_bytes()),
                        b'q' | b'Q' => buffer[j * 8..j * 8 + 8].copy_from_slice(&(*x as i64).to_le_bytes()),
                        _ => return format!("!argument {} of {name}: a structure that is not made of numbers", k + 1),
                    }
                }
            }
            _ => return format!("!argument {} of {name} is not of the kind it wants ({})", k + 1, String::from_utf8_lossy(&t)),
        }
        msg!(kit, extern "C" fn(Id, Id, *const u8, i64))(inv, kit.sel("setArgument:atIndex:"), buffer.as_ptr(), (k + 2) as i64);
    }
    kit.send(inv, "invoke");
    let ret = {
        let p = kit.send(sig, "methodReturnType") as *const u8;
        let mut t = unsafe { crate::c_text(p) }.to_vec();
        while t.first().is_some_and(|c| b"rnNoORV".contains(c)) {
            t.remove(0);
        }
        t
    };
    let mut buffer = [0u8; 64];
    let kind = ret.first().copied().unwrap_or(b'v');
    if kind != b'v' {
        msg!(kit, extern "C" fn(Id, Id, *mut u8))(inv, kit.sel("getReturnValue:"), buffer.as_mut_ptr());
    }
    let word = |n: usize| u64::from_le_bytes(buffer[n..n + 8].try_into().unwrap());
    match kind {
        b'v' => "n".into(),
        b'B' | b'c' | b'C' if kind == b'B' => format!("b{}", buffer[0] & 1),
        b'c' => format!("i{}", buffer[0] as i8),
        b'C' => format!("i{}", buffer[0]),
        b's' => format!("i{}", i16::from_le_bytes([buffer[0], buffer[1]])),
        b'S' => format!("i{}", u16::from_le_bytes([buffer[0], buffer[1]])),
        b'i' => format!("i{}", i32::from_le_bytes(buffer[..4].try_into().unwrap())),
        b'I' => format!("i{}", u32::from_le_bytes(buffer[..4].try_into().unwrap())),
        b'l' | b'q' => format!("i{}", word(0) as i64),
        b'L' | b'Q' => format!("i{}", word(0) as i64),
        b'f' => format!("f{}", f32::from_le_bytes(buffer[..4].try_into().unwrap())),
        b'd' => format!("f{}", f64::from_le_bytes(buffer[..8].try_into().unwrap())),
        b'@' | b'#' => {
            let o = word(0) as usize as Id;
            if o.is_null() {
                "n".into()
            } else {
                // Kept alive: the pool this runs in is emptied when the call is over.
                kit.send(o, "retain");
                format!("o{}", o as usize)
            }
        }
        b'{' => {
            let fields = struct_fields(&ret);
            let numbers: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(j, &f)| {
                    let w = word(j * 8);
                    if f == b'd' { format!("{}", f64::from_bits(w)) } else { format!("{}", w as i64) }
                })
                .collect();
            format!("d{}", numbers.join(" "))
        }
        _ => format!("!{name} gives a value of a kind Kova cannot read ({})", String::from_utf8_lossy(&ret)),
    }
}

/// `ios.raw`: kind 10 sends a message, 11 waits for a handler, 12 makes one, 13 gives the window's controller and
/// 14 finds a class.
fn ios_raw(kind: i64, title: &str, text: &str) -> Option<String> {
    if kind == 11 {
        loop {
            let seen = RING.load(SeqCst);
            if let Some((answer, _)) = HEARD.with(|q| q.next()) {
                return Some(alloc::format!("i{answer}"));
            }
            super::wait(&RING, seen);
        }
    }
    struct Job<'a> {
        kind: i64,
        title: &'a str,
        text: &'a str,
        out: String,
    }
    extern "C" fn run(job: *mut u8) {
        let job = unsafe { &mut *(job as *mut Job) };
        let screen = screen();
        let kit = &screen.kit;
        let pool = (kit.push)();
        job.out = match job.kind {
            10 => {
                let (object, args) = job.text.split_once('\n').unwrap_or((job.text, ""));
                match object.parse::<i64>() {
                    Ok(o) => ios_send(kit, o as usize as Id, job.title, args.as_bytes()),
                    Err(_) => "!the object is badly written".into(),
                }
            }
            12 => {
                let handler = kit.send(kit.send(*HANDLER_CLASS.get(), "alloc"), "init");
                HANDLERS.get().push((handler, job.text.parse().unwrap_or(0)));
                alloc::format!("o{}", handler as usize)
            }
            13 => alloc::format!("o{}", screen.controller as usize),
            14 => {
                let class = kit.class(job.title);
                if class.is_null() { "n".into() } else { alloc::format!("o{}", class as usize) }
            }
            _ => "!no such request".into(),
        };
        (kit.pop)(pool);
    }
    if SCREEN.get().is_none() {
        return None;
    }
    let system = library("/usr/lib/libSystem.B.dylib")?;
    let sync: extern "C" fn(usize, *mut u8, extern "C" fn(*mut u8)) = unsafe { core::mem::transmute(symbol(system, "dispatch_sync_f")?) };
    let mut job = Job { kind, title, text, out: String::new() };
    sync(symbol(system, "_dispatch_main_q")?, &mut job as *mut Job as *mut u8, run);
    Some(job.out)
}

/// Each view with a context menu now on screen, and its items (label, button number).
static CONTEXTS: crate::Global<Vec<(Id, Vec<(String, usize)>)>> = crate::Global::new(Vec::new());
/// The menu the next call of `provide` gives back.
static PROVIDED: crate::Global<Id> = crate::Global::new(core::ptr::null_mut());

extern "C" fn provide(_block: Id, _suggested: Id) -> Id {
    *PROVIDED.get()
}

/// A view was pressed and held: the menu its `.context` described.
extern "C" fn context_for(_me: Id, _message: Id, interaction: Id, _x: f64, _y: f64) -> Id {
    let kit = &screen().kit;
    let view = kit.send(interaction, "view");
    let Some(items) = CONTEXTS.get().iter().find(|e| e.0 == view).map(|e| e.1.clone()) else { return core::ptr::null_mut() };
    let actions: Vec<Id> = items
        .iter()
        .map(|(label, n)| {
            msg!(kit, extern "C" fn(Id, Id, Id, Id, Id, Id) -> Id)(
                kit.class("UIAction"),
                kit.sel("actionWithTitle:image:identifier:handler:"),
                kit.text(label),
                core::ptr::null_mut(),
                kit.text(&alloc::format!("{n}")),
                global_block(context_chosen as usize),
            )
        })
        .collect();
    let array = msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), actions.as_ptr(), actions.len() as u64);
    // UIKit asks for the menu later, after this call's autorelease pool is emptied: keep it, let go of the last one.
    let menu = msg!(kit, extern "C" fn(Id, Id, Id, Id) -> Id)(kit.class("UIMenu"), kit.sel("menuWithTitle:children:"), kit.text(""), array);
    kit.send(menu, "retain");
    let old = core::mem::replace(PROVIDED.get(), menu);
    if !old.is_null() {
        kit.send(old, "release");
    }
    msg!(kit, extern "C" fn(Id, Id, Id, Id, Id) -> Id)(
        kit.class("UIContextMenuConfiguration"),
        kit.sel("configurationWithIdentifier:previewProvider:actionProvider:"),
        core::ptr::null_mut(),
        core::ptr::null_mut(),
        global_block(provide as usize),
    )
}

/// An item of a context menu was chosen: its identifier is the number of its button.
extern "C" fn context_chosen(_block: Id, action: Id) {
    let Some(screen) = SCREEN.get().as_ref() else { return };
    let kit = &screen.kit;
    if let Ok(n) = text_of(kit, kit.send(action, "identifier")).parse::<usize>() {
        hear(n as i64, String::new());
    }
}


/// A new object, owned by the current pool.
fn made(kit: &Kit, class: &str) -> Id {
    kit.send(kit.send(kit.send(kit.class(class), "alloc"), "init"), "autorelease")
}

/// The program starts here, on the main thread: UIKit takes it.
pub fn launch(argc: i32, argv: *const *const u8) -> i32 {
    let (Some(uikit), Some(objc), Some(kit)) = (library("/System/Library/Frameworks/UIKit.framework/UIKit"), library("/usr/lib/libobjc.A.dylib"), objc_kit()) else { return 1 };
    let f = |name: &str| symbol(objc, name).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let new_class: extern "C" fn(Id, *const u8, usize) -> Id = t(f("objc_allocateClassPair"));
        let add: extern "C" fn(Id, Id, usize, *const u8) -> bool = t(f("class_addMethod"));
        let register: extern "C" fn(Id) = t(f("objc_registerClassPair"));
        let class = new_class(kit.class("NSObject"), c"KovaApp".as_ptr() as *const u8, 0);
        add(class, kit.sel("application:didFinishLaunchingWithOptions:"), finished as usize, c"c@:@@".as_ptr() as *const u8);
        add(class, kit.sel("window"), window as usize, c"@@:".as_ptr() as *const u8);
        add(class, kit.sel("setWindow:"), set_window as usize, c"v@:@".as_ptr() as *const u8);
        register(class);
        let main: extern "C" fn(i32, *const *const u8, Id, Id) -> i32 = t(symbol(uikit, "UIApplicationMain").unwrap_or(0));
        main(argc, argv, core::ptr::null_mut(), kit.text("KovaApp"))
    }
}

extern "C" fn window(_me: Id, _message: Id) -> Id {
    *WINDOW.get()
}

extern "C" fn set_window(_me: Id, _message: Id, w: Id) {
    *WINDOW.get() = w;
}

extern "C" fn uncaught(exception: Id) {
    if let Some(kit) = objc_kit() {
        let name = text_of(&kit, kit.send(exception, "name"));
        let reason = text_of(&kit, kit.send(exception, "reason"));
        super::write(2, alloc::format!("error: {name}: {reason}\n").as_bytes());
    }
    super::exit(101);
}

/// The app has started: a window with an empty view, then the program.
extern "C" fn finished(_me: Id, _message: Id, _app: Id, _options: Id) -> bool {
    let Some(mut kit) = objc_kit() else { return false };
    unsafe {
        use core::mem::transmute as t;
        let objc = library("/usr/lib/libobjc.A.dylib").unwrap();
        // An Objective-C exception no program catches (a wrong selector in `import ios`): say it, then stop with 101.
        if let Some(foundation) = library("/System/Library/Frameworks/Foundation.framework/Foundation") {
            if let Some(set) = symbol(foundation, "NSSetUncaughtExceptionHandler") {
                let set: extern "C" fn(extern "C" fn(Id)) = t(set);
                set(uncaught);
            }
        }
        let f = |name: &str| symbol(objc, name).unwrap_or(0);
        let new_class: extern "C" fn(Id, *const u8, usize) -> Id = t(f("objc_allocateClassPair"));
        let add: extern "C" fn(Id, Id, usize, *const u8) -> bool = t(f("class_addMethod"));
        let register: extern "C" fn(Id) = t(f("objc_registerClassPair"));
        let (action, ret) = (c"v@:@".as_ptr() as *const u8, c"c@:@".as_ptr() as *const u8);
        let class = new_class(kit.class("NSObject"), c"KovaUiTarget".as_ptr() as *const u8, 0);
        add(class, kit.sel("tapped:"), tapped as usize, action);
        add(class, kit.sel("edited:"), edited as usize, action);
        add(class, kit.sel("switched:"), switched as usize, action);
        add(class, kit.sel("slid:"), slid as usize, action);
        add(class, kit.sel("chosen:"), chosen as usize, action);
        add(class, kit.sel("tick:"), tick as usize, action);
        add(class, kit.sel("rowed:"), rowed as usize, action);
        add(class, kit.sel("dated:"), dated as usize, action);
        // What `import ios` gives to controls: an object that tells the program it was called.
        let handler = new_class(kit.class("NSObject"), c"KovaUiHandler".as_ptr() as *const u8, 0);
        add(handler, kit.sel("invoke:"), handler_invoke as usize, action);
        register(handler);
        *HANDLER_CLASS.get() = handler;
        add(class, kit.sel("contextMenuInteraction:configurationForMenuAtLocation:"), context_for as usize, c"@@:@{CGPoint=dd}".as_ptr() as *const u8);
        add(class, kit.sel("documentPicker:didPickDocumentsAtURLs:"), picked_file as usize, c"v@:@@".as_ptr() as *const u8);
        add(class, kit.sel("documentPickerWasCancelled:"), picked_nothing as usize, action);
        add(class, kit.sel("stepped:"), stepped as usize, action);
        add(class, kit.sel("tabBar:didSelectItem:"), tab_chosen as usize, c"v@:@@".as_ptr() as *const u8);
        add(class, kit.sel("navigationController:didShowViewController:animated:"), page_shown as usize, c"v@:@@c".as_ptr() as *const u8);
        add(class, kit.sel("textFieldShouldReturn:"), returned as usize, ret);
        register(class);
        let target = kit.send(kit.send(class, "alloc"), "init");
        // The view of a `ui.canvas`: it draws what the program says and hears touches.
        let canvas = new_class(kit.class("UIView"), c"KovaUiCanvas".as_ptr() as *const u8, 0);
        add(canvas, kit.sel("drawRect:"), draw_canvas as usize, c"v@:{CGRect={CGPoint=dd}{CGSize=dd}}".as_ptr() as *const u8);
        add(canvas, kit.sel("touchesBegan:withEvent:"), touch_began as usize, c"v@:@@".as_ptr() as *const u8);
        add(canvas, kit.sel("touchesMoved:withEvent:"), touch_moved as usize, c"v@:@@".as_ptr() as *const u8);
        add(canvas, kit.sel("touchesEnded:withEvent:"), touch_ended as usize, c"v@:@@".as_ptr() as *const u8);
        add(canvas, kit.sel("touchesCancelled:withEvent:"), touch_ended as usize, c"v@:@@".as_ptr() as *const u8);
        register(canvas);
        kit.canvas = canvas;
        // The view of a `ui.gpu`: its layer is an OpenGL ES surface (`+layerClass`), painted by `gpu_paint`.
        let gpu = new_class(kit.class("UIView"), c"KovaUiGpu".as_ptr() as *const u8, 0);
        let meta: extern "C" fn(Id) -> Id = t(f("object_getClass"));
        add(meta(gpu), kit.sel("layerClass"), gpu_layer as usize, c"#@:".as_ptr() as *const u8);
        add(gpu, kit.sel("touchesBegan:withEvent:"), touch_began as usize, c"v@:@@".as_ptr() as *const u8);
        add(gpu, kit.sel("touchesMoved:withEvent:"), touch_moved as usize, c"v@:@@".as_ptr() as *const u8);
        add(gpu, kit.sel("touchesEnded:withEvent:"), touch_ended as usize, c"v@:@@".as_ptr() as *const u8);
        add(gpu, kit.sel("touchesCancelled:withEvent:"), touch_ended as usize, c"v@:@@".as_ptr() as *const u8);
        register(gpu);
        let bounds = msg!(kit, extern "C" fn(Id, Id) -> Rect)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("bounds"));
        let w = msg!(kit, extern "C" fn(Id, Id, Rect) -> Id)(kit.send(kit.class("UIWindow"), "alloc"), kit.sel("initWithFrame:"), bounds);
        let controller = kit.send(kit.send(kit.class("UIViewController"), "alloc"), "init");
        let root = kit.send(controller, "view");
        (kit.send_two)(root, kit.sel("setBackgroundColor:"), kit.send(kit.class("UIColor"), "systemBackgroundColor"));
        // The controller sits in a navigation controller: its bar shows the title and the menus.
        let nav = (kit.send_two)(kit.send(kit.class("UINavigationController"), "alloc"), kit.sel("initWithRootViewController:"), controller);
        (kit.send_two)(w, kit.sel("setRootViewController:"), nav);
        (kit.send_two)(nav, kit.sel("setDelegate:"), target);
        // Large titles that shrink when the content scrolls, as in the system's own apps.
        (kit.send_id)(kit.send(nav, "navigationBar"), kit.sel("setPrefersLargeTitles:"), 1);
        (kit.send_id)(kit.send(controller, "navigationItem"), kit.sel("setLargeTitleDisplayMode:"), 1);
        // The content scrolls when it is taller than the screen; the keyboard goes away when it is dragged.
        let scroll = made(&kit, "UIScrollView");
        (kit.send_id)(scroll, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
        (kit.send_id)(scroll, kit.sel("setKeyboardDismissMode:"), 1);
        *SCROLL.get() = scroll;
        (kit.send_two)(root, kit.sel("addSubview:"), scroll);
        let (safe, by) = (kit.send(root, "safeAreaLayoutGuide"), msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id));
        for edge in ["topAnchor", "leadingAnchor", "trailingAnchor", "bottomAnchor"] {
            // The top goes under the navigation bar (large titles collapse by scrolling), the content is inset by it.
            let to = if edge == "topAnchor" { root } else { safe };
            let c = by(kit.send(scroll, edge), kit.sel("constraintEqualToAnchor:constant:"), kit.send(to, edge), 0.0);
            // The bottom gives way to a tab bar (required `<=` below it).
            if edge == "bottomAnchor" {
                msg!(kit, extern "C" fn(Id, Id, f32))(c, kit.sel("setPriority:"), 750.0);
            }
            (kit.send_id)(c, kit.sel("setActive:"), 1);
        }
        kit.send(w, "makeKeyAndVisible");
        *WINDOW.get() = w;
        *SCREEN.get() = Some(Screen { menus: Vec::new(), kit, target, root, scroll, bar: core::ptr::null_mut(), controller, content: core::ptr::null_mut(), pushed: core::ptr::null_mut(), page_scroll: core::ptr::null_mut(), page_content: core::ptr::null_mut(), back: 0, shape: String::new(), made: Vec::new(), clocks: Vec::new(), timers: Vec::new() });
    }
    // The program: a thread of its own, which never comes back here.
    extern "C" fn program(_: i64) {
        crate::run()
    }
    let _ = super::spawn(program, 0);
    true
}

fn screen() -> &'static Screen {
    SCREEN.get().as_ref().expect("made with the window")
}

fn tag_of(sender: Id) -> usize {
    let kit = &screen().kit;
    (kit.send_int)(sender, kit.sel("tag")) as usize
}

/// `yyyy-MM-dd`, as the date pickers and `ui.date` write dates.
fn date_formatter(kit: &Kit) -> Id {
    let f = kit.send(kit.send(kit.class("NSDateFormatter"), "alloc"), "init");
    (kit.send_two)(f, kit.sel("setDateFormat:"), kit.text("yyyy-MM-dd"));
    f
}

fn picked_date(kit: &Kit, picker: Id) -> String {
    text_of(kit, (kit.send_two)(date_formatter(kit), kit.sel("stringFromDate:"), kit.send(picker, "date")))
}

extern "C" fn stepped(_me: Id, _message: Id, sender: Id) {
    let kit = &screen().kit;
    let value = msg!(kit, extern "C" fn(Id, Id) -> f64)(sender, kit.sel("value"));
    hear(crate::shared::ui_changed(tag_of(sender)), alloc::format!("{}", value as i64));
}

extern "C" fn dated(_me: Id, _message: Id, picker: Id) {
    let kit = &screen().kit;
    hear(crate::shared::ui_changed(tag_of(picker)), picked_date(kit, picker));
}

/// A tab of a tab bar was chosen: its tag is the number of its button.
extern "C" fn tab_chosen(_me: Id, _message: Id, _bar: Id, item: Id) {
    let tag = (screen().kit.send_int)(item, screen().kit.sel("tag"));
    if tag >= 0 {
        hear(tag, String::new());
    }
}

/// A page was shown by the navigation controller: when it is the window's own again while the program still gives a
/// pushed page, the person went back (button or swipe): the program is told.
extern "C" fn page_shown(_me: Id, _message: Id, _nav: Id, shown: Id, _animated: u8) {
    let Some(screen) = SCREEN.get().as_mut() else { return };
    if shown == screen.controller && !screen.pushed.is_null() {
        screen.pushed = core::ptr::null_mut();
        screen.page_content = core::ptr::null_mut();
        screen.page_scroll = core::ptr::null_mut();
        // The shape no longer holds the page: the next view is built afresh.
        screen.shape = String::new();
        hear(screen.back as i64, String::new());
    }
}

extern "C" fn tapped(_me: Id, _message: Id, sender: Id) {
    hear(tag_of(sender) as i64, String::new());
}

fn text_of(kit: &Kit, text: Id) -> String {
    if text.is_null() {
        return String::new();
    }
    let bytes = (kit.send)(text, kit.sel("UTF8String")) as *const u8;
    String::from_utf8_lossy(unsafe { crate::c_text(bytes) }).into_owned()
}

extern "C" fn edited(_me: Id, _message: Id, field: Id) {
    let kit = &screen().kit;
    hear(crate::shared::ui_changed(tag_of(field)), text_of(kit, kit.send(field, "text")));
}

extern "C" fn switched(_me: Id, _message: Id, sender: Id) {
    let kit = &screen().kit;
    let on = (kit.send_int)(sender, kit.sel("isOn")) & 0xff != 0;
    hear(crate::shared::ui_changed(tag_of(sender)), (if on { "on" } else { "off" }).into());
}

extern "C" fn slid(_me: Id, _message: Id, sender: Id) {
    let kit = &screen().kit;
    let value = msg!(kit, extern "C" fn(Id, Id) -> f32)(sender, kit.sel("value"));
    hear(crate::shared::ui_changed(tag_of(sender)), alloc::format!("{}", (value + if value < 0.0 { -0.5 } else { 0.5 }) as i64));
}

extern "C" fn chosen(_me: Id, _message: Id, sender: Id) {
    let kit = &screen().kit;
    hear(crate::shared::ui_changed(tag_of(sender)), alloc::format!("{}", (kit.send_int)(sender, kit.sel("selectedSegmentIndex"))));
}

/// A row of a list: the button's tag is the list's number times 100000, plus the row.
extern "C" fn rowed(_me: Id, _message: Id, sender: Id) {
    let tag = tag_of(sender);
    hear(crate::shared::ui_changed(tag / 100000), alloc::format!("{}", tag % 100000));
}

extern "C" fn tick(_me: Id, _message: Id, timer: Id) {
    let kit = &screen().kit;
    hear((kit.send_int)(kit.send(timer, "userInfo"), kit.sel("longValue")), String::new());
}

/// The return key puts the keyboard away.
extern "C" fn returned(_me: Id, _message: Id, field: Id) -> bool {
    let kit = &screen().kit;
    kit.send(field, "resignFirstResponder");
    true
}

/// What `ui.show` does on iOS: hands `view` to the main thread, then waits for what the user does.
pub fn show(title: &str, view: &UiNode, menus: &[crate::shared::UiMenu], _width: i32, _height: i32, acts: &[i64], typed: &mut String) -> i64 {
    // `ui.close`: the app ends.
    if acts.contains(&1) {
        return -1;
    }
    struct Job<'a> {
        title: &'a str,
        view: &'a UiNode,
        menus: &'a [crate::shared::UiMenu],
    }
    extern "C" fn draw(job: *mut u8) {
        let job = unsafe { &*(job as *const Job) };
        draw_view(job.title, job.view, job.menus);
    }
    let system = library("/usr/lib/libSystem.B.dylib").expect("libSystem");
    let sync: extern "C" fn(usize, *mut u8, extern "C" fn(*mut u8)) = unsafe { core::mem::transmute(symbol(system, "dispatch_sync_f").expect("dispatch_sync_f")) };
    let main_queue = symbol(system, "_dispatch_main_q").expect("the main queue");
    let job = Job { title, view, menus };
    sync(main_queue, &job as *const Job as *mut u8, draw);
    loop {
        let seen = RING.load(SeqCst);
        if let Some((answer, text)) = HEARD.with(|q| q.next()) {
            *typed = text;
            return answer;
        }
        super::wait(&RING, seen);
    }
}

/// On the main thread: the window shows `view`.
fn draw_view(title: &str, view: &UiNode, menus: &[crate::shared::UiMenu]) {
    let Some(screen) = SCREEN.get().as_mut() else { return };
    let kit = &screen.kit;
    let pool = (kit.push)();
    let shape = crate::shared::ui_shape(view);
    // A page pushed over the window's view: the window's own part is built apart from it.
    let (view, page) = match view {
        UiNode::Pushed(main, page, back) => (&**main, Some((&**page, *back))),
        other => (other, None),
    };
    let whole = crate::shared::ui_leaves_of(view, page.map(|p| p.0));
    // A view may give its page a title of its own (`.title("Stats")`), else the window's.
    let title = crate::shared::ui_title(view).unwrap_or_else(|| title.into());
    (kit.send_two)(kit.send(screen.controller, "navigationItem"), kit.sel("setTitle:"), kit.text(&title));
    // `.background(ui.page)` on the whole view paints the window, as grouped lists have it.
    let window_color = match crate::shared::ui_backdrop(view) {
        Some(c) => ui_color(kit, c),
        None => kit.send(kit.class("UIColor"), "systemBackgroundColor"),
    };
    (kit.send_two)(screen.root, kit.sel("setBackgroundColor:"), window_color);
    if shape == screen.shape && menus == screen.menus.as_slice() {
        for ((leaf, _), &(v, label)) in whole.iter().zip(screen.made.iter()) {
            refresh(kit, leaf, v, label);
        }
    } else {
        if !screen.content.is_null() {
            kit.send(screen.content, "removeFromSuperview");
        }
        if !screen.bar.is_null() {
            kit.send(screen.bar, "removeFromSuperview");
            screen.bar = core::ptr::null_mut();
        }
        if !screen.page_content.is_null() {
            kit.send(screen.page_content, "removeFromSuperview");
            screen.page_content = core::ptr::null_mut();
        }
        screen.made.clear();
        CANVASES.get().clear();
        CONTEXTS.get().clear();
        let content = build(kit, screen.target, view, &mut screen.made);
        let content = if kit.class("UIStackView") == kit.send(content, "class") { content } else { wrapped(kit, content) };
        // The menus go in the navigation bar, each a button opening its items.
        let item = kit.send(screen.controller, "navigationItem");
        let buttons: Vec<Id> = menus.iter().rev().map(|m| bar_button(kit, m)).collect();
        let array = msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), buttons.as_ptr(), buttons.len() as u64);
        let own = crate::shared::ui_bar_buttons(view);
        (kit.send_two)(item, kit.sel("setRightBarButtonItems:"), if own.is_empty() { array } else { nav_items(kit, screen.target, &own) });
        place(kit, screen.scroll, content, crate::shared::ui_fills(view));
        // A tab bar goes to the bottom of the screen, under the scrolled content.
        let tab_bar = screen.made.iter().map(|&(v, _)| v).find(|&v| kit.class("UITabBar") == kit.send(v, "class"));
        if let Some(bar) = tab_bar {
            kit.send(bar, "removeFromSuperview");
            (kit.send_two)(screen.root, kit.sel("addSubview:"), bar);
            let (safe, by) = (kit.send(screen.root, "safeAreaLayoutGuide"), msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id));
            for edge in ["leadingAnchor", "trailingAnchor", "bottomAnchor"] {
                (kit.send_id)(by(kit.send(bar, edge), kit.sel("constraintEqualToAnchor:constant:"), kit.send(safe, edge), 0.0), kit.sel("setActive:"), 1);
            }
            let tall = (kit.send_float)(kit.send(bar, "heightAnchor"), kit.sel("constraintEqualToConstant:"), 76.0);
            (kit.send_id)(tall, kit.sel("setActive:"), 1);
            let above = (kit.send_two)(kit.send(screen.scroll, "bottomAnchor"), kit.sel("constraintLessThanOrEqualToAnchor:"), kit.send(bar, "topAnchor"));
            (kit.send_id)(above, kit.sel("setActive:"), 1);
            // Its items are drawn only once it is in the window: give them again.
            let (items, now) = (kit.send(bar, "items"), kit.send(bar, "selectedItem"));
            msg!(kit, extern "C" fn(Id, Id, Id, bool))(bar, kit.sel("setItems:animated:"), items, false);
            (kit.send_two)(bar, kit.sel("setSelectedItem:"), now);
            screen.bar = bar;
        }
        screen.content = content;
        // The pushed page: a screen of its own over the window's, with a back button.
        if let Some((page, back)) = page {
            let content = build(kit, screen.target, page, &mut screen.made);
            let content = if kit.class("UIStackView") == kit.send(content, "class") { content } else { wrapped(kit, content) };
            if screen.pushed.is_null() {
                let controller = kit.send(kit.send(kit.class("UIViewController"), "alloc"), "init");
                let root = kit.send(controller, "view");
                (kit.send_two)(root, kit.sel("setBackgroundColor:"), window_color);
                let scroll = made(kit, "UIScrollView");
                (kit.send_id)(scroll, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
                (kit.send_id)(scroll, kit.sel("setKeyboardDismissMode:"), 1);
                (kit.send_two)(root, kit.sel("addSubview:"), scroll);
                let (safe, by) = (kit.send(root, "safeAreaLayoutGuide"), msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id));
                for edge in ["topAnchor", "leadingAnchor", "trailingAnchor", "bottomAnchor"] {
                    let to = if edge == "topAnchor" { root } else { safe };
                    (kit.send_id)(by(kit.send(scroll, edge), kit.sel("constraintEqualToAnchor:constant:"), kit.send(to, edge), 0.0), kit.sel("setActive:"), 1);
                }
                // The title stays small on a detail page.
                (kit.send_id)(kit.send(controller, "navigationItem"), kit.sel("setLargeTitleDisplayMode:"), 2);
                screen.pushed = controller;
                screen.page_scroll = scroll;
                msg!(kit, extern "C" fn(Id, Id, Id, bool))(kit.send(screen.controller, "navigationController"), kit.sel("pushViewController:animated:"), controller, true);
            }
            place(kit, screen.page_scroll, content, crate::shared::ui_fills(page));
            screen.page_content = content;
            screen.back = back;
        }
        screen.shape = shape;
        screen.menus = menus.to_vec();
    }
    // The pushed page's title, and the page going away when the program stops giving one.
    match page {
        Some((page, _)) => {
            let name = crate::shared::ui_title(page).unwrap_or_default();
            (kit.send_two)(kit.send(screen.pushed, "navigationItem"), kit.sel("setTitle:"), kit.text(&name));
            let own = crate::shared::ui_bar_buttons(page);
            (kit.send_two)(kit.send(screen.pushed, "navigationItem"), kit.sel("setRightBarButtonItems:"), nav_items(kit, screen.target, &own));
            (kit.send_id)(kit.send(screen.pushed, "navigationItem"), kit.sel("setLargeTitleDisplayMode:"), 2);
        }
        None if !screen.pushed.is_null() => {
            let nav = kit.send(screen.controller, "navigationController");
            screen.pushed = core::ptr::null_mut();
            screen.page_scroll = core::ptr::null_mut();
            screen.page_content = core::ptr::null_mut();
            (kit.send_id)(nav, kit.sel("popViewControllerAnimated:"), 1);
        }
        None => {}
    }
    for ((leaf, style), &(v, label)) in whole.iter().zip(screen.made.iter()) {
        styled(kit, v, label, leaf, *style);
    }
    let clocks = crate::shared::ui_clocks(view);
    if clocks != screen.clocks {
        for timer in screen.timers.drain(..) {
            kit.send(timer, "invalidate");
        }
        for &(ms, n) in &clocks {
            let timer = msg!(kit, extern "C" fn(Id, Id, f64, Id, Id, Id, bool) -> Id)(
                kit.class("NSTimer"),
                kit.sel("scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:"),
                ms as f64 / 1000.0,
                screen.target,
                kit.sel("tick:"),
                (kit.send_id)(kit.class("NSNumber"), kit.sel("numberWithLong:"), n as i64),
                true,
            );
            screen.timers.push(timer);
        }
        screen.clocks = clocks;
    }
    // The canvases that fill: the program is told how much room they have.
    kit.send(screen.root, "layoutIfNeeded");
    for (view, canvas) in CANVASES.get().iter() {
        let bounds = msg!(kit, extern "C" fn(Id, Id) -> Rect)(*view, kit.sel("bounds"));
        if (canvas.width == 0 || canvas.height == 0) && crate::ui_room(canvas.number, bounds.width as i32, bounds.height as i32) {
            hear(crate::shared::UI_RESIZED, String::new());
        }
        if canvas.gpu() {
            gpu_paint(kit, *view, canvas, bounds);
        }
    }
    // `KOVA_UI_TAP=0,0,1`: for a test, the app taps those buttons itself, one for each time the
    // view is drawn (`simctl` cannot touch the screen).
    static TAPS: crate::Global<Option<Vec<usize>>> = crate::Global::new(None);
    let taps = TAPS.get().get_or_insert_with(|| super::var("KOVA_UI_TAP").map_or(Vec::new(), |v| v.split(',').filter_map(|n| n.parse().ok()).rev().collect()));
    if let Some(n) = taps.pop() {
        for ((leaf, _), &(v, _)) in crate::shared::ui_leaves(view).into_iter().zip(screen.made.iter()) {
            if matches!(leaf, UiNode::Button(_, k) if *k == n) {
                (kit.send_id)(v, kit.sel("sendActionsForControlEvents:"), 64);
                break;
            }
        }
    }
    (kit.pop)(pool);
}

/// A stack view of one part.
fn wrapped(kit: &Kit, part: Id) -> Id {
    let stack = made(kit, "UIStackView");
    (kit.send_id)(stack, kit.sel("setAxis:"), 1);
    (kit.send_id)(stack, kit.sel("setAlignment:"), 1);
    (kit.send_two)(stack, kit.sel("addArrangedSubview:"), part);
    stack
}

/// `content` in `scroll`, 16 points from its edges: at the top left, or across the width when
/// something in it fills. The scrolled area is as wide as the screen and as tall as the content.
fn place(kit: &Kit, scroll: Id, content: Id, fills: bool) {
    (kit.send_id)(content, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
    (kit.send_two)(scroll, kit.sel("addSubview:"), content);
    let (area, frame) = (kit.send(scroll, "contentLayoutGuide"), kit.send(scroll, "frameLayoutGuide"));
    let by = msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id);
    let on = |constraint: Id| {
        (kit.send_id)(constraint, kit.sel("setActive:"), 1);
    };
    on(by(kit.send(content, "topAnchor"), kit.sel("constraintEqualToAnchor:constant:"), kit.send(area, "topAnchor"), 16.0));
    on(by(kit.send(content, "bottomAnchor"), kit.sel("constraintEqualToAnchor:constant:"), kit.send(area, "bottomAnchor"), -16.0));
    on(by(kit.send(content, "leadingAnchor"), kit.sel("constraintEqualToAnchor:constant:"), kit.send(area, "leadingAnchor"), 16.0));
    // As wide as the screen, or wider when something fixed (a canvas) is: then it scrolls sideways.
    on(by(kit.send(area, "widthAnchor"), kit.sel("constraintGreaterThanOrEqualToAnchor:constant:"), kit.send(frame, "widthAnchor"), 0.0));
    let end = if fills { "constraintEqualToAnchor:constant:" } else { "constraintLessThanOrEqualToAnchor:constant:" };
    on(by(kit.send(content, "trailingAnchor"), kit.sel(end), kit.send(area, "trailingAnchor"), -16.0));
}

/// A width of at least `points`.
/// A color of a view: red, green and blue in the low 24 bits, or one of the system's (`ui.panel`, `ui.page`, `ui.card`) past them.
fn ui_color(kit: &Kit, c: u32) -> Id {
    if c > 0xff_ffff {
        return kit.send(kit.class("UIColor"), match c & 0xff {
            2 => "systemGroupedBackgroundColor",
            3 => "secondarySystemGroupedBackgroundColor",
            _ => "secondarySystemBackgroundColor",
        });
    }
    let channel = |shift: u32| ((c >> shift) & 0xff) as f64 / 255.0;
    msg!(kit, extern "C" fn(Id, Id, f64, f64, f64, f64) -> Id)(kit.class("UIColor"), kit.sel("colorWithRed:green:blue:alpha:"), channel(16), channel(8), channel(0), 1.0)
}

/// A list: its rows in a rounded white card as the Settings app has them, a chevron on each; a line break in a
/// row (a tab) gives a title, a subtitle and a detail. Each row is a button laid over its labels.
fn grouped_rows(kit: &Kit, target: Id, table: &crate::shared::UiTable, made_views: &mut Vec<(Id, Id)>) -> Id {
    let card = made(kit, "UIStackView");
    (kit.send_id)(card, kit.sel("setAxis:"), 1);
    (kit.send_id)(card, kit.sel("setAlignment:"), 0);
    (kit.send_two)(card, kit.sel("setBackgroundColor:"), ui_color(kit, 0x100_0003));
    let layer = kit.send(card, "layer");
    (kit.send_float)(layer, kit.sel("setCornerRadius:"), 24.0);
    (kit.send_id)(layer, kit.sel("setMasksToBounds:"), 1);
    let screen_width = msg!(kit, extern "C" fn(Id, Id) -> Rect)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("bounds")).width;
    at_least(kit, card, screen_width - 32.0);
    let (by, inset) = (msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id), msg!(kit, extern "C" fn(Id, Id, Insets)));
    for (k, row) in table.rows.iter().enumerate() {
        if k > 0 {
            // A hairline from the text's edge to the card's.
            let line = made(kit, "UIView");
            (kit.send_two)(line, kit.sel("setBackgroundColor:"), kit.send(kit.class("UIColor"), "separatorColor"));
            (kit.send_id)((kit.send_float)(kit.send(line, "heightAnchor"), kit.sel("constraintEqualToConstant:"), 0.5), kit.sel("setActive:"), 1);
            let wrap = wrapped(kit, line);
            (kit.send_id)(wrap, kit.sel("setAlignment:"), 0);
            inset(wrap, kit.sel("setLayoutMargins:"), Insets { top: 0.0, left: 16.0, bottom: 0.0, right: 0.0 });
            (kit.send_id)(wrap, kit.sel("setLayoutMarginsRelativeArrangement:"), 1);
            (kit.send_two)(card, kit.sel("addArrangedSubview:"), wrap);
        }
        let labels = made(kit, "UIStackView");
        (kit.send_id)(labels, kit.sel("setAxis:"), 1);
        (kit.send_float)(labels, kit.sel("setSpacing:"), 2.0);
        for (j, text) in row.join(" ").split('\t').enumerate() {
            let label = made(kit, "UILabel");
            (kit.send_two)(label, kit.sel("setText:"), kit.text(text));
            let size = [17.0, 15.0, 12.0][j.min(2)];
            let font = (kit.send_float)(kit.class("UIFont"), kit.sel(if j == 0 { "boldSystemFontOfSize:" } else { "systemFontOfSize:" }), size);
            (kit.send_two)(label, kit.sel("setFont:"), font);
            if j > 0 {
                (kit.send_two)(label, kit.sel("setTextColor:"), kit.send(kit.class("UIColor"), "secondaryLabelColor"));
            }
            (kit.send_id)(label, kit.sel("setNumberOfLines:"), 1);
            (kit.send_two)(labels, kit.sel("addArrangedSubview:"), label);
        }
        let chevron = (kit.send_two)(kit.send(kit.class("UIImageView"), "alloc"), kit.sel("initWithImage:"), (kit.send_two)(kit.class("UIImage"), kit.sel("systemImageNamed:"), kit.text("chevron.right")));
        let chevron = kit.send(chevron, "autorelease");
        (kit.send_two)(chevron, kit.sel("setTintColor:"), kit.send(kit.class("UIColor"), "tertiaryLabelColor"));
        (kit.send_id)(chevron, kit.sel("setContentMode:"), 1); // scale to fit
        (kit.send_id)(chevron, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
        (kit.send_id)((kit.send_float)(kit.send(chevron, "widthAnchor"), kit.sel("constraintEqualToConstant:"), 14.0), kit.sel("setActive:"), 1);
        let line = made(kit, "UIStackView");
        (kit.send_id)(line, kit.sel("setAlignment:"), 3);
        (kit.send_float)(line, kit.sel("setSpacing:"), 8.0);
        (kit.send_two)(line, kit.sel("addArrangedSubview:"), labels);
        (kit.send_two)(line, kit.sel("addArrangedSubview:"), chevron);
        inset(line, kit.sel("setLayoutMargins:"), Insets { top: 12.0, left: 16.0, bottom: 12.0, right: 16.0 });
        (kit.send_id)(line, kit.sel("setLayoutMarginsRelativeArrangement:"), 1);
        // The button over the whole row.
        let b = (kit.send_id)(kit.class("UIButton"), kit.sel("buttonWithType:"), 0);
        (kit.send_id)(b, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
        (kit.send_id)(b, kit.sel("setTag:"), (table.number * 100000 + k) as i64);
        msg!(kit, extern "C" fn(Id, Id, Id, Id, i64) -> Id)(b, kit.sel("addTarget:action:forControlEvents:"), target, kit.sel("rowed:"), 64);
        (kit.send_two)(line, kit.sel("addSubview:"), b);
        for edge in ["topAnchor", "bottomAnchor", "leadingAnchor", "trailingAnchor"] {
            (kit.send_id)(by(kit.send(b, edge), kit.sel("constraintEqualToAnchor:constant:"), kit.send(line, edge), 0.0), kit.sel("setActive:"), 1);
        }
        (kit.send_two)(card, kit.sel("addArrangedSubview:"), line);
    }
    // The rows are not leaves: the list as a whole is one, nothing to refresh in place.
    made_views.push((card, core::ptr::null_mut()));
    card
}

fn at_least(kit: &Kit, view: Id, points: f64) {
    let wide = (kit.send_float)(kit.send(view, "widthAnchor"), kit.sel("constraintGreaterThanOrEqualToConstant:"), points);
    (kit.send_id)(wide, kit.sel("setActive:"), 1);
}

/// What is typed on a view: its number and what to do when it changes (`UIControlEvent`:
/// touch up inside 64, value changed 4096, editing changed 131072).
fn hears(kit: &Kit, target: Id, view: Id, number: usize, action: &str, events: i64) {
    (kit.send_id)(view, kit.sel("setTag:"), number as i64);
    msg!(kit, extern "C" fn(Id, Id, Id, Id, i64) -> Id)(view, kit.sel("addTarget:action:forControlEvents:"), target, kit.sel(action), events);
}

fn build(kit: &Kit, target: Id, node: &UiNode, made_views: &mut Vec<(Id, Id)>) -> Id {
    let leaf = match node {
        UiNode::Text(s) => {
            let label = made(kit, "UILabel");
            (kit.send_two)(label, kit.sel("setText:"), kit.text(s));
            (kit.send_id)(label, kit.sel("setNumberOfLines:"), 0);
            // The text stays within the screen: a long word (a path) breaks where it must, not at the edge.
            // `.no_wrap` leaves it alone.
            if !core::mem::take(WIDE.get()) {
                let width = msg!(kit, extern "C" fn(Id, Id) -> Rect)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("bounds")).width;
                let cap = (kit.send_float)(kit.send(label, "widthAnchor"), kit.sel("constraintLessThanOrEqualToConstant:"), width - 32.0);
                (kit.send_id)(cap, kit.sel("setActive:"), 1);
            }
            label
        }
        UiNode::Button(s, n) => {
            let b = (kit.send_id)(kit.class("UIButton"), kit.sel("buttonWithType:"), 1);
            msg!(kit, extern "C" fn(Id, Id, Id, i64) -> Id)(b, kit.sel("setTitle:forState:"), kit.text(s), 0);
            hears(kit, target, b, *n, "tapped:", 64);
            // Left-aligned, as a button in a form is.
            (kit.send_id)(b, kit.sel("setContentHorizontalAlignment:"), 1);
            b
        }
        UiNode::Input(s, n) | UiNode::Search(s, n) => {
            let field = made(kit, if matches!(node, UiNode::Search(..)) { "UISearchTextField" } else { "UITextField" });
            (kit.send_two)(field, kit.sel("setText:"), kit.text(s));
            // No frame, as in the system's forms: a search field has its own look.
            if !matches!(node, UiNode::Search(..)) {
                (kit.send_id)(field, kit.sel("setBorderStyle:"), 0);
            }
            (kit.send_two)(field, kit.sel("setDelegate:"), target);
            hears(kit, target, field, *n, "edited:", 131072);
            // A search field takes the screen's width, as on iOS.
            let width = msg!(kit, extern "C" fn(Id, Id) -> Rect)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("bounds")).width;
            at_least(kit, field, if matches!(node, UiNode::Search(..)) { width - 32.0 } else { 200.0 });
            field
        }
        UiNode::Check(s, on, n) => {
            let stack = made(kit, "UIStackView");
            (kit.send_id)(stack, kit.sel("setAlignment:"), 3);
            (kit.send_float)(stack, kit.sel("setSpacing:"), 8.0);
            let label = made(kit, "UILabel");
            (kit.send_two)(label, kit.sel("setText:"), kit.text(s));
            let switch = made(kit, "UISwitch");
            (kit.send_id)(switch, kit.sel("setOn:"), *on as i64);
            hears(kit, target, switch, *n, "switched:", 4096);
            (kit.send_two)(stack, kit.sel("addArrangedSubview:"), label);
            (kit.send_two)(stack, kit.sel("addArrangedSubview:"), switch);
            made_views.push((switch, label));
            return stack;
        }
        UiNode::Date(text, n) => {
            let picker = made(kit, "UIDatePicker");
            // UIDatePickerMode date = 1, UIDatePickerStyle compact = 2.
            (kit.send_id)(picker, kit.sel("setDatePickerMode:"), 1);
            (kit.send_id)(picker, kit.sel("setPreferredDatePickerStyle:"), 2);
            let date = (kit.send_two)(date_formatter(kit), kit.sel("dateFromString:"), kit.text(text));
            if !date.is_null() {
                (kit.send_two)(picker, kit.sel("setDate:"), date);
            }
            hears(kit, target, picker, *n, "dated:", 4096);
            picker
        }
        UiNode::Stepper(v, min, max, n) => {
            let s = made(kit, "UIStepper");
            let set = msg!(kit, extern "C" fn(Id, Id, f64) -> Id);
            set(s, kit.sel("setMinimumValue:"), *min as f64);
            set(s, kit.sel("setMaximumValue:"), *max as f64);
            set(s, kit.sel("setValue:"), *v as f64);
            hears(kit, target, s, *n, "stepped:", 4096);
            s
        }
        UiNode::Native(id, width, height) => {
            // A box of the asked size, with the view pinned inside: the view may sit in only one place, so each
            // build makes a new box and takes it over, and the view keeps its own constraints.
            let (inner, by) = (*id as Id, msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id));
            let boxed = made(kit, "UIView");
            (kit.send_id)(boxed, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
            (kit.send_id)(inner, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
            (kit.send_two)(boxed, kit.sel("addSubview:"), inner);
            for edge in ["topAnchor", "bottomAnchor", "leadingAnchor", "trailingAnchor"] {
                (kit.send_id)(by(kit.send(inner, edge), kit.sel("constraintEqualToAnchor:constant:"), kit.send(boxed, edge), 0.0), kit.sel("setActive:"), 1);
            }
            for (anchor, length) in [("widthAnchor", *width), ("heightAnchor", *height)] {
                (kit.send_id)((kit.send_float)(kit.send(boxed, anchor), kit.sel("constraintEqualToConstant:"), length as f64), kit.sel("setActive:"), 1);
            }
            boxed
        }
        UiNode::Spinner => {
            let wheel = (kit.send_id)(kit.send(kit.class("UIActivityIndicatorView"), "alloc"), kit.sel("initWithActivityIndicatorStyle:"), 100);
            kit.send(wheel, "startAnimating");
            wheel
        }
        UiNode::Progress(v, most) => {
            let bar = (kit.send_id)(kit.class("UIProgressView"), kit.sel("alloc"), 0);
            let bar = (kit.send_id)(bar, kit.sel("initWithProgressViewStyle:"), 0);
            (kit.send_id)(bar, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
            msg!(kit, extern "C" fn(Id, Id, f32) -> Id)(bar, kit.sel("setProgress:"), *v as f32 / *most as f32);
            at_least(kit, bar, 200.0);
            bar
        }
        UiNode::Slider(v, min, max, n) => {
            let s = made(kit, "UISlider");
            let set = msg!(kit, extern "C" fn(Id, Id, f32) -> Id);
            set(s, kit.sel("setMinimumValue:"), *min as f32);
            set(s, kit.sel("setMaximumValue:"), *max as f32);
            set(s, kit.sel("setValue:"), *v as f32);
            hears(kit, target, s, *n, "slid:", 4096);
            at_least(kit, s, 200.0);
            s
        }
        UiNode::Choice(items, k, n) => {
            let list = (kit.send)(kit.class("NSMutableArray"), kit.sel("array"));
            for item in items {
                (kit.send_two)(list, kit.sel("addObject:"), kit.text(item));
            }
            let c = (kit.send_two)(kit.send(kit.class("UISegmentedControl"), "alloc"), kit.sel("initWithItems:"), list);
            let c = kit.send(c, "autorelease");
            (kit.send_id)(c, kit.sel("setSelectedSegmentIndex:"), *k as i64);
            hears(kit, target, c, *n, "chosen:", 4096);
            c
        }
        // Rows of buttons: the one picked starts with an arrow.
        UiNode::Table(table) if table.headers.is_empty() => return grouped_rows(kit, target, table, made_views),
        UiNode::Table(table) => {
            let stack = made(kit, "UIStackView");
            (kit.send_id)(stack, kit.sel("setAxis:"), 1);
            (kit.send_id)(stack, kit.sel("setAlignment:"), 1);
            // The headers: a line of bold text above the rows, not a button.
            if !table.headers.is_empty() {
                let head = made(kit, "UILabel");
                (kit.send_two)(head, kit.sel("setText:"), kit.text(&table.headers.join("  ")));
                let font = (kit.send_float)(kit.class("UIFont"), kit.sel("boldSystemFontOfSize:"), 17.0);
                (kit.send_two)(head, kit.sel("setFont:"), font);
                (kit.send_two)(stack, kit.sel("addArrangedSubview:"), head);
            }
            for (k, row) in table.rows.iter().enumerate() {
                let b = (kit.send_id)(kit.class("UIButton"), kit.sel("buttonWithType:"), 1);
                let line = row.join("  ");
                msg!(kit, extern "C" fn(Id, Id, Id, i64) -> Id)(b, kit.sel("setTitle:forState:"), kit.text(&alloc::format!("{}{line}", if table.selected == k as i32 { "▸ " } else { "" })), 0);
                (kit.send_id)(b, kit.sel("setTag:"), (table.number * 100000 + k) as i64);
                msg!(kit, extern "C" fn(Id, Id, Id, Id, i64) -> Id)(b, kit.sel("addTarget:action:forControlEvents:"), target, kit.sel("rowed:"), 64);
                (kit.send_two)(stack, kit.sel("addArrangedSubview:"), b);
            }
            // The rows are not leaves: the list as a whole is one, nothing to refresh in place.
            made_views.push((stack, core::ptr::null_mut()));
            return stack;
        }
        // `ui.tabs`: a tab bar. The tab in use is a text (tag -1), each other a button (its number).
        UiNode::Tabs(parts, icons) => {
            let bar = made(kit, "UITabBar");
            let mut items = Vec::new();
            let mut now = core::ptr::null_mut();
            for (k, p) in parts.iter().enumerate() {
                let (name, tag) = match p {
                    UiNode::Text(t) => (t, -1),
                    UiNode::Button(t, n) => (t, *n as i64),
                    _ => continue,
                };
                // An SF Symbol, when the program gave names for them.
                let image = match icons.get(k) {
                    Some(symbol) => (kit.send_two)(kit.class("UIImage"), kit.sel("systemImageNamed:"), kit.text(symbol)),
                    None => core::ptr::null_mut(),
                };
                let item = msg!(kit, extern "C" fn(Id, Id, Id, Id, i64) -> Id)(kit.send(kit.class("UITabBarItem"), "alloc"), kit.sel("initWithTitle:image:tag:"), kit.text(name), image, tag);
                if tag < 0 {
                    now = item;
                }
                items.push(item);
            }
            let array = msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), items.as_ptr(), items.len() as u64);
            msg!(kit, extern "C" fn(Id, Id, Id, bool))(bar, kit.sel("setItems:animated:"), array, false);
            (kit.send_two)(bar, kit.sel("setSelectedItem:"), now);
            (kit.send_two)(bar, kit.sel("setDelegate:"), target);
            // In a column aligned to the left a bar has no width of its own: it takes the screen's, less the margins.
            let screen_width = msg!(kit, extern "C" fn(Id, Id) -> Rect)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("bounds")).width;
            at_least(kit, bar, screen_width - 32.0);
            made_views.push((bar, core::ptr::null_mut()));
            return bar;
        }
        UiNode::Column(parts, a) | UiNode::Row(parts, a) => {
            let stack = made(kit, "UIStackView");
            let column = matches!(node, UiNode::Column(..));
            (kit.send_id)(stack, kit.sel("setAxis:"), column as i64);
            // UIStackViewAlignment: fill 0, leading or top 1, center 3, trailing or bottom 4.
            (kit.send_id)(stack, kit.sel("setAlignment:"), match a {
                UiAlign::Start => 1,
                UiAlign::Center => 3,
                UiAlign::End => 4,
            });
            (kit.send_float)(stack, kit.sel("setSpacing:"), 8.0);
            for p in parts {
                let part = build(kit, target, p, made_views);
                (kit.send_two)(stack, kit.sel("addArrangedSubview:"), part);
            }
            return stack;
        }
        UiNode::Image(path, width) => {
            let image = (kit.send_two)(kit.class("UIImage"), kit.sel("imageWithContentsOfFile:"), kit.text(path));
            if image.is_null() {
                made(kit, "UILabel")
            } else {
                let v = kit.send((kit.send_two)(kit.send(kit.class("UIImageView"), "alloc"), kit.sel("initWithImage:"), image), "autorelease");
                if *width > 0 {
                    let natural = msg!(kit, extern "C" fn(Id, Id) -> Size)(image, kit.sel("size"));
                    let width = *width as f64;
                    (kit.send_id)(v, kit.sel("setContentMode:"), 1); // scale to fit
                    for (anchor, length) in [("widthAnchor", width), ("heightAnchor", width * natural.height / natural.width.max(1.0))] {
                        let fixed = (kit.send_float)(kit.send(v, anchor), kit.sel("constraintEqualToConstant:"), length);
                        (kit.send_id)(fixed, kit.sel("setActive:"), 1);
                    }
                }
                v
            }
        }
        UiNode::Styled(style, inner) => {
            *WIDE.get() = style.wide && matches!(**inner, UiNode::Text(_));
            return build(kit, target, inner, made_views);
        }
        UiNode::Every(_, _, inner) => return build(kit, target, inner, made_views),
        UiNode::Titled(_, inner) | UiNode::Barred(inner, _) => return build(kit, target, inner, made_views),
        // Only at the top of a view does a page get its screen (`draw_view`).
        UiNode::Pushed(main, _, _) => return build(kit, target, main, made_views),
        UiNode::Context(inner, items) => {
            let view = build(kit, target, inner, made_views);
            let interaction = (kit.send_two)(kit.send(kit.class("UIContextMenuInteraction"), "alloc"), kit.sel("initWithDelegate:"), target);
            // A label ignores touches until it is told not to.
            (kit.send_id)(view, kit.sel("setUserInteractionEnabled:"), 1);
            (kit.send_two)(view, kit.sel("addInteraction:"), interaction);
            CONTEXTS.get().push((view, items.clone()));
            return view;
        }
        // A color behind a view (and its room), corners rounded: a stack view of one part, across the width.
        UiNode::Boxed(c, radius, inner) => {
            let built = build(kit, target, inner, made_views);
            // What is in a card spreads across it: a switch ends at its right edge, as in a form.
            let mut level = built;
            while kit.class("UIStackView") == kit.send(level, "class") {
                (kit.send_id)(level, kit.sel("setAlignment:"), 0);
                let kids = kit.send(level, "arrangedSubviews");
                if (kit.send_int)(kids, kit.sel("count")) != 1 {
                    break;
                }
                level = (kit.send_id)(kids, kit.sel("objectAtIndex:"), 0);
            }
            let stack = wrapped(kit, built);
            (kit.send_id)(stack, kit.sel("setAlignment:"), 0);
            let color = ui_color(kit, *c);
            (kit.send_two)(stack, kit.sel("setBackgroundColor:"), color);
            let layer = kit.send(stack, "layer");
            (kit.send_float)(layer, kit.sel("setCornerRadius:"), *radius as f64);
            (kit.send_id)(layer, kit.sel("setMasksToBounds:"), 1);
            let screen_width = msg!(kit, extern "C" fn(Id, Id) -> Rect)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("bounds")).width;
            at_least(kit, stack, screen_width - 32.0);
            // Never wider than the screen: its texts wrap inside the room, they do not push the page sideways.
            let narrow = (kit.send_float)(kit.send(stack, "widthAnchor"), kit.sel("constraintLessThanOrEqualToConstant:"), screen_width - 32.0);
            (kit.send_id)(narrow, kit.sel("setActive:"), 1);
            return stack;
        }
        // Room around a view: a stack view with margins.
        UiNode::Spaced(m, inner) => {
            let stack = wrapped(kit, build(kit, target, inner, made_views));
            let m = *m as f64;
            (kit.send_insets)(stack, kit.sel("setLayoutMargins:"), Insets { top: m, left: m, bottom: m, right: m });
            (kit.send_id)(stack, kit.sel("setLayoutMarginsRelativeArrangement:"), 1);
            return stack;
        }
        UiNode::Canvas(c) => {
            let view = made(kit, if c.gpu() { "KovaUiGpu" } else { "KovaUiCanvas" });
            if c.gpu() {
                // A custom view's scale is 1: ask for the screen's, so that the surface has its pixels.
                let scale = msg!(kit, extern "C" fn(Id, Id) -> f64)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("scale"));
                (kit.send_float)(view, kit.sel("setContentScaleFactor:"), scale);
            }
            (kit.send_id)(view, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
            (kit.send_two)(view, kit.sel("setBackgroundColor:"), kit.send(kit.class("UIColor"), "whiteColor"));
            let (scroll, by) = (*SCROLL.get(), msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id));
            let frame = kit.send(scroll, "frameLayoutGuide");
            // A size of 0 (`ui.fill`): the room of the screen, less the margins.
            for (anchor, length) in [("widthAnchor", c.width), ("heightAnchor", c.height)] {
                let fixed = if length == 0 {
                    by(kit.send(view, anchor), kit.sel("constraintEqualToAnchor:constant:"), kit.send(frame, anchor), -32.0)
                } else {
                    (kit.send_float)(kit.send(view, anchor), kit.sel("constraintEqualToConstant:"), length as f64)
                };
                (kit.send_id)(fixed, kit.sel("setActive:"), 1);
            }
            CANVASES.get().push((view, c.clone()));
            view
        }
    };
    made_views.push((leaf, core::ptr::null_mut()));
    leaf
}

/// The texts and values of the views already there.
fn refresh(kit: &Kit, leaf: &UiNode, v: Id, label: Id) {
    match leaf {
        UiNode::Text(s) => {
            if text_of(kit, kit.send(v, "text")) != *s {
                (kit.send_two)(v, kit.sel("setText:"), kit.text(s));
            }
        }
        UiNode::Input(s, _) | UiNode::Search(s, _) => {
            if text_of(kit, kit.send(v, "text")) != *s {
                (kit.send_two)(v, kit.sel("setText:"), kit.text(s));
            }
        }
        UiNode::Button(s, _) => {
            msg!(kit, extern "C" fn(Id, Id, Id, i64) -> Id)(v, kit.sel("setTitle:forState:"), kit.text(s), 0);
        }
        UiNode::Check(s, on, _) => {
            (kit.send_id)(v, kit.sel("setOn:"), *on as i64);
            if text_of(kit, kit.send(label, "text")) != *s {
                (kit.send_two)(label, kit.sel("setText:"), kit.text(s));
            }
        }
        UiNode::Slider(value, ..) => {
            let now = msg!(kit, extern "C" fn(Id, Id) -> f32)(v, kit.sel("value"));
            if (now + if now < 0.0 { -0.5 } else { 0.5 }) as i64 != *value as i64 {
                msg!(kit, extern "C" fn(Id, Id, f32) -> Id)(v, kit.sel("setValue:"), *value as f32);
            }
        }
        UiNode::Date(text, _) => {
            if picked_date(kit, v) != *text {
                let date = (kit.send_two)(date_formatter(kit), kit.sel("dateFromString:"), kit.text(text));
                if !date.is_null() {
                    (kit.send_two)(v, kit.sel("setDate:"), date);
                }
            }
        }
        UiNode::Stepper(value, ..) => {
            let now = msg!(kit, extern "C" fn(Id, Id) -> f64)(v, kit.sel("value"));
            if now as i64 != *value as i64 {
                msg!(kit, extern "C" fn(Id, Id, f64) -> Id)(v, kit.sel("setValue:"), *value as f64);
            }
        }
        UiNode::Progress(value, most) => {
            msg!(kit, extern "C" fn(Id, Id, f32) -> Id)(v, kit.sel("setProgress:"), *value as f32 / *most as f32);
        }
        UiNode::Choice(_, k, _) => {
            (kit.send_id)(v, kit.sel("setSelectedSegmentIndex:"), *k as i64);
        }
        UiNode::Canvas(c) => {
            if let Some(there) = CANVASES.get().iter_mut().find(|e| e.0 == v) {
                there.1 = c.clone();
            }
            kit.send(v, "setNeedsDisplay");
        }
        UiNode::Table(table) => {
            let rows = kit.send(v, "arrangedSubviews");
            // The headers, when there are some, are the first line: a label, not a row.
            let first = !table.headers.is_empty() as i64;
            if first == 1 {
                let head = (kit.send_id)(rows, kit.sel("objectAtIndex:"), 0);
                (kit.send_two)(head, kit.sel("setText:"), kit.text(&table.headers.join("  ")));
            }
            for (k, row) in table.rows.iter().enumerate() {
                let b = (kit.send_id)(rows, kit.sel("objectAtIndex:"), k as i64 + first);
                msg!(kit, extern "C" fn(Id, Id, Id, i64) -> Id)(b, kit.sel("setTitle:forState:"), kit.text(&alloc::format!("{}{}", if table.selected == k as i32 { "▸ " } else { "" }, row.join("  "))), 0);
            }
        }
        _ => {}
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Range {
    location: u64,
    length: u64,
}

/// A label showing a line of Markdown: its marks gone, bold, stressed, code and links as attributes of the text.
fn rich_text(kit: &Kit, label: Id, text: &str, size: f64, style: UiStyle) {
    let (plain, runs) = crate::shared::ui_marks(text);
    let all = (kit.send_two)(kit.send(kit.class("NSMutableAttributedString"), "alloc"), kit.sel("initWithString:"), kit.text(&plain));
    let all = kit.send(all, "autorelease");
    let add = msg!(kit, extern "C" fn(Id, Id, Id, Id, Range));
    let ui_font = |name: &str| (kit.send_float)(kit.class("UIFont"), kit.sel(name), size);
    let width = plain.encode_utf16().count() as u64;
    let color = match style.color {
        Some(c) => ui_color(kit, c),
        None => kit.send(kit.class("UIColor"), "labelColor"),
    };
    add(all, kit.sel("addAttribute:value:range:"), kit.text("NSFont"), ui_font(if style.bold { "boldSystemFontOfSize:" } else { "systemFontOfSize:" }), Range { location: 0, length: width });
    add(all, kit.sel("addAttribute:value:range:"), kit.text("NSColor"), color, Range { location: 0, length: width });
    for (start, length, kind) in runs {
        let range = Range { location: start as u64, length: length as u64 };
        match kind {
            1 => add(all, kit.sel("addAttribute:value:range:"), kit.text("NSFont"), ui_font("boldSystemFontOfSize:"), range),
            2 => add(all, kit.sel("addAttribute:value:range:"), kit.text("NSFont"), ui_font("italicSystemFontOfSize:"), range),
            3 => {
                let mono = msg!(kit, extern "C" fn(Id, Id, f64, f64) -> Id)(kit.class("UIFont"), kit.sel("monospacedSystemFontOfSize:weight:"), size * 0.9, 0.0);
                add(all, kit.sel("addAttribute:value:range:"), kit.text("NSFont"), mono, range);
            }
            _ => {
                add(all, kit.sel("addAttribute:value:range:"), kit.text("NSColor"), kit.send(kit.class("UIColor"), "systemBlueColor"), range);
            }
        }
    }
    (kit.send_two)(label, kit.sel("setAttributedText:"), all);
}

/// Gives a text, button or field its style; without one, the system's font and color.
fn styled(kit: &Kit, view: Id, label: Id, leaf: &UiNode, style: UiStyle) {
    if !matches!(leaf, UiNode::Text(_) | UiNode::Button(..) | UiNode::Input(..)) {
        return;
    }
    let size = if style.size > 0 { style.size as f64 } else { 17.0 }; // `UIFont.systemFontSize`
    let font = (kit.send_float)(kit.class("UIFont"), kit.sel(if style.bold { "boldSystemFontOfSize:" } else { "systemFontOfSize:" }), size);
    let _ = label;
    if matches!(leaf, UiNode::Button(..)) {
        (kit.send_two)(kit.send(view, "titleLabel"), kit.sel("setFont:"), font);
        // A color the program asked for (a red `Delete`); else the system's tint.
        if let Some(c) = style.color {
            msg!(kit, extern "C" fn(Id, Id, Id, i64) -> Id)(view, kit.sel("setTitleColor:forState:"), ui_color(kit, c), 0);
        }
        return;
    }
    (kit.send_two)(view, kit.sel("setFont:"), font);
    if style.marks {
        if let UiNode::Text(text) = leaf {
            rich_text(kit, view, text, size, style);
            return;
        }
    }
    let color = match style.color {
        Some(c) => {
            let channel = |shift: u32| ((c >> shift) & 0xff) as f64 / 255.0;
            msg!(kit, extern "C" fn(Id, Id, f64, f64, f64, f64) -> Id)(kit.class("UIColor"), kit.sel("colorWithRed:green:blue:alpha:"), channel(16), channel(8), channel(0), 1.0)
        }
        None => kit.send(kit.class("UIColor"), "labelColor"),
    };
    (kit.send_two)(view, kit.sel("setTextColor:"), color);
}

// ---- Dialogs and sound ----

/// What an alert's button said: 1 yes, 0 no, `i64::MIN` nothing yet.
static ANSWER: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(i64::MIN);
static ASKED: AtomicU32 = AtomicU32::new(0);

/// A block of Objective-C that holds no value, as the system's own `^{ }` at file level: the
/// system finds its code in `invoke` and copies nothing.
#[repr(C)]
struct Block {
    isa: *const u8,
    flags: i32,
    reserved: i32,
    invoke: usize,
    descriptor: *const Descriptor,
}

#[repr(C)]
struct Descriptor {
    reserved: usize,
    size: usize,
}

static DESCRIPTOR: Descriptor = Descriptor { reserved: 0, size: core::mem::size_of::<Block>() };

/// BLOCK_IS_GLOBAL.
fn global_block(invoke: usize) -> Id {
    let isa = library("/usr/lib/libSystem.B.dylib").and_then(|l| symbol(l, "_NSConcreteGlobalBlock")).unwrap_or(0);
    alloc::boxed::Box::leak(alloc::boxed::Box::new(Block { isa: isa as *const u8, flags: 1 << 28, reserved: 0, invoke, descriptor: &DESCRIPTOR })) as *mut Block as Id
}

fn answered(value: i64) {
    ANSWER.store(value, SeqCst);
    ASKED.fetch_add(1, SeqCst);
    super::wake(&ASKED);
}

extern "C" fn said_yes(_block: Id, _action: Id) {
    answered(1);
}

extern "C" fn said_no(_block: Id, _action: Id) {
    answered(0);
}

/// `ui.dialog` on iOS: a message or a question (`kind` 3 and 4); the file dialogs are not there yet.
/// What the file picker gave back: its path, read by `dialog` once `ANSWER` says 1.
static PICKED: crate::Global<Option<String>> = crate::Global::new(None);

extern "C" fn picked_file(_me: Id, _message: Id, _picker: Id, urls: Id) {
    let kit = &screen().kit;
    let url = (kit.send_id)(urls, kit.sel("objectAtIndex:"), 0);
    // A file outside the app's folder is only reachable while this is on.
    kit.send(url, "startAccessingSecurityScopedResource");
    *PICKED.get() = Some(text_of(kit, kit.send(url, "path")));
    answered(1);
}

extern "C" fn picked_nothing(_me: Id, _message: Id, _picker: Id) {
    answered(0);
}

pub fn dialog(kind: i64, title: &str, text: &str) -> Option<String> {
    if kind >= 10 {
        return ios_raw(kind, title, text);
    }
    if SCREEN.get().is_none() {
        return None;
    }
    struct Job<'a> {
        kind: i64,
        title: &'a str,
        text: &'a str,
    }
    extern "C" fn present(job: *mut u8) {
        let job = unsafe { &*(job as *const Job) };
        let screen = screen();
        let kit = &screen.kit;
        if job.kind < 3 {
            // The system's file picker: open a file (0), save one (1), open a folder (2).
            if let Some(types) = library("/System/Library/Frameworks/UniformTypeIdentifiers.framework/UniformTypeIdentifiers") {
                let _ = types;
            }
            let make = msg!(kit, extern "C" fn(Id, Id, Id) -> Id);
            let one = |name: &str| {
                let t = make(kit.class("UTType"), kit.sel("typeWithIdentifier:"), kit.text(name));
                msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), &t, 1)
            };
            let alloc = kit.send(kit.class("UIDocumentPickerViewController"), "alloc");
            let picker = match job.kind {
                0 => msg!(kit, extern "C" fn(Id, Id, Id, bool) -> Id)(alloc, kit.sel("initForOpeningContentTypes:asCopy:"), one("public.data"), true),
                2 => (kit.send_two)(alloc, kit.sel("initForOpeningContentTypes:"), one("public.folder")),
                _ => {
                    // The picker saves a copy of a file that exists: an empty one, with the name offered.
                    let folder = kit.send(kit.send(kit.class("NSFileManager"), "defaultManager"), "temporaryDirectory");
                    let url = (kit.send_two)(folder, kit.sel("URLByAppendingPathComponent:"), kit.text(if job.text.is_empty() { "untitled" } else { job.text }));
                    msg!(kit, extern "C" fn(Id, Id, Id, bool) -> bool)(kit.send(kit.class("NSData"), "data"), kit.sel("writeToURL:atomically:"), url, false);
                    let urls = msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), &url, 1);
                    msg!(kit, extern "C" fn(Id, Id, Id, bool) -> Id)(alloc, kit.sel("initForExportingURLs:asCopy:"), urls, true)
                }
            };
            (kit.send_two)(picker, kit.sel("setDelegate:"), screen.target);
            msg!(kit, extern "C" fn(Id, Id, Id, bool, Id) -> Id)(screen.controller, kit.sel("presentViewController:animated:completion:"), picker, true, core::ptr::null_mut());
            return;
        }
        let alert = msg!(kit, extern "C" fn(Id, Id, Id, Id, i64) -> Id)(kit.class("UIAlertController"), kit.sel("alertControllerWithTitle:message:preferredStyle:"), kit.text(job.title), kit.text(job.text), 1);
        let button = msg!(kit, extern "C" fn(Id, Id, Id, i64, Id) -> Id);
        let names: &[(&str, usize, i64)] = if job.kind == 3 { &[("OK", said_yes as usize, 0)] } else { &[("No", said_no as usize, 1), ("Yes", said_yes as usize, 0)] };
        for &(name, invoke, style) in names {
            let action = button(kit.class("UIAlertAction"), kit.sel("actionWithTitle:style:handler:"), kit.text(name), style, global_block(invoke));
            (kit.send_two)(alert, kit.sel("addAction:"), action);
        }
        msg!(kit, extern "C" fn(Id, Id, Id, bool, Id) -> Id)(screen.controller, kit.sel("presentViewController:animated:completion:"), alert, true, core::ptr::null_mut());
    }
    ANSWER.store(i64::MIN, SeqCst);
    let system = library("/usr/lib/libSystem.B.dylib")?;
    let sync: extern "C" fn(usize, *mut u8, extern "C" fn(*mut u8)) = unsafe { core::mem::transmute(symbol(system, "dispatch_sync_f")?) };
    let job = Job { kind, title, text };
    sync(symbol(system, "_dispatch_main_q")?, &job as *const Job as *mut u8, present);
    loop {
        let seen = ASKED.load(SeqCst);
        let answer = ANSWER.load(SeqCst);
        if answer != i64::MIN {
            if kind < 3 {
                return if answer == 1 { PICKED.get().take() } else { None };
            }
            return (answer == 1).then(|| "yes".into());
        }
        super::wait(&ASKED, seen);
    }
}

/// `sound.play` on iOS: AVFoundation's player; the program waits for the end of the sound.
pub fn sound(whole: &[u8], wav: &crate::shared::Wav) -> bool {
    let (Some(_), Some(kit)) = (library("/System/Library/Frameworks/AVFoundation.framework/AVFoundation"), objc_kit()) else { return false };
    let pool = (kit.push)();
    let data = msg!(kit, extern "C" fn(Id, Id, *const u8, u64) -> Id)(kit.class("NSData"), kit.sel("dataWithBytes:length:"), whole.as_ptr(), whole.len() as u64);
    let player = msg!(kit, extern "C" fn(Id, Id, Id, *mut Id) -> Id)(kit.send(kit.class("AVAudioPlayer"), "alloc"), kit.sel("initWithData:error:"), data, core::ptr::null_mut());
    let played = !player.is_null() && (kit.send_int)(player, kit.sel("play")) & 0xff != 0;
    if played {
        super::sleep(wav.samples.len() as f64 / (wav.rate * wav.channels * wav.bits / 8).max(1) as f64 + 0.1);
        kit.send(player, "stop");
    }
    if !player.is_null() {
        kit.send(player, "release");
    }
    (kit.pop)(pool);
    played
}

// ---- Canvases ----

fn touch(me: Id, touches: Id, kind: &str) {
    let Some(screen) = SCREEN.get().as_ref() else { return };
    let kit = &screen.kit;
    let Some(n) = CANVASES.get().iter().find(|e| e.0 == me).and_then(|e| e.1.mouse) else { return };
    let any = kit.send(touches, "anyObject");
    let at = msg!(kit, extern "C" fn(Id, Id, Id) -> Size)(any, kit.sel("locationInView:"), me);
    hear(crate::shared::ui_changed(n), crate::shared::ui_mouse(kind, at.width as i32, at.height as i32, 1));
}

extern "C" fn touch_began(me: Id, _message: Id, touches: Id, _event: Id) {
    touch(me, touches, "press");
}

extern "C" fn touch_moved(me: Id, _message: Id, touches: Id, _event: Id) {
    touch(me, touches, "move");
}

extern "C" fn touch_ended(me: Id, _message: Id, touches: Id, _event: Id) {
    touch(me, touches, "release");
}

/// The drawing of a canvas: Core Graphics, in the view's own points (y goes down, as in Kova).
extern "C" fn draw_canvas(me: Id, _message: Id, _dirty: Rect) {
    use crate::shared::UiOp as Op;
    let Some(screen) = SCREEN.get().as_ref() else { return };
    let kit = &screen.kit;
    let Some((_, canvas)) = CANVASES.get().iter().find(|e| e.0 == me) else { return };
    let bounds = msg!(kit, extern "C" fn(Id, Id) -> Rect)(me, kit.sel("bounds"));
    let (Some(graphics), Some(uikit)) = (library("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics"), library("/System/Library/Frameworks/UIKit.framework/UIKit")) else { return };
    let f = |name: &str| symbol(graphics, name).unwrap_or(0);
    unsafe {
        use core::mem::transmute as t;
        let current: extern "C" fn() -> Id = t(symbol(uikit, "UIGraphicsGetCurrentContext").unwrap_or(0));
        let context = current();
        let fill_color: extern "C" fn(Id, f64, f64, f64, f64) = t(f("CGContextSetRGBFillColor"));
        let stroke_color: extern "C" fn(Id, f64, f64, f64, f64) = t(f("CGContextSetRGBStrokeColor"));
        let fill_rect: extern "C" fn(Id, Rect) = t(f("CGContextFillRect"));
        let fill_oval: extern "C" fn(Id, Rect) = t(f("CGContextFillEllipseInRect"));
        let stroke_rect: extern "C" fn(Id, Rect) = t(f("CGContextStrokeRect"));
        let stroke_oval: extern "C" fn(Id, Rect) = t(f("CGContextStrokeEllipseInRect"));
        let thickness: extern "C" fn(Id, f64) = t(f("CGContextSetLineWidth"));
        let move_to: extern "C" fn(Id, f64, f64) = t(f("CGContextMoveToPoint"));
        let line_to: extern "C" fn(Id, f64, f64) = t(f("CGContextAddLineToPoint"));
        let stroke: extern "C" fn(Id) = t(f("CGContextStrokePath"));
        let part = |c: u32, shift: u32| (c >> shift & 255) as f64 / 255.0;
        let fill = |c: u32| fill_color(context, part(c, 16), part(c, 8), part(c, 0), 1.0);
        let line = |c: u32, n: i32| {
            stroke_color(context, part(c, 16), part(c, 8), part(c, 0), 1.0);
            thickness(context, n as f64);
        };
        let inside = |x: i32, y: i32, w: i32, h: i32, n: i32| {
            let inset = n as f64 / 2.0;
            Rect { x: x as f64 + inset, y: y as f64 + inset, width: w as f64 - 2.0 * inset, height: h as f64 - 2.0 * inset }
        };
        let whole = Rect { x: 0.0, y: 0.0, width: bounds.width, height: bounds.height };
        fill(0xffffff);
        fill_rect(context, whole);
        if canvas.gpu() {
            // The graphics card's drawing is OpenGL's: not on iOS yet.
            return;
        }
        for op in &canvas.ops {
            match op {
                Op::Gpu | Op::GlClear(_) | Op::GlShader(..) | Op::GlUniform(..) | Op::GlTriangles(..) | Op::GlTexture(..) => {}
                Op::Clear(c) => {
                    fill(*c);
                    fill_rect(context, whole);
                }
                Op::Rect { x, y, w, h, color, outline: 0 } => {
                    fill(*color);
                    fill_rect(context, inside(*x, *y, *w, *h, 0));
                }
                Op::Rect { x, y, w, h, color, outline } => {
                    line(*color, *outline);
                    stroke_rect(context, inside(*x, *y, *w, *h, *outline));
                }
                Op::Circle { x, y, r, color, outline: 0 } => {
                    fill(*color);
                    fill_oval(context, inside(x - r, y - r, 2 * r, 2 * r, 0));
                }
                Op::Circle { x, y, r, color, outline } => {
                    line(*color, *outline);
                    stroke_oval(context, inside(x - r, y - r, 2 * r, 2 * r, *outline));
                }
                Op::Line { x, y, x2, y2, color, thickness } => {
                    line(*color, *thickness);
                    move_to(context, *x as f64, *y as f64);
                    line_to(context, *x2 as f64, *y2 as f64);
                    stroke(context);
                }
                Op::Image { x, y, w, h, path } => {
                    let image = (kit.send_two)(kit.class("UIImage"), kit.sel("imageWithContentsOfFile:"), kit.text(path));
                    if !image.is_null() {
                        let own = msg!(kit, extern "C" fn(Id, Id) -> Size)(image, kit.sel("size"));
                        let place = Rect { x: *x as f64, y: *y as f64, width: if *w > 0 { *w as f64 } else { own.width }, height: if *h > 0 { *h as f64 } else { own.height } };
                        msg!(kit, extern "C" fn(Id, Id, Rect) -> Id)(image, kit.sel("drawInRect:"), place);
                    }
                }
                Op::Text { x, y, size, color, text } => {
                    let named = |name: &str| symbol(uikit, name).map_or(core::ptr::null_mut(), |p| *(p as *const Id));
                    let keys = [named("NSFontAttributeName"), named("NSForegroundColorAttributeName")];
                    let values = [
                        (kit.send_float)(kit.class("UIFont"), kit.sel("systemFontOfSize:"), *size as f64),
                        msg!(kit, extern "C" fn(Id, Id, f64, f64, f64, f64) -> Id)(kit.class("UIColor"), kit.sel("colorWithRed:green:blue:alpha:"), part(*color, 16), part(*color, 8), part(*color, 0), 1.0),
                    ];
                    let attributes = msg!(kit, extern "C" fn(Id, Id, *const Id, *const Id, u64) -> Id)(kit.class("NSDictionary"), kit.sel("dictionaryWithObjects:forKeys:count:"), values.as_ptr(), keys.as_ptr(), 2);
                    msg!(kit, extern "C" fn(Id, Id, Size, Id))(kit.text(text), kit.sel("drawAtPoint:withAttributes:"), Size { width: *x as f64, height: *y as f64 }, attributes);
                }
            }
        }
    }
}

// ---- Menus ----

/// A row of buttons, one for each menu; a tap opens the menu (`UIMenu`, as the primary action).
/// A navigation bar button that opens one menu.
/// The buttons a page gives its navigation bar (`.nav_button`): each a bar item that sends its event's number.
fn nav_items(kit: &Kit, target: Id, buttons: &[(String, usize)]) -> Id {
    let items: Vec<Id> = buttons
        .iter()
        .rev()
        .map(|(label, n)| {
            let item = msg!(kit, extern "C" fn(Id, Id, Id, i64, Id, Id) -> Id)(kit.send(kit.class("UIBarButtonItem"), "alloc"), kit.sel("initWithTitle:style:target:action:"), kit.text(label), 0, target, kit.sel("tapped:"));
            (kit.send_id)(item, kit.sel("setTag:"), *n as i64);
            kit.send(item, "autorelease")
        })
        .collect();
    msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), items.as_ptr(), items.len() as u64)
}

fn bar_button(kit: &Kit, m: &crate::shared::UiMenu) -> Id {
    let actions: Vec<Id> = m
        .items
        .iter()
        .filter_map(|(label, event, _key)| {
            // The number of the event is the action's identifier, which the handler reads back.
            let n = (*event)?;
            Some(msg!(kit, extern "C" fn(Id, Id, Id, Id, Id, Id) -> Id)(
                kit.class("UIAction"),
                kit.sel("actionWithTitle:image:identifier:handler:"),
                kit.text(label),
                core::ptr::null_mut(),
                kit.text(&alloc::format!("{n}")),
                global_block(menu_chosen as usize),
            ))
        })
        .collect();
    let array = msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), actions.as_ptr(), actions.len() as u64);
    let menu = msg!(kit, extern "C" fn(Id, Id, Id, Id) -> Id)(kit.class("UIMenu"), kit.sel("menuWithTitle:children:"), kit.text(&m.title), array);
    if !m.icon.is_empty() {
        let image = (kit.send_two)(kit.class("UIImage"), kit.sel("systemImageNamed:"), kit.text(&m.icon));
        return msg!(kit, extern "C" fn(Id, Id, Id, Id) -> Id)(kit.send(kit.class("UIBarButtonItem"), "alloc"), kit.sel("initWithImage:menu:"), image, menu);
    }
    msg!(kit, extern "C" fn(Id, Id, Id, Id) -> Id)(kit.send(kit.class("UIBarButtonItem"), "alloc"), kit.sel("initWithTitle:menu:"), kit.text(&m.title), menu)
}

/// An item of a menu was chosen: its action's identifier is the number of its event.
extern "C" fn menu_chosen(_block: Id, action: Id) {
    let Some(screen) = SCREEN.get().as_ref() else { return };
    let kit = &screen.kit;
    if let Ok(n) = text_of(kit, kit.send(action, "identifier")).parse::<usize>() {
        // A click, as on the other systems: the number of the item's event.
        hear(n as i64, String::new());
    }
}

// ---- ui.gpu ----

/// The OpenGL ES 3.0 context of a `ui.gpu` view, and the buffers it draws into.
struct Gpu {
    view: Id,
    context: Id,
    framebuffer: u32,
    color: u32,
    depth: u32,
    size: (i32, i32),
}

static GPUS: crate::Global<Vec<Gpu>> = crate::Global::new(Vec::new());

extern "C" fn gpu_layer(_class: Id, _message: Id) -> Id {
    screen().kit.class("CAEAGLLayer")
}

/// Paints a frame of a `ui.gpu`: the view's layer is the screen of an OpenGL ES context; `gl_paint`
/// draws as on the other systems, then the frame is shown. OpenGL ES is old on Apple's systems
/// too, and works on every iPhone.
fn gpu_paint(kit: &Kit, view: Id, canvas: &crate::shared::UiCanvas, bounds: Rect) {
    let Some(gl) = library("/System/Library/Frameworks/OpenGLES.framework/OpenGLES") else { crate::gl_stop(crate::GPU_MISSING, b"") };
    unsafe {
        use core::mem::transmute as t;
        let load = |name: &str| symbol(gl, name).unwrap_or(0);
        let scale = msg!(kit, extern "C" fn(Id, Id) -> f64)(view, kit.sel("contentScaleFactor"));
        let (width, height) = ((bounds.width * scale) as i32, (bounds.height * scale) as i32);
        if width <= 0 || height <= 0 {
            return;
        }
        let gpus = GPUS.get();
        let at = match gpus.iter().position(|g| g.view == view) {
            Some(at) => at,
            None => {
                // kEAGLRenderingAPIOpenGLES3 is 3.
                let context = (kit.send_id)(kit.send(kit.class("EAGLContext"), "alloc"), kit.sel("initWithAPI:"), 3);
                if context.is_null() {
                    crate::gl_stop(crate::GPU_MISSING, b"");
                }
                gpus.push(Gpu { view, context, framebuffer: 0, color: 0, depth: 0, size: (0, 0) });
                gpus.len() - 1
            }
        };
        let gpu = &mut gpus[at];
        (kit.send_two)(kit.class("EAGLContext"), kit.sel("setCurrentContext:"), gpu.context);
        let generate: extern "C" fn(i32, *mut u32) = t(load("glGenFramebuffers"));
        let generate_render: extern "C" fn(i32, *mut u32) = t(load("glGenRenderbuffers"));
        let bind_frame: extern "C" fn(u32, u32) = t(load("glBindFramebuffer"));
        let bind_render: extern "C" fn(u32, u32) = t(load("glBindRenderbuffer"));
        let storage: extern "C" fn(u32, u32, i32, i32) = t(load("glRenderbufferStorage"));
        let attach: extern "C" fn(u32, u32, u32, u32) = t(load("glFramebufferRenderbuffer"));
        if gpu.framebuffer == 0 {
            generate(1, &mut gpu.framebuffer);
            generate_render(1, &mut gpu.color);
            generate_render(1, &mut gpu.depth);
        }
        bind_frame(0x8D40, gpu.framebuffer); // GL_FRAMEBUFFER
        if gpu.size != (width, height) {
            // The color comes from the layer, which gets its size from the view; the depth is ours.
            bind_render(0x8D41, gpu.color); // GL_RENDERBUFFER
            let layer = kit.send(view, "layer");
            msg!(kit, extern "C" fn(Id, Id, u64, Id) -> bool)(gpu.context, kit.sel("renderbufferStorage:fromDrawable:"), 0x8D41, layer);
            attach(0x8D40, 0x8CE0, 0x8D41, gpu.color); // GL_COLOR_ATTACHMENT0
            bind_render(0x8D41, gpu.depth);
            storage(0x8D41, 0x81A6, width, height); // GL_DEPTH_COMPONENT24
            attach(0x8D40, 0x8D00, 0x8D41, gpu.depth); // GL_DEPTH_ATTACHMENT
            gpu.size = (width, height);
        }
        crate::gl_paint(view as usize, &canvas.ops, width, height, &load);
        bind_render(0x8D41, gpu.color);
        msg!(kit, extern "C" fn(Id, Id, u64) -> bool)(gpu.context, kit.sel("presentRenderbuffer:"), 0x8D41);
    }
}
