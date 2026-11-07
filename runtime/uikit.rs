// `ui.show` on iOS (PLAN Part 4 "iOS", step 2): UIKit through the Objective-C runtime, as
// `macos.rs` does AppKit. A child module of `macos.rs`, built only with `--cfg ios`.
//
// UIKit owns the main thread (`UIApplicationMain` never returns), so the Kova program runs on a
// second thread, started once the app has a window. `show` (the program's thread) hands the view
// to the main thread with `dispatch_sync_f`, which draws it, then sleeps until a tap, a keystroke
// or a clock puts an event in `HEARD`. What is drawn: texts, buttons, fields, check boxes, sliders,
// choices, images, lists (as rows of buttons), columns and rows with their style and margins. The
// canvas, `ui.gpu`, tables with headers, menus and dialogs are not drawn yet.

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
    /// The object that hears the buttons and fields (`KovaUiTarget`).
    target: Id,
    /// The view of the window's controller.
    root: Id,
    /// The stack view now shown.
    content: Id,
    shape: String,
    /// Each text, button and field now shown, with the label that goes with it (0: none).
    made: Vec<(Id, Id)>,
    clocks: Vec<(i32, usize)>,
    timers: Vec<Id>,
}

/// Only the main thread reaches it.
static SCREEN: crate::Global<Option<Screen>> = crate::Global::new(None);
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

