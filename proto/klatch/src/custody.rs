//! The store's key, from the keystore (epic #152, #155): find it, make it on first use, convert a plaintext
//! database, and open the store. The key is never printed here, and a hook never prints a recovery code: showing it
//! is `klatch admin recovery`, behind the terminal gate.

use anyhow::{Result, bail};
use klatch::keys::{Key, KeySource};
use klatch::store::SqliteStore;
use klatch::store::encrypt::{ensure_encrypted, is_plaintext};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// Where this store's key lives and under what name.
pub(crate) struct KeyHome {
    pub source: Box<dyn KeySource>,
    pub name: String,
    pub warning: Option<String>,
}

/// A directory for key files when there is no keystore: `$XDG_STATE_HOME/klatch/keys`, else
/// `$HOME/.local/state/klatch/keys`. Never beside the data.
fn state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("klatch").join("keys")
}

/// The key source for the store at `db`. `KLATCH_KEY_DIR` names a directory of key files: how an unattended worker
/// is keyed (the service manager or the parent hands over a path, never the key) and how the tests are. Otherwise
/// the operating-system keystore, with a key file as the stand-in where there is none.
pub(crate) fn key_home(db: &Path) -> KeyHome {
    let home = db.parent().unwrap_or(Path::new("."));
    let name = klatch::keys::store_key_name(home);
    // A worker is handed the path of its key bundle and reads no keystore: its keys are the ones the launcher chose.
    if let Some(path) = std::env::var_os("KLATCH_IDENTITY_FILE") {
        let source: Box<dyn KeySource> = match klatch::keys::Bundle::read(Path::new(&path)) {
            Ok(bundle) => Box::new(bundle.into_source(&name)),
            Err(e) => Box::new(BrokenKeys(e.to_string())),
        };
        return KeyHome {
            source,
            name,
            warning: None,
        };
    }
    if let Some(dir) = std::env::var_os("KLATCH_KEY_DIR") {
        return KeyHome {
            source: Box::new(klatch::keys::FileKeys::new(Path::new(&dir))),
            name,
            warning: None,
        };
    }
    let (source, warning) = klatch::keys::default_source(&state_dir());
    KeyHome {
        source,
        name,
        warning,
    }
}

/// What stands in for the key source when the key bundle cannot be used: every call says why.
struct BrokenKeys(String);

impl KeySource for BrokenKeys {
    fn get(&self, _name: &str) -> Result<Option<Key>, klatch::keys::Error> {
        Err(klatch::keys::Error::Bad(self.0.clone()))
    }
    fn put(&self, _name: &str, _key: &Key) -> Result<(), klatch::keys::Error> {
        Err(klatch::keys::Error::Bad(self.0.clone()))
    }
    fn delete(&self, _name: &str) -> Result<(), klatch::keys::Error> {
        Err(klatch::keys::Error::Bad(self.0.clone()))
    }
    fn describe(&self) -> &'static str {
        "key bundle"
    }
}

fn next_name(name: &str) -> String {
    format!("{name}-next")
}

/// A store file that holds data (an empty file is a database that was never written).
fn has_data(db: &Path) -> bool {
    std::fs::metadata(db).is_ok_and(|m| m.len() > 0)
}

/// Open the encrypted store at `db`: get its key (making one for a new store, never for an existing encrypted store
/// whose key is missing), convert a plaintext database first, and finish a rotation a crash interrupted.
pub(crate) fn open_store(db: &Path) -> Result<SqliteStore> {
    open_store_in(db, &key_home(db))
}

