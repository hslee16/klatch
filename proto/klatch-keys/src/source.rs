use crate::{Key, recovery_code};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug)]
pub enum Error {
    /// The keystore could not be reached or refused (the text says why, never a key).
    Store(String),
    /// A stored value is not a key (wrong length, or a file readable by others).
    Bad(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(m) => write!(f, "key store: {m}"),
            Self::Bad(m) => write!(f, "key store: {m}"),
        }
    }
}
impl std::error::Error for Error {}

/// Where named keys are kept. One interface for every backend: create or replace, fetch, remove.
pub trait KeySource {
    /// The key named `name`, or `None` when there is none.
    fn get(&self, name: &str) -> Result<Option<Key>, Error>;
    /// Store `key` as `name`, replacing any earlier one (this is how a key is rotated).
    fn put(&self, name: &str, key: &Key) -> Result<(), Error>;
    /// Remove the key named `name` (revoke it). Removing a missing key is not an error.
    fn delete(&self, name: &str) -> Result<(), Error>;
    /// A short name for this backend, for messages (`keychain`, `file`, ...).
    fn describe(&self) -> &'static str;
}

/// Names are used as file names and keystore labels, so they stay in a small alphabet.
pub(crate) fn check_name(name: &str) -> Result<(), Error> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(Error::Bad(format!("bad key name {name:?}")))
    }
}

/// An in-memory source, for tests.
#[derive(Default)]
pub struct MemoryKeys(RefCell<HashMap<String, [u8; crate::KEY_LEN]>>);

impl KeySource for MemoryKeys {
    fn get(&self, name: &str) -> Result<Option<Key>, Error> {
        check_name(name)?;
        Ok(self.0.borrow().get(name).map(|b| Key::from_bytes(*b)))
    }
    fn put(&self, name: &str, key: &Key) -> Result<(), Error> {
        check_name(name)?;
        self.0.borrow_mut().insert(name.to_string(), *key.expose());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), Error> {
        check_name(name)?;
        self.0.borrow_mut().remove(name);
        Ok(())
    }
    fn describe(&self) -> &'static str {
        "memory"
    }
}

/// Make a new key named `name`, store it, and return it with its recovery code. Replaces an existing key: rotate
/// by calling this and then re-encrypting under the new key.
pub fn create(source: &dyn KeySource, name: &str) -> Result<(Key, String), Error> {
    let key = Key::generate().map_err(|e| Error::Store(format!("no random source: {e}")))?;
    source.put(name, &key)?;
    let code = recovery_code(&key);
    Ok((key, code))
}

/// The key named `name`; if there is none, make one. The recovery code is returned only when a key was made, so a
/// caller can show it exactly once.
pub fn open_or_create(source: &dyn KeySource, name: &str) -> Result<(Key, Option<String>), Error> {
    match source.get(name)? {
        Some(key) => Ok((key, None)),
        None => create(source, name).map(|(k, c)| (k, Some(c))),
    }
}

/// The source this machine uses by default: the keystore of the operating system when there is one, else a 0600
/// file under `dir`. `Ok` carries a warning when the file had to stand in for a keystore.
pub fn default_source(dir: &std::path::Path) -> (Box<dyn KeySource>, Option<String>) {
    #[cfg(target_os = "macos")]
    {
        let _ = dir;
        (Box::new(crate::Keychain::new("klatch")), None)
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(s) = crate::linux::SecretService::detect("klatch") {
            return (Box::new(s), None);
        }
        (
            Box::new(crate::FileKeys::new(dir)),
            Some("no secret service found: the key is kept in a file readable only by you; anyone who can read your files as you can read it".into()),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        (
            Box::new(crate::FileKeys::new(dir)),
            Some("no keystore backend for this system yet: the key is kept in a file readable only by you".into()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_stores_a_key_and_returns_its_recovery_code() {
        let src = MemoryKeys::default();
        let (key, code) = create(&src, "store").unwrap();
        assert_eq!(src.get("store").unwrap().unwrap(), key);
        assert_eq!(crate::restore(&code).unwrap(), key);
    }

    #[test]
    fn open_or_create_shows_the_code_only_the_first_time() {
        let src = MemoryKeys::default();
        let (a, first) = open_or_create(&src, "store").unwrap();
        let (b, second) = open_or_create(&src, "store").unwrap();
        assert!(first.is_some() && second.is_none());
        assert_eq!(a, b);
    }

    #[test]
    fn rotating_replaces_the_key_and_revoking_removes_it() {
        let src = MemoryKeys::default();
        let (old, _) = create(&src, "store").unwrap();
        let (new, _) = create(&src, "store").unwrap();
        assert_ne!(old, new);
        assert_eq!(src.get("store").unwrap().unwrap(), new);
        src.delete("store").unwrap();
        assert!(src.get("store").unwrap().is_none());
        src.delete("store").unwrap();
    }

    #[test]
    fn a_name_outside_the_safe_alphabet_is_refused_by_every_call() {
        let src = MemoryKeys::default();
        let k = Key::from_bytes([1; 32]);
        for bad in ["", "a b", "../x", "a/b", "x\n", &"n".repeat(65)] {
            assert!(
                src.get(bad).is_err() && src.put(bad, &k).is_err() && src.delete(bad).is_err(),
                "{bad:?}"
            );
        }
    }
}
