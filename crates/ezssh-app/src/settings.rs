//! App-wide preferences, a port of `ui/lib/settings.ts`: one flat map of
//! dotted keys, each with a typed spec (default, kind, bounds). Values live
//! in the SQLite `app_settings` table as strings, are loaded once at unlock
//! and written through on every change (no Save button).
//!
//! Per-session values (scrollback, font, keepalive, …) win over the
//! `terminal.*` / `sessions.*` defaults here; those only seed the session
//! dialog for new sessions.

use std::collections::HashMap;

/// Typed value of one setting.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Str(String),
}

impl Value {
    pub fn serialize(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Str(s) => s.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Bool { default: bool },
    Int { default: i64, min: i64, max: i64 },
    Str { default: &'static str, max_len: usize },
    Enum { default: &'static str, values: &'static [&'static str] },
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub key: &'static str,
    pub kind: Kind,
}

impl Spec {
    pub fn default_value(&self) -> Value {
        match self.kind {
            Kind::Bool { default } => Value::Bool(default),
            Kind::Int { default, .. } => Value::Int(default),
            Kind::Str { default, .. } | Kind::Enum { default, .. } => Value::Str(default.to_string()),
        }
    }

    /// Parse a stored string. Anything out of range or unrecognised falls
    /// back to the default: the DB is user-editable and schemas drift.
    pub fn parse(&self, raw: Option<&str>) -> Value {
        let Some(raw) = raw else {
            return self.default_value();
        };
        match self.kind {
            Kind::Bool { default } => Value::Bool(match raw {
                "true" => true,
                "false" => false,
                _ => default,
            }),
            Kind::Int { default, min, max } => Value::Int(match raw.parse::<i64>() {
                Ok(n) if (min..=max).contains(&n) => n,
                _ => default,
            }),
            Kind::Str { default, max_len } => {
                Value::Str(if raw.chars().count() <= max_len { raw.to_string() } else { default.to_string() })
            }
            Kind::Enum { default, values } => {
                Value::Str(if values.contains(&raw) { raw.to_string() } else { default.to_string() })
            }
        }
    }

    /// Clamp / validate an in-app value before storing. `None` when it is
    /// not representable for this key (wrong type, unknown enum value).
    pub fn normalize(&self, value: Value) -> Option<Value> {
        match (self.kind, value) {
            (Kind::Bool { .. }, v @ Value::Bool(_)) => Some(v),
            (Kind::Int { min, max, .. }, Value::Int(n)) => Some(Value::Int(n.clamp(min, max))),
            (Kind::Str { max_len, .. }, Value::Str(s)) => Some(Value::Str(s.chars().take(max_len).collect())),
            (Kind::Enum { values, .. }, Value::Str(s)) => values.contains(&s.as_str()).then_some(Value::Str(s)),
            _ => None,
        }
    }
}

pub const SPEC: &[Spec] = &[
    Spec { key: "terminal.scrollback", kind: Kind::Int { default: 5000, min: 0, max: 100_000 } },
    Spec { key: "terminal.fontSize", kind: Kind::Int { default: 13, min: 8, max: 48 } },
    Spec { key: "terminal.fontFamily", kind: Kind::Str { default: "", max_len: 200 } },
    Spec { key: "terminal.cursorStyle", kind: Kind::Enum { default: "block", values: &["block", "underline", "bar"] } },
    Spec { key: "terminal.cursorBlink", kind: Kind::Bool { default: true } },
    Spec { key: "terminal.copyOnSelect", kind: Kind::Bool { default: false } },
    Spec { key: "terminal.rightClick", kind: Kind::Enum { default: "menu", values: &["menu", "paste"] } },
    Spec { key: "terminal.warnMultilinePaste", kind: Kind::Bool { default: true } },
    Spec { key: "terminal.osc52Clipboard", kind: Kind::Bool { default: true } },
    Spec { key: "terminal.bell", kind: Kind::Enum { default: "none", values: &["none", "visual", "sound"] } },
    Spec { key: "sessions.confirmCloseLive", kind: Kind::Bool { default: true } },
    Spec { key: "sessions.confirmExit", kind: Kind::Bool { default: true } },
    Spec { key: "sessions.autoOpenSftp", kind: Kind::Bool { default: false } },
    Spec { key: "sessions.keepaliveSecs", kind: Kind::Int { default: 60, min: 0, max: 7200 } },
    Spec { key: "sessions.connectTimeoutSecs", kind: Kind::Int { default: 15, min: 1, max: 600 } },
    Spec { key: "appearance.tabLabel", kind: Kind::Enum { default: "name", values: &["name", "userhost"] } },
    Spec { key: "terminal.palette", kind: Kind::Enum { default: ezssh_term::palette::DEFAULT_PALETTE, values: PALETTE_IDS } },
    Spec { key: "updates.checkOnLaunch", kind: Kind::Bool { default: true } },
    Spec { key: "updates.preRelease", kind: Kind::Bool { default: false } },
    Spec { key: "updates.intervalDays", kind: Kind::Int { default: 30, min: 1, max: 365 } },
    Spec { key: "security.idleLockMinutes", kind: Kind::Int { default: 0, min: 0, max: 1440 } },
];

/// Palette ids in `ezssh_term::palette::PALETTES` order.
const PALETTE_IDS: &[&str] = &{
    let mut ids = [""; ezssh_term::palette::PALETTES.len()];
    let mut i = 0;
    while i < ids.len() {
        ids[i] = ezssh_term::palette::PALETTES[i].id;
        i += 1;
    }
    ids
};

pub fn spec(key: &str) -> Option<&'static Spec> {
    SPEC.iter().find(|s| s.key == key)
}

/// The hydrated store. Every key of [`SPEC`] always has a value.
#[derive(Clone, Debug)]
pub struct Settings {
    values: HashMap<&'static str, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { values: SPEC.iter().map(|s| (s.key, s.default_value())).collect() }
    }
}

impl Settings {
    /// Build from the stored rows (`settings_list`). Unknown keys (e.g.
    /// `theme`, sync state) are ignored.
    pub fn from_rows(rows: &[(String, String)]) -> Self {
        let stored: HashMap<&str, &str> = rows.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        Self { values: SPEC.iter().map(|s| (s.key, s.parse(stored.get(s.key).copied()))).collect() }
    }

    pub fn get(&self, key: &str) -> &Value {
        match self.values.get(key) {
            Some(v) => v,
            None => panic!("settings: unknown key {key:?} (not in SPEC)"),
        }
    }

    pub fn bool(&self, key: &str) -> bool {
        match self.get(key) {
            Value::Bool(b) => *b,
            other => panic!("settings: {key} is {other:?}, not a bool"),
        }
    }

    pub fn int(&self, key: &str) -> i64 {
        match self.get(key) {
            Value::Int(n) => *n,
            other => panic!("settings: {key} is {other:?}, not an int"),
        }
    }

    pub fn str(&self, key: &str) -> &str {
        match self.get(key) {
            Value::Str(s) => s,
            other => panic!("settings: {key} is {other:?}, not a string"),
        }
    }

    /// Validate and store. Returns the previous and the stored value when
    /// something changed, `None` when the value was rejected or equal.
    pub fn set(&mut self, key: &str, value: Value) -> Option<(Value, Value)> {
        let Some(spec) = spec(key) else {
            tracing::error!(key, "settings: set of unknown key ignored");
            return None;
        };
        let Some(clean) = spec.normalize(value.clone()) else {
            tracing::warn!(key, ?value, "settings: value rejected");
            return None;
        };
        let prev = self.values.insert(spec.key, clean.clone()).unwrap_or_else(|| spec.default_value());
        (prev != clean).then_some((prev, clean))
    }

    /// Put `prev` back if `key` still holds `expected` (a write failed and
    /// nothing newer landed on the key meanwhile).
    pub fn rollback(&mut self, key: &str, expected: &Value, prev: Value) -> bool {
        match spec(key) {
            Some(spec) if self.values.get(spec.key) == Some(expected) => {
                self.values.insert(spec.key, prev);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(key: &str, raw: Option<&str>) -> Value {
        spec(key).unwrap().parse(raw)
    }

    #[test]
    fn falls_back_to_the_default_for_missing_values() {
        assert_eq!(parse("terminal.scrollback", None), Value::Int(5000));
        assert_eq!(parse("terminal.bell", None), Value::Str("none".into()));
    }

    #[test]
    fn parses_booleans_strictly() {
        assert_eq!(parse("terminal.copyOnSelect", Some("true")), Value::Bool(true));
        assert_eq!(parse("terminal.copyOnSelect", Some("false")), Value::Bool(false));
        assert_eq!(parse("terminal.copyOnSelect", Some("yes")), Value::Bool(false));
        assert_eq!(parse("terminal.osc52Clipboard", Some("0")), Value::Bool(true));
    }

    #[test]
    fn rejects_out_of_range_or_non_integer_numbers() {
        assert_eq!(parse("terminal.fontSize", Some("14")), Value::Int(14));
        assert_eq!(parse("terminal.fontSize", Some("4")), Value::Int(13));
        assert_eq!(parse("terminal.fontSize", Some("14.5")), Value::Int(13));
        assert_eq!(parse("terminal.fontSize", Some("big")), Value::Int(13));
    }

    #[test]
    fn rejects_unknown_enum_values() {
        assert_eq!(parse("terminal.rightClick", Some("paste")), Value::Str("paste".into()));
        assert_eq!(parse("terminal.rightClick", Some("middle")), Value::Str("menu".into()));
        assert_eq!(parse("terminal.palette", Some("no-such-palette")), Value::Str(ezssh_term::palette::DEFAULT_PALETTE.into()));
    }

    #[test]
    fn normalize_clamps_integers() {
        let s = spec("security.idleLockMinutes").unwrap();
        assert_eq!(s.normalize(Value::Int(99_999)), Some(Value::Int(1440)));
        assert_eq!(s.normalize(Value::Int(-3)), Some(Value::Int(0)));
    }

    #[test]
    fn normalize_returns_none_for_unrepresentable_input() {
        assert_eq!(spec("terminal.fontSize").unwrap().normalize(Value::Str("x".into())), None);
        assert_eq!(spec("terminal.bell").unwrap().normalize(Value::Str("loud".into())), None);
    }

    #[test]
    fn normalize_truncates_over_long_strings() {
        let long = "x".repeat(500);
        assert_eq!(spec("terminal.fontFamily").unwrap().normalize(Value::Str(long)), Some(Value::Str("x".repeat(200))));
    }

    #[test]
    fn every_key_has_a_default_and_the_store_mirrors_it() {
        let s = Settings::default();
        for spec in SPEC {
            assert_eq!(s.get(spec.key), &spec.default_value(), "{}", spec.key);
        }
        // Same key set as settings.ts (21 keys).
        assert_eq!(SPEC.len(), 21);
    }

    #[test]
    fn palette_ids_match_the_palette_table() {
        assert_eq!(PALETTE_IDS.len(), ezssh_term::palette::PALETTES.len());
        assert!(PALETTE_IDS.contains(&ezssh_term::palette::DEFAULT_PALETTE));
    }

    #[test]
    fn from_rows_parses_and_ignores_unknown_keys() {
        let rows = vec![
            ("terminal.bell".to_string(), "sound".to_string()),
            ("theme".to_string(), "light".to_string()),
            ("terminal.fontSize".to_string(), "99".to_string()),
        ];
        let s = Settings::from_rows(&rows);
        assert_eq!(s.str("terminal.bell"), "sound");
        assert_eq!(s.int("terminal.fontSize"), 13);
    }

    #[test]
    fn set_reports_changes_and_rollback_respects_newer_writes() {
        let mut s = Settings::default();
        assert_eq!(s.set("terminal.bell", Value::Str("visual".into())), Some((Value::Str("none".into()), Value::Str("visual".into()))));
        assert_eq!(s.set("terminal.bell", Value::Str("visual".into())), None, "equal value is no change");
        assert_eq!(s.set("terminal.bell", Value::Str("loud".into())), None, "rejected");
        assert_eq!(s.set("no.such.key", Value::Bool(true)), None);
        // A failed write rolls back only if nothing newer landed.
        s.set("terminal.bell", Value::Str("sound".into()));
        assert!(!s.rollback("terminal.bell", &Value::Str("visual".into()), Value::Str("none".into())));
        assert_eq!(s.str("terminal.bell"), "sound");
        assert!(s.rollback("terminal.bell", &Value::Str("sound".into()), Value::Str("none".into())));
        assert_eq!(s.str("terminal.bell"), "none");
    }
}