/// The app has started: a window with an empty view, then the program.
extern "C" fn finished(_me: Id, _message: Id, _app: Id, _options: Id) -> bool {
    let Some(kit) = objc_kit() else { return false };
    unsafe {
        use core::mem::transmute as t;
        let objc = library("/usr/lib/libobjc.A.dylib").unwrap();
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
        add(class, kit.sel("textFieldShouldReturn:"), returned as usize, ret);
        register(class);
        let target = kit.send(kit.send(class, "alloc"), "init");
        let bounds = msg!(kit, extern "C" fn(Id, Id) -> Rect)(kit.send(kit.class("UIScreen"), "mainScreen"), kit.sel("bounds"));
        let w = msg!(kit, extern "C" fn(Id, Id, Rect) -> Id)(kit.send(kit.class("UIWindow"), "alloc"), kit.sel("initWithFrame:"), bounds);
        let controller = kit.send(kit.send(kit.class("UIViewController"), "alloc"), "init");
        let root = kit.send(controller, "view");
        (kit.send_two)(root, kit.sel("setBackgroundColor:"), kit.send(kit.class("UIColor"), "systemBackgroundColor"));
        (kit.send_two)(w, kit.sel("setRootViewController:"), controller);
        kit.send(w, "makeKeyAndVisible");
        *WINDOW.get() = w;
        *SCREEN.get() = Some(Screen { kit, target, root, content: core::ptr::null_mut(), shape: String::new(), made: Vec::new(), clocks: Vec::new(), timers: Vec::new() });
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
pub fn show(_title: &str, view: &UiNode, _menus: &[crate::shared::UiMenu], _width: i32, _height: i32, acts: &[i64], typed: &mut String) -> i64 {
    // `ui.close`: the app ends.
    if acts.contains(&1) {
        return -1;
    }
    struct Job<'a> {
        view: &'a UiNode,
    }
    extern "C" fn draw(job: *mut u8) {
        let job = unsafe { &*(job as *const Job) };
        draw_view(job.view);
    }
    let system = library("/usr/lib/libSystem.B.dylib").expect("libSystem");
    let sync: extern "C" fn(usize, *mut u8, extern "C" fn(*mut u8)) = unsafe { core::mem::transmute(symbol(system, "dispatch_sync_f").expect("dispatch_sync_f")) };
    let main_queue = symbol(system, "_dispatch_main_q").expect("the main queue");
    let job = Job { view };
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
fn draw_view(view: &UiNode) {
    let Some(screen) = SCREEN.get().as_mut() else { return };
    let kit = &screen.kit;
    let pool = (kit.push)();
    let shape = crate::shared::ui_shape(view);
    if shape == screen.shape {
        for ((leaf, _), &(v, label)) in crate::shared::ui_leaves(view).into_iter().zip(screen.made.iter()) {
            refresh(kit, leaf, v, label);
        }
    } else {
        if !screen.content.is_null() {
            kit.send(screen.content, "removeFromSuperview");
        }
        screen.made.clear();
        let content = build(kit, screen.target, view, &mut screen.made);
        let content = if kit.class("UIStackView") == kit.send(content, "class") { content } else { wrapped(kit, content) };
        place(kit, screen.root, content, crate::shared::ui_fills(view));
        screen.content = content;
        screen.shape = shape;
    }
    for ((leaf, style), &(v, label)) in crate::shared::ui_leaves(view).into_iter().zip(screen.made.iter()) {
        styled(kit, v, label, leaf, style);
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

/// `content` in the safe area of `root`, 16 points from its edges: at the top left, or across the
/// width when something in it fills.
fn place(kit: &Kit, root: Id, content: Id, fills: bool) {
    (kit.send_id)(content, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
    (kit.send_two)(root, kit.sel("addSubview:"), content);
    let safe = kit.send(root, "safeAreaLayoutGuide");
    let by: extern "C" fn(Id, Id, Id, f64) -> Id = msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id);
    let on = |constraint: Id| {
        (kit.send_id)(constraint, kit.sel("setActive:"), 1);
    };
    on(by(kit.send(content, "topAnchor"), kit.sel("constraintEqualToAnchor:constant:"), kit.send(safe, "topAnchor"), 16.0));
    on(by(kit.send(content, "leadingAnchor"), kit.sel("constraintEqualToAnchor:constant:"), kit.send(safe, "leadingAnchor"), 16.0));
    let end = if fills { "constraintEqualToAnchor:constant:" } else { "constraintLessThanOrEqualToAnchor:constant:" };
    on(by(kit.send(content, "trailingAnchor"), kit.sel(end), kit.send(safe, "trailingAnchor"), -16.0));
}

/// A width of at least `points`.
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
            label
        }
        UiNode::Button(s, n) => {
            let b = (kit.send_id)(kit.class("UIButton"), kit.sel("buttonWithType:"), 1);
            msg!(kit, extern "C" fn(Id, Id, Id, i64) -> Id)(b, kit.sel("setTitle:forState:"), kit.text(s), 0);
            hears(kit, target, b, *n, "tapped:", 64);
            b
        }
        UiNode::Input(s, n) => {
            let field = made(kit, "UITextField");
            (kit.send_two)(field, kit.sel("setText:"), kit.text(s));
            (kit.send_id)(field, kit.sel("setBorderStyle:"), 3);
            (kit.send_two)(field, kit.sel("setDelegate:"), target);
            hears(kit, target, field, *n, "edited:", 131072);
            at_least(kit, field, 200.0);
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
            (kit.send_two)(stack, kit.sel("addArrangedSubview:"), switch);
            (kit.send_two)(stack, kit.sel("addArrangedSubview:"), label);
            made_views.push((switch, label));
            return stack;
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
        UiNode::Table(table) => {
            let stack = made(kit, "UIStackView");
            (kit.send_id)(stack, kit.sel("setAxis:"), 1);
            (kit.send_id)(stack, kit.sel("setAlignment:"), 1);
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
        UiNode::Styled(_, inner) | UiNode::Every(_, _, inner) => return build(kit, target, inner, made_views),
        // Room around a view: a stack view with margins.
        UiNode::Spaced(m, inner) => {
            let stack = wrapped(kit, build(kit, target, inner, made_views));
            let m = *m as f64;
            (kit.send_insets)(stack, kit.sel("setLayoutMargins:"), Insets { top: m, left: m, bottom: m, right: m });
            (kit.send_id)(stack, kit.sel("setLayoutMarginsRelativeArrangement:"), 1);
            return stack;
        }
        // Not drawn yet: said where it would be.
        UiNode::Canvas(_) => {
            let label = made(kit, "UILabel");
            (kit.send_two)(label, kit.sel("setText:"), kit.text("(a canvas: not on iOS yet)"));
            label
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
        UiNode::Input(s, _) => {
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
        UiNode::Choice(_, k, _) => {
            (kit.send_id)(v, kit.sel("setSelectedSegmentIndex:"), *k as i64);
        }
        UiNode::Table(table) => {
            let rows = kit.send(v, "arrangedSubviews");
            for (k, row) in table.rows.iter().enumerate() {
                let b = (kit.send_id)(rows, kit.sel("objectAtIndex:"), k as i64);
                msg!(kit, extern "C" fn(Id, Id, Id, i64) -> Id)(b, kit.sel("setTitle:forState:"), kit.text(&alloc::format!("{}{}", if table.selected == k as i32 { "▸ " } else { "" }, row.join("  "))), 0);
            }
        }
        _ => {}
    }
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
        return;
    }
    (kit.send_two)(view, kit.sel("setFont:"), font);
    let color = match style.color {
        Some(c) => {
            let channel = |shift: u32| ((c >> shift) & 0xff) as f64 / 255.0;
            msg!(kit, extern "C" fn(Id, Id, f64, f64, f64, f64) -> Id)(kit.class("UIColor"), kit.sel("colorWithRed:green:blue:alpha:"), channel(16), channel(8), channel(0), 1.0)
        }
        None => kit.send(kit.class("UIColor"), "labelColor"),
    };
    (kit.send_two)(view, kit.sel("setTextColor:"), color);
}
