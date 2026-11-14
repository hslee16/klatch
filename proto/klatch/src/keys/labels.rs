//! The keys of the labels a principal holds (DESIGN section 32 item 8): one independent random key per sealed label,
//! kept in the keystore under its own name, so holding the store key does not give a label's content.

use crate::keys::{Error, Key, KeySource};
use std::collections::HashMap;

/// The keystore name of `label`'s key for the store whose key is named `store_name`.
pub fn label_key_name(store_name: &str, label: &str) -> String {
    // label names are short words; anything outside the key-name alphabet is folded so the name stays safe
    let safe: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{store_name}.label.{safe}")
}

/// The label keys one principal holds. A label it has no key for stays sealed to it.
#[derive(Default)]
pub struct LabelKeys {
    keys: HashMap<String, Key>,
}

impl LabelKeys {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, label: &str, key: Key) -> Self {
        self.keys.insert(label.to_string(), key);
        self
    }

    pub fn get(&self, label: &str) -> Option<&Key> {
        self.keys.get(label)
    }

    pub fn labels(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.keys.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// Load the keys for `labels` that exist in `source`; a label with no key is simply not held.
    pub fn load(source: &dyn KeySource, store_name: &str, labels: &[&str]) -> Result<Self, Error> {
        let mut ring = Self::new();
        for label in labels {
            if let Some(k) = source.get(&label_key_name(store_name, label))? {
                ring.keys.insert((*label).to_string(), k);
            }
        }
        Ok(ring)
    }
}

impl std::fmt::Debug for LabelKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LabelKeys({:?})", self.labels())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::MemoryKeys;

    #[test]
    fn a_label_key_has_its_own_safe_name_per_store_and_label() {
        let a = label_key_name("store-abc", "private");
        assert_eq!(a, "store-abc.label.private");
        assert_ne!(a, label_key_name("store-abc", "secret"));
        assert_ne!(a, label_key_name("store-xyz", "private"));
        assert!(
            label_key_name("s", "we ird/label\n")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        );
    }

    #[test]
    fn loading_returns_only_the_keys_that_exist_and_never_prints_them() {
        let src = MemoryKeys::default();
        src.put(&label_key_name("s", "private"), &Key::from_bytes([5; 32]))
            .unwrap();
        let ring = LabelKeys::load(&src, "s", &["internal", "private", "secret"]).unwrap();
        assert_eq!(ring.labels(), vec!["private"]);
        assert!(ring.get("secret").is_none());
        assert_eq!(format!("{ring:?}"), "LabelKeys([\"private\"])");
    }
}
