use crate::keys::{Key, recovery_code};
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug)]
pub enum Error {
    /// The keystore could not be reached or refused (the text says why, never a key).
    Store(String),
    /// The keystore is there but locked, or has no login collection, so it cannot say whether a key exists. This is
    /// never "no key": a caller that read it as missing could make a new key and orphan the store (#561).
    Locked(String),
    /// A stored value is not a key (wrong length, or a file readable by others).
    Bad(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(m) => write!(f, "key store: {m}"),
            Self::Locked(m) => write!(f, "key store: {m}"),
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
pub struct MemoryKeys(RefCell<HashMap<String, [u8; crate::keys::KEY_LEN]>>);

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

/// Keys live in a file readable only by you; a key found only in `legacy` (the keystore of an older install) is
/// copied into the file the first time it is read. A rebuilt unsigned program is a stranger to the macOS Keychain
/// and would make it ask for a password (#480), so new keys never go there; the file is the same protection
/// against other users and stolen disks. Signed releases may use the Keychain again (#371, #424).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct FileFirst {
    file: crate::keys::FileKeys,
    legacy: Box<dyn KeySource>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl FileFirst {
    pub fn new(file: crate::keys::FileKeys, legacy: Box<dyn KeySource>) -> Self {
        Self { file, legacy }
    }
}

impl KeySource for FileFirst {
    fn get(&self, name: &str) -> Result<Option<Key>, Error> {
        if let Some(key) = self.file.get(name)? {
            return Ok(Some(key));
        }
        let Some(key) = self.legacy.get(name)? else {
            return Ok(None);
        };
        self.file.put(name, &key)?;
        Ok(Some(key))
    }
    fn put(&self, name: &str, key: &Key) -> Result<(), Error> {
        self.file.put(name, key)
    }
    fn delete(&self, name: &str) -> Result<(), Error> {
        self.file.delete(name)?;
        // an older keystore copy must not outlive a revoked key, but a stuck keystore must not block the revoke
        let _ = self.legacy.delete(name);
        Ok(())
    }
    fn describe(&self) -> &'static str {
        "key file"
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

/// The name of the key for the store in `home`: one key per store, so two stores on one machine never share a key.
pub fn store_key_name(home: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let canonical = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let d = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let hex: String = d.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("store-{hex}")
}

/// The source this machine uses by default: the keystore of the operating system when there is one, else a 0600
/// file under `dir`. `Ok` carries a warning when the file had to stand in for a keystore. `name` is the key the
/// caller is about to read: the Linux keyring is asked for it once, to find out whether it answers and to read it
/// (#562).
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub fn default_source(dir: &std::path::Path, name: &str) -> (Box<dyn KeySource>, Option<String>) {
    #[cfg(target_os = "macos")]
    {
        (
            Box::new(FileFirst::new(
                crate::keys::FileKeys::new(dir),
                Box::new(crate::keys::Keychain::new("klatch")),
            )),
            None,
        )
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(s) = crate::keys::linux::SecretService::detect(name) {
            return (Box::new(s), None);
        }
        (
            Box::new(crate::keys::FileKeys::new(dir)),
            Some("no secret service found: the key is kept in a file readable only by you; anyone who can read your files as you can read it".into()),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        (
            Box::new(crate::keys::FileKeys::new(dir)),
            Some("no keystore backend for this system yet: the key is kept in a file readable only by you".into()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_first_copies_an_older_keystore_key_into_the_file_once() {
        let dir = crate::store::test_dir::TestDir::new("ff");
        let legacy = MemoryKeys::default();
        let key = Key::from_bytes([9; 32]);
        legacy.put("store", &key).unwrap();
        let src = FileFirst::new(crate::keys::FileKeys::new(&dir), Box::new(legacy));
        assert_eq!(src.get("store").unwrap().unwrap(), key);
        // the file now holds it, so a keystore that is gone or stuck is not asked again
        let only_file = crate::keys::FileKeys::new(&dir);
        assert_eq!(only_file.get("store").unwrap().unwrap(), key);
    }

    /// A keystore that must never be asked: a read of the macOS Keychain by a binary it does not know is the one
    /// slow, prompting call in key custody (#372), so a key the file already holds must never reach it.
    struct NeverAsked;

    impl KeySource for NeverAsked {
        fn get(&self, _name: &str) -> Result<Option<Key>, Error> {
            panic!("the older keystore was asked although the key file holds the key");
        }
        fn put(&self, _name: &str, _key: &Key) -> Result<(), Error> {
            panic!("the older keystore was written");
        }
        fn delete(&self, _name: &str) -> Result<(), Error> {
            Ok(())
        }
        fn describe(&self) -> &'static str {
            "never asked"
        }
    }

    #[test]
    fn file_first_never_asks_the_older_keystore_for_a_key_the_file_holds() {
        let dir = crate::store::test_dir::TestDir::new("ff3");
        let file = crate::keys::FileKeys::new(&dir);
        let key = Key::from_bytes([4; 32]);
        file.put("store", &key).unwrap();
        let src = FileFirst::new(file, Box::new(NeverAsked));
        assert_eq!(src.get("store").unwrap().unwrap(), key);
        assert_eq!(src.get("store").unwrap().unwrap(), key);
    }

    #[test]
    fn file_first_writes_new_keys_to_the_file_not_the_keystore() {
        let dir = crate::store::test_dir::TestDir::new("ff2");
        let src = FileFirst::new(
            crate::keys::FileKeys::new(&dir),
            Box::new(MemoryKeys::default()),
        );
        let (key, _) = create(&src, "store").unwrap();
        assert_eq!(
            crate::keys::FileKeys::new(&dir)
                .get("store")
                .unwrap()
                .unwrap(),
            key
        );
        src.delete("store").unwrap();
        assert!(src.get("store").unwrap().is_none());
    }

    #[test]
    fn create_stores_a_key_and_returns_its_recovery_code() {
        let src = MemoryKeys::default();
        let (key, code) = create(&src, "store").unwrap();
        assert_eq!(src.get("store").unwrap().unwrap(), key);
        assert_eq!(crate::keys::restore(&code).unwrap(), key);
    }

    #[test]
    fn open_or_create_shows_the_code_only_the_first_time() {
        let src = MemoryKeys::default();
        let (a, first) = open_or_create(&src, "store").unwrap();
        let (b, second) = open_or_create(&src, "store").unwrap();
        assert!(first.is_some() && second.is_none());
        assert_eq!(a, b);
    }

    /// A keystore that is locked: it cannot say whether a key exists, and it records any key put into it.
    struct LockedKeys(std::cell::Cell<bool>);

    impl KeySource for LockedKeys {
        fn get(&self, _name: &str) -> Result<Option<Key>, Error> {
            Err(Error::Locked("the keyring is locked".into()))
        }
        fn put(&self, _name: &str, _key: &Key) -> Result<(), Error> {
            self.0.set(true);
            Ok(())
        }
        fn delete(&self, _name: &str) -> Result<(), Error> {
            Ok(())
        }
        fn describe(&self) -> &'static str {
            "locked"
        }
    }

    #[test]
    fn a_locked_keystore_never_leads_to_a_new_key_being_made() {
        let src = LockedKeys(std::cell::Cell::new(false));
        assert!(matches!(
            open_or_create(&src, "store"),
            Err(Error::Locked(_))
        ));
        assert!(!src.0.get(), "a key was made while the keystore was locked");
    }

    #[test]
    fn each_store_directory_has_its_own_key_name_and_the_name_is_safe() {
        let a = store_key_name(std::path::Path::new("/tmp/klatch-a"));
        let b = store_key_name(std::path::Path::new("/tmp/klatch-b"));
        assert_ne!(a, b);
        assert_eq!(a, store_key_name(std::path::Path::new("/tmp/klatch-a")));
        assert!(check_name(&a).is_ok(), "{a}");
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