/// Make the key of a store that has none, as one step with making the store. The directory lock is taken before
/// anything is decided and, for a store that does not exist yet, handed back so the caller keeps it until the store
/// file exists. Without that, a process that looked before the key was made and checked the file after it was
/// made saw an encrypted store with no key and called it lost (#596). Many processes starting at once each make
/// no key of their own: the others wait here, look again, and find it.
fn make_key_under_lock(db: &Path, kh: &KeyHome) -> Result<(Key, bool, Option<std::fs::File>)> {
    let lock = klatch::store::encrypt::lock_dir(db)?;
    // another process may have made both the key and the store while this one waited for the lock
    if let Some(k) = kh.source.get(&kh.name)? {
        return Ok((k, false, None));
    }
    if has_data(db) && !is_plaintext(db) {
        bail!(
            "the store's key is missing from the {}; `klatch admin recover <recovery code>` restores it",
            kh.source.describe()
        );
    }
    let key = klatch::keys::create(kh.source.as_ref(), &kh.name)?.0;
    // A plaintext database is converted by `ensure_encrypted`, which takes this lock itself, so the lock is kept
    // only when the store file is about to be created fresh.
    let keep = (!db.exists()).then_some(lock);
    Ok((key, true, keep))
}

fn open_store_in(db: &Path, kh: &KeyHome) -> Result<SqliteStore> {
    let (key, created, guard) = match kh.source.get(&kh.name)? {
        Some(k) => (k, false, None),
        None => make_key_under_lock(db, kh)?,
    };
    // `ensure_encrypted` locks the directory itself, and there is nothing to convert in a store made just now
    if guard.is_none() {
        ensure_encrypted(db, &key)?;
    }
    let store = open_keyed(db, kh, key)?;
    drop(guard);
    if created && std::io::stderr().is_terminal() {
        eprintln!(
            "klatch: made an encryption key for this store in the {}; `klatch admin recovery` shows the code that restores it",
            kh.source.describe()
        );
    }
    if let Some(w) = kh
        .warning
        .as_ref()
        .filter(|_| std::io::stderr().is_terminal())
    {
        eprintln!("klatch: {w}");
    }
    Ok(store)
}

/// Open the store with `key`. The caller holds the directory lock.
fn open_keyed(db: &Path, kh: &KeyHome, key: Key) -> Result<SqliteStore> {
    match SqliteStore::open(db, &key) {
        Ok(s) => Ok(s),
        Err(first) => {
            // a rotation that died after the database was re-keyed but before the key was promoted
            match kh.source.get(&next_name(&kh.name))? {
                Some(next) if next != key => {
                    let s = SqliteStore::open(db, &next).map_err(|_| first)?;
                    kh.source.put(&kh.name, &next)?;
                    kh.source.delete(&next_name(&kh.name))?;
                    Ok(s)
                }
                _ => Err(first.into()),
            }
        }
    }
}

/// The current key, if there is one (for showing its recovery code).
pub(crate) fn current_key(db: &Path) -> Result<Option<Key>> {
    let kh = key_home(db);
    Ok(kh.source.get(&kh.name)?)
}

/// Put `key` in as the store's key, after checking that it opens the store when one exists.
pub(crate) fn restore_key(db: &Path, key: &Key) -> Result<()> {
    let kh = key_home(db);
    if has_data(db) && !is_plaintext(db) && SqliteStore::open(db, key).is_err() {
        bail!("that recovery code does not open this store; nothing was changed");
    }
    if let Some(existing) = kh.source.get(&kh.name)?
        && existing != *key
    {
        bail!(
            "this store already has a different key; use `klatch admin rotate-key` to change it, or remove the old key from the {} first",
            kh.source.describe()
        );
    }
    kh.source.put(&kh.name, key)?;
    Ok(())
}

/// Re-encrypt the store and every archive file under a new key and make that the stored key. The order survives a
/// crash at any step (#628): the archives are checked first and the rotation refused, nothing changed, if one opens
/// with neither key; the new key is stored under a second name; each archive is re-keyed through a temp file (so at
/// every moment each holds a whole archive under one of the two keys); the store is re-keyed; then the new key takes
/// the main name and the second one goes. `open_store` finishes a run that died after the store was re-keyed, and a
/// repeated `rotate-key` finishes one that died before: it reuses the stored second key and skips the archives
/// already moved. Returns the new key.
pub(crate) fn rotate_key(db: &Path) -> Result<Key> {
    rotate_key_in(db, &key_home(db))
}

