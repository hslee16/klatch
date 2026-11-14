//! The store's key, from the keystore (epic #152, #155): find it, make it on first use, convert a plaintext
//! database, and open the store. The key is never printed here, and a hook never prints a recovery code: showing it
//! is `klatch admin recovery`, behind the terminal gate.

use anyhow::{Result, bail};
use klatch_keys::{Key, KeySource};
use klatch_store::SqliteStore;
use klatch_store::encrypt::{ensure_encrypted, is_plaintext};
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
    let name = klatch_keys::store_key_name(home);
    // A worker is handed the path of its key bundle and reads no keystore: its keys are the ones the launcher chose.
    if let Some(path) = std::env::var_os("KLATCH_IDENTITY_FILE") {
        let source: Box<dyn KeySource> = match klatch_keys::Bundle::read(Path::new(&path)) {
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
            source: Box::new(klatch_keys::FileKeys::new(Path::new(&dir))),
            name,
            warning: None,
        };
    }
    let (source, warning) = klatch_keys::default_source(&state_dir());
    KeyHome {
        source,
        name,
        warning,
    }
}

/// What stands in for the key source when the key bundle cannot be used: every call says why.
struct BrokenKeys(String);

impl KeySource for BrokenKeys {
    fn get(&self, _name: &str) -> Result<Option<Key>, klatch_keys::Error> {
        Err(klatch_keys::Error::Bad(self.0.clone()))
    }
    fn put(&self, _name: &str, _key: &Key) -> Result<(), klatch_keys::Error> {
        Err(klatch_keys::Error::Bad(self.0.clone()))
    }
    fn delete(&self, _name: &str) -> Result<(), klatch_keys::Error> {
        Err(klatch_keys::Error::Bad(self.0.clone()))
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
    let kh = key_home(db);
    let (key, created) = match kh.source.get(&kh.name)? {
        Some(k) => (k, false),
        None => {
            if has_data(db) && !is_plaintext(db) {
                bail!(
                    "the store's key is missing from the {}; `klatch admin recover <recovery code>` restores it",
                    kh.source.describe()
                );
            }
            // Many processes can start on a new store at once. Whoever holds the directory lock makes the key; the
            // others wait, look again, and find it. Without this each would make its own and all but one would be
            // locked out of the store.
            let _lock = klatch_store::encrypt::lock_dir(db)?;
            match kh.source.get(&kh.name)? {
                Some(k) => (k, false),
                None => (klatch_keys::create(kh.source.as_ref(), &kh.name)?.0, true),
            }
        }
    };
    ensure_encrypted(db, &key)?;
    let store = match SqliteStore::open(db, &key) {
        Ok(s) => s,
        Err(first) => {
            // a rotation that died after the database was re-keyed but before the key was promoted
            match kh.source.get(&next_name(&kh.name))? {
                Some(next) if next != key => {
                    let s = SqliteStore::open(db, &next).map_err(|_| first)?;
                    kh.source.put(&kh.name, &next)?;
                    kh.source.delete(&next_name(&kh.name))?;
                    s
                }
                _ => return Err(first.into()),
            }
        }
    };
    if created && std::io::stderr().is_terminal() {
        eprintln!(
            "klatch: made an encryption key for this store in the {}; `klatch admin recovery` shows the code that restores it",
            kh.source.describe()
        );
    }
    if let Some(w) = kh.warning.filter(|_| std::io::stderr().is_terminal()) {
        eprintln!("klatch: {w}");
    }
    Ok(store)
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

/// Re-encrypt the store under a new key and make that the stored key. The order survives a crash at any step: the new
/// key is stored under a second name first, the store is re-keyed, then the new key takes the main name and the
/// second one goes (`open_store` finishes a run that died in between). Returns the new key.
pub(crate) fn rotate_key(db: &Path) -> Result<Key> {
    let kh = key_home(db);
    let store = open_store(db)?;
    let new = Key::generate().map_err(|e| anyhow::anyhow!("no random source: {e}"))?;
    kh.source.put(&next_name(&kh.name), &new)?;
    store.rekey(&new)?;
    kh.source.put(&kh.name, &new)?;
    kh.source.delete(&next_name(&kh.name))?;
    Ok(new)
}

/// The keys of `labels` this store has in its key source; a label with no key there is not held.
pub(crate) fn label_ring(db: &Path, labels: &[String]) -> Result<klatch_keys::LabelKeys> {
    let kh = key_home(db);
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    Ok(klatch_keys::LabelKeys::load(
        kh.source.as_ref(),
        &kh.name,
        &names,
    )?)
}

/// Make the key of a sealed label if there is none. Returns whether one was made. Under the directory lock, so two
/// processes sealing at once make one key.
pub(crate) fn ensure_label_key(db: &Path, label: &str) -> Result<bool> {
    let kh = key_home(db);
    let name = klatch_keys::label_key_name(&kh.name, label);
    let _lock = klatch_store::encrypt::lock_dir(db)?;
    if kh.source.get(&name)?.is_some() {
        return Ok(false);
    }
    klatch_keys::create(kh.source.as_ref(), &name)?;
    Ok(true)
}

/// The key of a sealed label, for showing its recovery code.
pub(crate) fn label_key(db: &Path, label: &str) -> Result<Option<Key>> {
    let kh = key_home(db);
    Ok(kh
        .source
        .get(&klatch_keys::label_key_name(&kh.name, label))?)
}

/// Put a sealed label's key back from its recovery code. It is refused if a different key is already there.
pub(crate) fn restore_label_key(db: &Path, label: &str, key: &Key) -> Result<()> {
    let kh = key_home(db);
    let name = klatch_keys::label_key_name(&kh.name, label);
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
