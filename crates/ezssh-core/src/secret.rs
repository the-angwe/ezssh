//! A plaintext secret passed into the core (a session password typed in a
//! dialog). Zeroized on drop, never printed by `Debug`, and not
//! serializable, so it cannot leak into logs or saved files.

use serde::{Deserialize, Deserializer};
use zeroize::Zeroizing;

#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(s: String) -> Self {
        Self(Zeroizing::new(s))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() {
            "Secret(empty)"
        } else {
            "Secret(***)"
        })
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value_and_serde_reads_a_string() {
        let s: Secret = serde_json::from_str("\"hunter2\"").unwrap();
        assert_eq!(s.expose(), "hunter2");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert_eq!(format!("{:?}", Secret::default()), "Secret(empty)");
    }
}