fn rotate_key_in(db: &Path, kh: &KeyHome) -> Result<Key> {
    // makes the key of a new store, converts a plaintext one, finishes a rotation that died after the re-key
    let store = open_store_in(db, kh)?;
    let old = kh
        .source
        .get(&kh.name)?
        .ok_or_else(|| anyhow::anyhow!("this store has no key to rotate"))?;
    let archives = db.with_file_name("archive");
    let next = kh.source.get(&next_name(&kh.name))?.filter(|n| *n != old);
    let new = match next {
        Some(n) => n,
        None => Key::generate().map_err(|e| anyhow::anyhow!("no random source: {e}"))?,
    };
    klatch::store::archives_to_rekey(&archives, &old, &new)?;
    kh.source.put(&next_name(&kh.name), &new)?;
    klatch::store::rekey_archives(&archives, &old, &new)?;
    store.rekey(&new)?;
    kh.source.put(&kh.name, &new)?;
    kh.source.delete(&next_name(&kh.name))?;
    Ok(new)
}

/// The keys of `labels` this store has in its key source; a label with no key there is not held.
pub(crate) fn label_ring(db: &Path, labels: &[String]) -> Result<klatch::keys::LabelKeys> {
    let kh = key_home(db);
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    Ok(klatch::keys::LabelKeys::load(
        kh.source.as_ref(),
        &kh.name,
        &names,
    )?)
}

/// Make the key of a sealed label if there is none. Returns whether one was made. Under the directory lock, so two
/// processes sealing at once make one key.
pub(crate) fn ensure_label_key(db: &Path, label: &str) -> Result<bool> {
    let kh = key_home(db);
    let name = klatch::keys::label_key_name(&kh.name, label);
    let _lock = klatch::store::encrypt::lock_dir(db)?;
    if kh.source.get(&name)?.is_some() {
        return Ok(false);
    }
    klatch::keys::create(kh.source.as_ref(), &name)?;
    Ok(true)
}

/// The key of a sealed label, for showing its recovery code.
pub(crate) fn label_key(db: &Path, label: &str) -> Result<Option<Key>> {
    let kh = key_home(db);
    Ok(kh
        .source
        .get(&klatch::keys::label_key_name(&kh.name, label))?)
}

