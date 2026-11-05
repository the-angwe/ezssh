//! macOS: the application menu winit installs has a Quit item (⌘Q) wired
//! to `terminate:`, which ends the process at once, past the quit
//! confirmation (`sessions.confirmExit`) and the connections' teardown.
//! Re-pointing it at `performClose:` makes ⌘Q a close request for the key
//! window, which goes through `App::handle_close_request` like the
//! title-bar button. ⌘H (hide) and the rest of that menu stay as they are.

use objc2::{MainThreadMarker, sel};
use objc2_app_kit::NSApplication;

pub fn route_quit_through_close() {
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("macos: not on the main thread, ⌘Q stays an immediate quit");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(menu) = app.mainMenu() else {
        tracing::warn!("macos: no main menu, ⌘Q stays an immediate quit");
        return;
    };
    let mut found = 0;
    for item in menu.itemArray().iter() {
        let Some(sub) = item.submenu() else {
            continue;
        };
        for it in sub.itemArray().iter() {
            if it.keyEquivalent().to_string() != "q" {
                continue;
            }
            // SAFETY: `performClose:` is NSWindow's standard action; with a
            // nil target it travels the responder chain to the key window.
            unsafe {
                it.setTarget(None);
                it.setAction(Some(sel!(performClose:)));
            }
            found += 1;
        }
    }
    if found == 0 {
        tracing::warn!("macos: no Quit item in the menu, ⌘Q stays an immediate quit");
    } else {
        tracing::debug!(items = found, "macos: ⌘Q routed through the window close");
    }
}
