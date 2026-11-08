// `ui.show` on iOS (PLAN Part 4 "iOS", step 2): UIKit through the Objective-C runtime, as
// `macos.rs` does AppKit. A child module of `macos.rs`, built only with `--cfg ios`.
//
// UIKit owns the main thread (`UIApplicationMain` never returns), so the Kova program runs on a
// second thread, started once the app has a window. `show` (the program's thread) hands the view
// to the main thread with `dispatch_sync_f`, which draws it, then sleeps until a tap, a keystroke
// or a clock puts an event in `HEARD`. What is drawn: texts, buttons, fields, check boxes, sliders,
// choices, images, lists (as rows of buttons), canvases (2D drawing and touches), dialogs, sound,
// columns and rows with their style and margins. The
// `ui.gpu` and tables with headers are not drawn yet.

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
    /// The window's view controller, which shows the dialogs.
    controller: Id,
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
/// Each canvas now on screen (the main thread's).
static CANVASES: crate::Global<Vec<(Id, crate::shared::UiCanvas)>> = crate::Global::new(Vec::new());
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
    let Some(mut kit) = objc_kit() else { return false };
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
        add(class, kit.sel("dated:"), dated as usize, action);
        add(class, kit.sel("stepped:"), stepped as usize, action);
        add(class, kit.sel("tabBar:didSelectItem:"), tab_chosen as usize, c"v@:@@".as_ptr() as *const u8);
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
        // The content scrolls when it is taller than the screen; the keyboard goes away when it is dragged.
        let scroll = made(&kit, "UIScrollView");
        (kit.send_id)(scroll, kit.sel("setTranslatesAutoresizingMaskIntoConstraints:"), 0);
        (kit.send_id)(scroll, kit.sel("setKeyboardDismissMode:"), 1);
        *SCROLL.get() = scroll;
        (kit.send_two)(root, kit.sel("addSubview:"), scroll);
        let (safe, by) = (kit.send(root, "safeAreaLayoutGuide"), msg!(kit, extern "C" fn(Id, Id, Id, f64) -> Id));
        for edge in ["topAnchor", "leadingAnchor", "trailingAnchor", "bottomAnchor"] {
            (kit.send_id)(by(kit.send(scroll, edge), kit.sel("constraintEqualToAnchor:constant:"), kit.send(safe, edge), 0.0), kit.sel("setActive:"), 1);
        }
        kit.send(w, "makeKeyAndVisible");
        *WINDOW.get() = w;
        *SCREEN.get() = Some(Screen { menus: Vec::new(), kit, target, root, scroll, controller, content: core::ptr::null_mut(), shape: String::new(), made: Vec::new(), clocks: Vec::new(), timers: Vec::new() });
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
    (kit.send_two)(kit.send(screen.controller, "navigationItem"), kit.sel("setTitle:"), kit.text(title));
    if shape == screen.shape && menus == screen.menus.as_slice() {
        for ((leaf, _), &(v, label)) in crate::shared::ui_leaves(view).into_iter().zip(screen.made.iter()) {
            refresh(kit, leaf, v, label);
        }
    } else {
        if !screen.content.is_null() {
            kit.send(screen.content, "removeFromSuperview");
        }
        screen.made.clear();
        CANVASES.get().clear();
        let content = build(kit, screen.target, view, &mut screen.made);
        let content = if kit.class("UIStackView") == kit.send(content, "class") { content } else { wrapped(kit, content) };
        // The menus go in the navigation bar, each a button opening its items.
        let item = kit.send(screen.controller, "navigationItem");
        let buttons: Vec<Id> = menus.iter().rev().map(|m| bar_button(kit, m)).collect();
        let array = msg!(kit, extern "C" fn(Id, Id, *const Id, u64) -> Id)(kit.class("NSArray"), kit.sel("arrayWithObjects:count:"), buttons.as_ptr(), buttons.len() as u64);
        (kit.send_two)(item, kit.sel("setRightBarButtonItems:"), array);
        place(kit, screen.scroll, content, crate::shared::ui_fills(view));
        screen.content = content;
        screen.shape = shape;
        screen.menus = menus.to_vec();
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
        UiNode::Input(s, n) | UiNode::Search(s, n) => {
            let field = made(kit, if matches!(node, UiNode::Search(..)) { "UISearchTextField" } else { "UITextField" });
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
        // `ui.tabs`: a tab bar. The tab in use is a text (tag -1), each other a button (its number).
        UiNode::Tabs(parts) => {
            let bar = made(kit, "UITabBar");
            let mut items = Vec::new();
            let mut now = core::ptr::null_mut();
            for p in parts {
                let (name, tag) = match p {
                    UiNode::Text(t) => (t, -1),
                    UiNode::Button(t, n) => (t, *n as i64),
                    _ => continue,
                };
                let item = msg!(kit, extern "C" fn(Id, Id, Id, Id, i64) -> Id)(kit.send(kit.class("UITabBarItem"), "alloc"), kit.sel("initWithTitle:image:tag:"), kit.text(name), core::ptr::null_mut(), tag);
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
        UiNode::Styled(_, inner) | UiNode::Every(_, _, inner) => return build(kit, target, inner, made_views),
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
pub fn dialog(kind: i64, title: &str, text: &str) -> Option<String> {
    if kind < 3 || SCREEN.get().is_none() {
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
    msg!(kit, extern "C" fn(Id, Id, Id, Id) -> Id)(kit.send(kit.class("UIBarButtonItem"), "alloc"), kit.sel("initWithTitle:menu:"), kit.text(&m.title), menu)
}

/// An item of a menu was chosen: its action's identifier is the number of its event.
extern "C" fn menu_chosen(_block: Id, action: Id) {
    let Some(screen) = SCREEN.get().as_ref() else { return };
    let kit = &screen.kit;
    if let Ok(n) = text_of(kit, kit.send(action, "identifier")).parse::<usize>() {
        hear(crate::shared::ui_changed(n), String::new());
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
