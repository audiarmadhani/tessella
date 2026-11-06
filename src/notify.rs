//! Mac notifications and the Dock badge. macOS gives its notification center only to
//! an app with a bundle, so a plain binary, as Homebrew installs, posts through
//! AppleScript instead. Those work, but a click on one cannot come back here.

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{define_class, msg_send, ClassType};
use objc2_foundation::NSString;
use std::sync::{Mutex, Once};

#[link(name = "UserNotifications", kind = "framework")]
extern "C" {}

/// The notifications clicked since last asked, by the id each was posted with.
static CLICKED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PREPARE: Once = Once::new();

const AUTHORIZE_SOUND: usize = 1 << 1;
const AUTHORIZE_ALERT: usize = 1 << 2;
const PRESENT_LIST: usize = 1 << 3;
const PRESENT_BANNER: usize = 1 << 4;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "TessellaNotificationDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &AnyObject, response: &AnyObject, done: &Block<dyn Fn()>) {
            // SAFETY: each of these is a documented getter returning an object.
            let id: Option<Retained<NSString>> = unsafe {
                let notification: Retained<AnyObject> = msg_send![response, notification];
                let request: Retained<AnyObject> = msg_send![&*notification, request];
                msg_send![&*request, identifier]
            };
            if let (Some(id), Ok(mut clicked)) = (id, CLICKED.lock()) {
                clicked.push(id.to_string());
            }
            done.call(());
        }

        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _center: &AnyObject, _notification: &AnyObject, done: &Block<dyn Fn(usize)>) {
            // By default macOS hides a notification while its app is in front. This
            // app decides for itself, since a notice is for a pane not on show.
            done.call((PRESENT_BANNER | PRESENT_LIST,));
        }
    }
);

/// Whether this process is an app with a bundle id.
pub fn is_bundled() -> bool {
    let Some(class) = AnyClass::get(c"NSBundle") else { return false };
    // SAFETY: documented NSBundle methods, returning objects or nil.
    unsafe {
        let bundle: Option<Retained<AnyObject>> = msg_send![class, mainBundle];
        let id: Option<Retained<NSString>> = bundle.and_then(|b| msg_send![&*b, bundleIdentifier]);
        id.is_some()
    }
}

/// The notification center. It raises an exception without a bundle, so check first.
fn center() -> Option<Retained<AnyObject>> {
    if !is_bundled() {
        return None;
    }
    let class = AnyClass::get(c"UNUserNotificationCenter")?;
    // SAFETY: a documented class method, and this process has a bundle.
    unsafe { msg_send![class, currentNotificationCenter] }
}

/// Sets up the center once, which asks the person for permission the first time.
fn prepare(center: &AnyObject) {
    PREPARE.call_once(|| {
        let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::class(), new] };
        // SAFETY: the center keeps its delegate weakly, so this one is never freed.
        unsafe {
            let _: () = msg_send![center, setDelegate: &*delegate];
        }
        std::mem::forget(delegate);
        let answered = RcBlock::new(|_granted: Bool, _error: *mut AnyObject| {});
        // SAFETY: a documented method. The block has the signature it asks for.
        unsafe {
            let _: () = msg_send![center, requestAuthorizationWithOptions: AUTHORIZE_ALERT | AUTHORIZE_SOUND, completionHandler: &*answered];
        }
    });
}

/// Posts a notification. A newer one with the same id replaces it.
pub fn post(id: &str, title: &str, body: &str) {
    if std::env::var_os("TESSELLA_TEST_INPUT").is_some() && std::env::var_os("TESSELLA_TEST_REAL_NOTIFY").is_none() {
        eprintln!("tessella: notice [{id}] {title}: {body}");
        return;
    }
    match center() {
        Some(center) => post_to_center(&center, id, title, body),
        None => post_with_applescript(title, body),
    }
}

