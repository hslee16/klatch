//! Rotating the key of one sealed label (#378), modelled on `rotate_key`: the same text is protected again under a new
//! key, atomically and with the store directory lock held, and a crash at any step leaves both keys so a repeated
//! rotation, or the next load of the label's key, finishes it.
//!
//! The order is the whole safety argument:
//! 1. every sealed text of the label, in the store and in every archive file, is checked to open with the current key
//!    or the new one; one that opens with neither refuses the rotation with nothing changed;
//! 2. the new key is stored under a second name (`<label key name>-next`);
//! 3. each archive file is moved through a temp copy, so each is whole under one of the two keys at every moment;
//! 4. the store is moved in one transaction (the point after which the new key is the one that opens the store);
//! 5. the new key takes the main name and the second name goes.
//!
//! A text that already opens with the new key is skipped, so a repeat finishes a run that died anywhere.

use super::*;
use crate::keys::label_key_name;

/// The points a test can stand in a crash at, in the order a rotation passes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// The new key is stored under the second name.
    NextStored,
    /// Every archive file is moved.
    ArchivesMoved,
    /// Inside the store transaction, after this many rows were rewritten.
    Row(usize),
    /// The store transaction committed; the main key is still the old one.
    StoreMoved,
    /// The new key has the main name; the second name is still there.
    Promoted,
}

fn second_name(label_name: &str) -> String {
    next_name(label_name)
}

/// Move `label` to a new key and make that its key in the keystore. Returns the new key. See the module comment for
/// what survives a crash. The store directory lock is held throughout, so a second rotation, a store key rotation
/// and every opener wait for it.
pub fn rotate_label_key(db: &Path, label: &str) -> Result<Key> {
    rotate_label_key_in(db, &key_home(db), label, &|_| Ok(()))
}

pub(super) fn rotate_label_key_in(
    db: &Path,
    kh: &KeyHome,
    label: &str,
    hook: &dyn Fn(Step) -> Result<()>,
) -> Result<Key> {
    // makes the key of a new store, converts a plaintext one; takes and drops the lock itself
    drop(open_store_in(db, kh)?);
    let _lock = lock_store_dir(db)?;
    let current = kh
        .source
        .get(&kh.name)?
        .ok_or_else(|| anyhow::anyhow!("this store has no key"))?;
    let store = open_keyed(db, kh, current)?;
    // open_keyed may have finished a store key rotation, so the store key is read once more
    let store_key = kh
        .source
        .get(&kh.name)?
        .ok_or_else(|| anyhow::anyhow!("this store has no key"))?;
    if crate::store::areas::PolicyStore::policy_get(&store, &format!("sealed:{label}"))?.is_none() {
        bail!("label {label} is not sealed, so it has no key to rotate");
    }
    let name = label_key_name(&kh.name, label);
    let old = kh.source.get(&name)?.ok_or_else(|| {
        anyhow::anyhow!(
            "label {label} has no key in the {}; `klatch admin recover --label {label} <recovery code>` puts it back",
            kh.source.describe()
        )
    })?;
    let second = second_name(&name);
    let new = match kh.source.get(&second)?.filter(|n| *n != old) {
        Some(n) => n,
        None => Key::generate().map_err(|e| anyhow::anyhow!("no random source: {e}"))?,
    };
    let archives = db.with_file_name("archive");
    // 1: nothing is stored or changed until every sealed text is known to open
    store.check_label_reseal(label, &old, &new)?;
    crate::store::resealable_archives(&archives, &store_key, label, &old, &new)?;
    // 2
    kh.source.put(&second, &new)?;
    hook(Step::NextStored)?;
    // 3
    crate::store::reseal_archives(&archives, &store_key, label, &old, &new)?;
    hook(Step::ArchivesMoved)?;
    // 4
    store.reseal_label_hooked(label, &old, &new, &|n| {
        hook(Step::Row(n)).map_err(|e| crate::domain::Error::Backend(e.to_string()))
    })?;
    hook(Step::StoreMoved)?;
    // 5
    kh.source.put(&name, &new)?;
    hook(Step::Promoted)?;
    kh.source.delete(&second)?;
    Ok(new)
}

/// The label twin of `open_keyed`'s repair: a rotation that died after its transaction but before the new key got the
/// main name leaves the store under the second key while the main key opens none of it. If the main key opens no
/// sealed text of `label` and the second key opens all of them, the second key becomes the main one. A second key
/// equal to the main one is just removed. Under the exclusive directory lock; best effort, because a process that
/// cannot take the lock or open the store simply goes on with the key it has.
pub(super) fn repair_label_key(db: &Path, kh: &KeyHome, label: &str) {
    let name = label_key_name(&kh.name, label);
    let second = second_name(&name);
    if !matches!(kh.source.get(&second), Ok(Some(_))) {
        return;
    }
    let _ = (|| -> Result<()> {
        let _lock = lock_store_dir(db)?;
        // a rotation may have finished while this waited for the lock
        let (Some(main), Some(next)) = (kh.source.get(&name)?, kh.source.get(&second)?) else {
            return Ok(());
        };
        if main == next {
            kh.source.delete(&second)?;
            return Ok(());
        }
        let Some(store_key) = kh.source.get(&kh.name)? else {
            return Ok(());
        };
        let store = crate::store::SqliteStore::open(db, &store_key)?;
        let (total, with_main, with_next) = store.label_open_counts(label, &main, &next)?;
        if total > 0 && with_main == 0 && with_next == total {
            kh.source.put(&name, &next)?;
            kh.source.delete(&second)?;
        }
        Ok(())
    })();
}
