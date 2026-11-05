//! Real AppKit events, handed to the window for the test driver. They take the
//! same path as a person's input: NSWindow, then gpui's view, then the app. Unlike
//! events posted to a process, these need no Accessibility permission.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_foundation::{NSPoint, NSRect, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

const LEFT_MOUSE_DOWN: usize = 1;
const LEFT_MOUSE_UP: usize = 2;
const LEFT_MOUSE_DRAGGED: usize = 6;
const CONTROL: usize = 1 << 18;
const KEY_DOWN: usize = 10;
const KEY_UP: usize = 11;

/// Whether the person asked macOS for less motion.
pub fn reduce_motion() -> bool {
    let Some(class) = AnyClass::get(c"NSWorkspace") else { return false };
    // SAFETY: both are documented NSWorkspace methods with these types.
    unsafe {
        let shared: Option<Retained<AnyObject>> = msg_send![class, sharedWorkspace];
        match shared {
            Some(shared) => msg_send![&*shared, accessibilityDisplayShouldReduceMotion],
            None => false,
        }
    }
}

/// The window's Metal view, kept as a plain address for the test driver's task.
pub fn view_address(window: &impl HasWindowHandle) -> Option<usize> {
    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    Some(handle.ns_view.as_ptr() as usize)
}

/// # Safety
/// `view` is from `view_address`, the window is open, and this is the main thread
/// outside any gpui update, because the event re-enters gpui.
unsafe fn ns_window(view: usize) -> Option<Retained<AnyObject>> {
    let view = (view as *mut AnyObject).as_ref()?;
    msg_send![view, window]
}

/// Brings the window to the front without taking the keyboard from anything.
///
/// # Safety
/// As for `ns_window`.
pub unsafe fn raise(view: usize) {
    if let Some(window) = ns_window(view) {
        let _: () = msg_send![&*window, orderFrontRegardless];
    }
}

/// Moves a path to the Trash, as Finder does. It says whether that worked.
pub fn trash(path: &std::path::Path) -> bool {
    let (Some(manager_class), Some(url_class)) = (AnyClass::get(c"NSFileManager"), AnyClass::get(c"NSURL")) else { return false };
    let text = NSString::from_str(&path.to_string_lossy());
    // SAFETY: class methods with objects that live for the call, and no out values asked for.
    unsafe {
        let manager: Retained<AnyObject> = msg_send![manager_class, defaultManager];
        let url: Retained<AnyObject> = msg_send![url_class, fileURLWithPath: &*text];
        let none = std::ptr::null_mut::<*mut AnyObject>();
        let done: bool = msg_send![&*manager, trashItemAtURL: &*url, resultingItemURL: none, error: none];
        done
    }
}

unsafe fn mouse(view: usize, kind: usize, x: f64, y: f64, clicks: isize) {
    mouse_with(view, kind, x, y, clicks, 0);
}

unsafe fn mouse_with(view: usize, kind: usize, x: f64, y: f64, clicks: isize, flags: usize) {
    let (Some(window), Some(event_class)) = (ns_window(view), AnyClass::get(c"NSEvent")) else { return };
    let content: Retained<AnyObject> = msg_send![&*window, contentView];
    let frame: NSRect = msg_send![&*content, frame];
    let number: isize = msg_send![&*window, windowNumber];
    let location = NSPoint::new(x, frame.size.height - y);
    let event: Option<Retained<AnyObject>> = msg_send![
        event_class,
        mouseEventWithType: kind,
        location: location,
        modifierFlags: flags,
        timestamp: 0.0f64,
        windowNumber: number,
        context: std::ptr::null_mut::<AnyObject>(),
        eventNumber: 0isize,
        clickCount: clicks,
        pressure: 1.0f32
    ];
    if let Some(event) = event {
        let _: () = msg_send![&*window, sendEvent: &*event];
    }
}

/// Clicks at window points, measured from the top left as gpui measures them.
/// `clicks` is 2 for a double click and 3 for a triple click.
///
/// # Safety
/// As for `ns_window`.
pub unsafe fn click(view: usize, x: f64, y: f64, clicks: isize) {
    mouse(view, LEFT_MOUSE_DOWN, x, y, clicks);
    mouse(view, LEFT_MOUSE_UP, x, y, clicks);
}

/// Clicks with control held, which is a right click on a Mac.
///
/// # Safety
/// As for `ns_window`.
pub unsafe fn right_click(view: usize, x: f64, y: f64) {
    mouse_with(view, LEFT_MOUSE_DOWN, x, y, 1, CONTROL);
    mouse_with(view, LEFT_MOUSE_UP, x, y, 1, CONTROL);
}

/// Presses the left button at one point, moves it to another, and lets go.
///
/// # Safety
/// As for `ns_window`.
pub unsafe fn drag(view: usize, from: (f64, f64), to: (f64, f64)) {
    mouse(view, LEFT_MOUSE_DOWN, from.0, from.1, 1);
    let steps = 6;
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        mouse(view, LEFT_MOUSE_DRAGGED, from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, 1);
    }
    mouse(view, LEFT_MOUSE_UP, to.0, to.1, 1);
}

/// Presses a key by its macOS key code, with the text it types.
///
/// # Safety
/// As for `ns_window`.
pub unsafe fn key(view: usize, code: u16, text: &str, flags: usize) {
    let (Some(window), Some(event_class)) = (ns_window(view), AnyClass::get(c"NSEvent")) else { return };
    let number: isize = msg_send![&*window, windowNumber];
    let chars = NSString::from_str(text);
    for kind in [KEY_DOWN, KEY_UP] {
        let event: Option<Retained<AnyObject>> = msg_send![
            event_class,
            keyEventWithType: kind,
            location: NSPoint::new(0.0, 0.0),
            modifierFlags: flags,
            timestamp: 0.0f64,
            windowNumber: number,
            context: std::ptr::null_mut::<AnyObject>(),
            characters: &*chars,
            charactersIgnoringModifiers: &*chars,
            isARepeat: false,
            keyCode: code
        ];
        if let Some(event) = event {
            let _: () = msg_send![&*window, sendEvent: &*event];
        }
    }
}