/// Put a sealed label's key back from its recovery code. It is refused if a different key is already there.
pub(crate) fn restore_label_key(db: &Path, label: &str, key: &Key) -> Result<()> {
    let kh = key_home(db);
    let name = klatch::keys::label_key_name(&kh.name, label);
    if let Some(existing) = kh.source.get(&name)?
        && existing != *key
    {
        bail!(
            "label {label} already has a different key; remove it from the {} first",
            kh.source.describe()
        );
    }
    kh.source.put(&name, key)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Condvar, Mutex};

    /// A file key source whose first `get` reports what it found and then stalls until told to go: the moment in
    /// #596 when one process has looked and found no key while another makes the key and the store.
    struct Stalled {
        inner: klatch::keys::FileKeys,
        state: Arc<(Mutex<(bool, bool)>, Condvar)>, // (first get done, released)
        first: std::sync::atomic::AtomicBool,
    }

    impl KeySource for Stalled {
        fn get(&self, name: &str) -> Result<Option<Key>, klatch::keys::Error> {
            let found = self.inner.get(name)?;
            if !self.first.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let (m, cv) = &*self.state;
                let mut g = m.lock().unwrap();
                g.0 = true;
                cv.notify_all();
                let _ = cv
                    .wait_timeout_while(g, std::time::Duration::from_secs(20), |g| !g.1)
                    .unwrap();
            }
            Ok(found)
        }
        fn put(&self, name: &str, key: &Key) -> Result<(), klatch::keys::Error> {
            self.inner.put(name, key)
        }
        fn delete(&self, name: &str) -> Result<(), klatch::keys::Error> {
            self.inner.delete(name)
        }
        fn describe(&self) -> &'static str {
            "file"
        }
    }

    fn home(source: Box<dyn KeySource>) -> KeyHome {
        KeyHome {
            source,
            name: "store-test".into(),
            warning: None,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("klatch-custody-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_process_that_looked_before_the_key_was_made_still_opens_the_new_store() {
        let d = scratch("race");
        let db = d.join("klatch.db");
        let state = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let late = {
            let (db, keys, state) = (db.clone(), d.join("keys"), state.clone());
            std::thread::spawn(move || {
                let src = Stalled {
                    inner: klatch::keys::FileKeys::new(&keys),
                    state,
                    first: false.into(),
                };
                open_store_in(&db, &home(Box::new(src))).map(|_| ())
            })
        };
        // wait until the late process has looked and found no key
        {
            let (m, cv) = &*state;
            drop(cv.wait_while(m.lock().unwrap(), |g| !g.0).unwrap());
        }
        let keys = klatch::keys::FileKeys::new(&d.join("keys"));
        open_store_in(&db, &home(Box::new(keys))).unwrap();
        {
            let (m, cv) = &*state;
            m.lock().unwrap().1 = true;
            cv.notify_all();
        }
        let r = late.join().unwrap();
        assert!(r.is_ok(), "{:?}", r.err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_encrypted_store_without_a_key_still_says_how_to_recover() {
        let d = scratch("lost");
        let db = d.join("klatch.db");
        let keys = klatch::keys::FileKeys::new(&d.join("keys"));
        open_store_in(&db, &home(Box::new(keys))).unwrap();
        std::fs::remove_dir_all(d.join("keys")).unwrap();
        let keys = klatch::keys::FileKeys::new(&d.join("keys"));
        let err = open_store_in(&db, &home(Box::new(keys)))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("admin recover"), "{err}");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn file_home(d: &Path) -> KeyHome {
        home(Box::new(klatch::keys::FileKeys::new(&d.join("keys"))))
    }

    /// A store in `d` with one archive file of an old message, made under the store's current key.
    fn store_with_archive(d: &Path) -> PathBuf {
        let db = d.join("klatch.db");
        let kh = file_home(d);
        let store = open_store_in(&db, &kh).unwrap();
        let key = kh.source.get(&kh.name).unwrap().unwrap();
        klatch::store::Store::join(&store, "r", "alice", 1).unwrap();
        let seat = klatch::store::Seat::new("r", "alice");
        klatch::store::Store::post(&store, seat, "old words", 10).unwrap();
        let dir = d.join("archive");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("r-100.db");
        store.archive_before("r", 500, &file, &key).unwrap();
        file
    }

    #[test]
    fn rotating_the_key_moves_the_archives_to_the_new_key_so_they_still_restore() {
        let d = scratch("rot-archive");
        let file = store_with_archive(&d);
        let kh = file_home(&d);
        let new = rotate_key_in(&d.join("klatch.db"), &kh).unwrap();
        let listed = klatch::store::list_archives(&d.join("archive"), &new).unwrap();
        assert!(
            matches!(listed[0].state, klatch::store::ArchiveState::Ok { .. }),
            "{:?}",
            listed[0].state
        );
        let store = open_store_in(&d.join("klatch.db"), &kh).unwrap();
        assert_eq!(store.restore_archive(&file, &new).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rotating_with_an_archive_that_does_not_open_refuses_and_changes_nothing() {
        let d = scratch("rot-refuse");
        let file = store_with_archive(&d);
        std::fs::copy(&file, file.with_file_name("r-200.db")).unwrap();
        let strange = Key::generate().unwrap();
        let old = file_home(&d).source.get("store-test").unwrap().unwrap();
        klatch::store::rekey_archives(&d.join("archive"), &old, &strange).unwrap();
        // both files are now under a third key: neither the current nor a new key opens them
        let kh = file_home(&d);
        let err = rotate_key_in(&d.join("klatch.db"), &kh)
            .err()
            .unwrap()
            .to_string();
        assert!(
            err.contains("r-100.db") && err.contains("not changed"),
            "{err}"
        );
        let still = kh.source.get("store-test").unwrap().unwrap();
        assert!(still == old, "the store key changed");
        assert!(kh.source.get("store-test-next").unwrap().is_none());
        assert!(open_store_in(&d.join("klatch.db"), &kh).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
