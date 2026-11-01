//! `lib/fonts.ts`: the terminal font presets for the running OS, shared by
//! the session dialog and the terminal's Font… picker. Users can still type
//! any family in the session dialog; the picker shows "(custom)" for values
//! that are not a preset, without overwriting them.

pub const MIN_FONT_SIZE: i64 = 8;
pub const MAX_FONT_SIZE: i64 = 48;

/// One preset: stored value (`""` = the default stack) and label.
pub type FontChoice = (&'static str, &'static str);

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

/// Presets for the OS this binary runs on.
pub fn choices() -> &'static [FontChoice] {
    if cfg!(windows) {
        WINDOWS
    } else if cfg!(target_os = "macos") {
        MACOS
    } else {
        LINUX
    }
}

/// True when `value` is one of the presets (so the picker can show it as
/// selected instead of "(custom)").
pub fn is_preset(value: &str) -> bool {
    choices().iter().any(|(v, _)| *v == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_first_and_presets_recognised() {
        assert_eq!(choices()[0], ("", "(default)"));
        assert!(is_preset(""));
        assert!(is_preset("JetBrains Mono"));
        assert!(!is_preset("Comic Mono, monospace"));
    }
}
