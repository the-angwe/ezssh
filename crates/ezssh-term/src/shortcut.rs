//! Shortcut labels for tooltips and menus: "Ctrl+K" on Linux and Windows,
//! "⌘K" on macOS. Only what the UI prints lives here; the key events are
//! matched where they are handled (`view::handle_keyboard`, the app's
//! `global_shortcuts`).

/// Built for macOS, where the Command key carries the shortcuts Ctrl has
/// elsewhere (Ctrl+letter stays a control character for the shell).
pub const IS_MAC: bool = cfg!(target_os = "macos");

/// Ctrl+`key` / ⌘`key`.
pub fn cmd(key: &str) -> String {
    if IS_MAC {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// Ctrl+Shift+`key` / ⇧⌘`key`.
pub fn cmd_shift(key: &str) -> String {
    if IS_MAC {
        format!("⇧⌘{key}")
    } else {
        format!("Ctrl+Shift+{key}")
    }
}

/// The terminal's copy shortcut.
pub const COPY: &str = if IS_MAC { "⌘C" } else { "Ctrl+Shift+C" };
/// The terminal's paste shortcut (Ctrl+Shift+V works too outside macOS).
pub const PASTE: &str = if IS_MAC { "⌘V" } else { "Shift+Insert" };
/// The terminal's find shortcut.
pub const FIND: &str = if IS_MAC { "⌘F" } else { "Ctrl+Shift+F" };
