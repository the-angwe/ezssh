//! The terminal font list shared by the session dialog and the terminal's
//! Font… picker: "(default)" plus the monospace families installed on this
//! machine (`ezssh_term::fonts::monospace_families`). A fixed list per OS
//! is the fallback when the system scan finds nothing. Users can still type any
//! family in the session dialog; the picker shows "(custom)" for values
//! that are not in the list, without overwriting them.

use std::sync::OnceLock;

pub const MIN_FONT_SIZE: i64 = 8;
pub const MAX_FONT_SIZE: i64 = 48;

/// One fallback preset: stored value (`""` = the default stack) and label.
type FontChoice = (&'static str, &'static str);

const WINDOWS: &[FontChoice] = &[
    ("", "(default)"),
    ("Cascadia Mono", "Cascadia Mono"),
    ("Cascadia Code", "Cascadia Code"),
    ("Consolas", "Consolas"),
    ("Courier New", "Courier New"),
    ("Lucida Console", "Lucida Console"),
    ("Fira Code", "Fira Code"),
    ("JetBrains Mono", "JetBrains Mono"),
    ("MS Gothic", "MS Gothic"),
];

const MACOS: &[FontChoice] = &[
    ("", "(default)"),
    ("SF Mono", "SF Mono"),
    ("Menlo", "Menlo"),
    ("Monaco", "Monaco"),
    ("Andale Mono", "Andale Mono"),
    ("Courier New", "Courier New"),
    ("Fira Code", "Fira Code"),
    ("JetBrains Mono", "JetBrains Mono"),
];

const LINUX: &[FontChoice] = &[
    ("", "(default)"),
    ("DejaVu Sans Mono", "DejaVu Sans Mono"),
    ("Liberation Mono", "Liberation Mono"),
    ("Ubuntu Mono", "Ubuntu Mono"),
    ("Noto Sans Mono", "Noto Sans Mono"),
    ("Source Code Pro", "Source Code Pro"),
    ("Courier 10 Pitch", "Courier 10 Pitch"),
    ("Fira Code", "Fira Code"),
    ("JetBrains Mono", "JetBrains Mono"),
];

/// Fixed presets for the OS this binary runs on.
fn fallback() -> &'static [FontChoice] {
    if cfg!(windows) {
        WINDOWS
    } else if cfg!(target_os = "macos") {
        MACOS
    } else {
        LINUX
    }
}

fn build(installed: &[String]) -> Vec<(String, String)> {
    if installed.is_empty() {
        tracing::warn!("font_presets: system scan found no monospace fonts; offering the fixed presets");
        return fallback().iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    }
    std::iter::once((String::new(), "(default)".to_string())).chain(installed.iter().map(|n| (n.clone(), n.clone()))).collect()
}

/// `(stored value, label)` pairs: "(default)" first, then the installed
/// monospace families. The first call waits for the font scan if
/// `prewarm_monospace_families` has not finished it yet.
pub fn choices() -> &'static [(String, String)] {
    static LIST: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LIST.get_or_init(|| build(ezssh_term::fonts::monospace_families()))
}

/// True when `value` is in [`choices`] (so the picker can show it as
/// selected instead of "(custom)").
pub fn is_preset(value: &str) -> bool {
    choices().iter().any(|(v, _)| v == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_first_and_installed_listed() {
        assert_eq!(choices()[0], (String::new(), "(default)".to_string()));
        assert!(is_preset(""));
        for name in ezssh_term::fonts::monospace_families() {
            assert!(is_preset(name), "{name} installed but not offered");
        }
        assert!(!is_preset("Comic Mono, monospace"));
    }

    #[test]
    fn empty_scan_falls_back_to_presets() {
        let list = build(&[]);
        assert_eq!(list[0], (String::new(), "(default)".to_string()));
        assert!(list.iter().any(|(v, _)| v == "JetBrains Mono"));
        let list = build(&["JetBrains Mono NL".to_string()]);
        assert_eq!(list, vec![(String::new(), "(default)".to_string()), ("JetBrains Mono NL".to_string(), "JetBrains Mono NL".to_string())]);
    }
}