fn post_to_center(center: &AnyObject, id: &str, title: &str, body: &str) {
    prepare(center);
    let (Some(content_class), Some(request_class)) = (AnyClass::get(c"UNMutableNotificationContent"), AnyClass::get(c"UNNotificationRequest")) else {
        return;
    };
    // SAFETY: documented methods with these types. A nil trigger delivers at once, and
    // a nil handler is allowed.
    unsafe {
        let content: Retained<AnyObject> = msg_send![content_class, new];
        let _: () = msg_send![&*content, setTitle: &*NSString::from_str(title)];
        let _: () = msg_send![&*content, setBody: &*NSString::from_str(body)];
        let sound_class = AnyClass::get(c"UNNotificationSound");
        if let Some(sound) = sound_class.and_then(|c| -> Option<Retained<AnyObject>> { msg_send![c, defaultSound] }) {
            let _: () = msg_send![&*content, setSound: &*sound];
        }
        let no_trigger: *const AnyObject = std::ptr::null();
        let request: Retained<AnyObject> = msg_send![request_class, requestWithIdentifier: &*NSString::from_str(id), content: &*content, trigger: no_trigger];
        let no_handler: *const Block<dyn Fn(*mut AnyObject)> = std::ptr::null();
        let _: () = msg_send![center, addNotificationRequest: &*request, withCompletionHandler: no_handler];
    }
}

/// The text goes as arguments, so nothing in it can be read as a script.
fn post_with_applescript(title: &str, body: &str) {
    let child = std::process::Command::new("osascript")
        .args(["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", "--", title, body])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || child.wait());
    }
}

/// The ids of notifications clicked since last asked.
pub fn take_clicks() -> Vec<String> {
    CLICKED.lock().map_or_else(|_| Vec::new(), |mut clicked| std::mem::take(&mut clicked))
}

/// Sets the number on the Dock icon. Zero clears it.
pub fn set_badge(count: usize) {
    let Some(class) = AnyClass::get(c"NSApplication") else { return };
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    // SAFETY: documented NSApplication and NSDockTile methods, on the main thread. A
    // nil label clears the badge.
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![class, sharedApplication];
        let tile: Option<Retained<AnyObject>> = app.and_then(|app| msg_send![&*app, dockTile]);
        if let Some(tile) = tile {
            let raw: *const NSString = label.as_deref().map_or(std::ptr::null(), |l| l as *const NSString);
            let _: () = msg_send![&*tile, setBadgeLabel: raw];
        }
    }
}

/// Shows an error in a dialog, for an app started with no terminal to print in.
pub fn alert(text: &str) {
    let Some(class) = AnyClass::get(c"NSAlert") else { return };
    // SAFETY: documented NSAlert methods, on the main thread.
    unsafe {
        let alert: Retained<AnyObject> = msg_send![class, new];
        let _: () = msg_send![&*alert, setMessageText: &*NSString::from_str("Tessella could not start")];
        let _: () = msg_send![&*alert, setInformativeText: &*NSString::from_str(text)];
        let _: isize = msg_send![&*alert, runModal];
    }
}

/// An app opened from the Finder gets the system's short PATH and starts in `/`. A tuios
/// daemon it starts would pass both on to every pane, so it takes them from the login
/// shell and the home folder, as a terminal would.
pub fn adopt_login_environment() {
    let path = merged_path(&login_shell_path().unwrap_or_default(), &std::env::var("PATH").unwrap_or_default());
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe { std::env::set_var("PATH", path) };
    if let Ok(home) = std::env::var("HOME") {
        let _ = std::env::set_current_dir(home);
    }
}

/// PATH as a login shell sets it. A slow or stuck shell start file cannot hold up the
/// app: after a few seconds it goes without.
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut child = std::process::Command::new(shell)
        .args(["-lc", "printf '%s' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    while child.try_wait().ok()?.is_none() {
        if started.elapsed() > std::time::Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut text = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut()?, &mut text).ok()?;
    // A start file can print first, and only the last line is the path.
    text.lines().last().map(str::to_string)
}

/// The login shell's PATH, then what else was set, then where Homebrew installs.
fn merged_path(login: &str, current: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in login.split(':').chain(current.split(':')).chain(["/opt/homebrew/bin", "/usr/local/bin"]) {
        if part.starts_with('/') && !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_keeps_the_login_order_and_finds_homebrew() {
        assert_eq!(merged_path("/a:/b", "/b:/usr/bin"), "/a:/b:/usr/bin:/opt/homebrew/bin:/usr/local/bin");
        assert_eq!(merged_path("", "/usr/bin"), "/usr/bin:/opt/homebrew/bin:/usr/local/bin");
        assert!(!merged_path("not a path:/a", "").contains("not a path"));
    }
}
