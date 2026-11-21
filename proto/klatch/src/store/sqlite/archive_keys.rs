//! Moving one label's key in the archive files (#378). An archive holds copies of messages, sealed bodies included,
//! so a label's key cannot move without them. Each file is rewritten through a temp copy that is opened and checked
//! before it replaces the original, as `rekey_archives_in` does for the store key, so at every moment each file is
//! whole, and a file whose rows already open with the new key is left alone, which makes a repeat finish a crash.

use super::archive::archive_files;
use super::be;
use super::label_keys::{Moved, move_text};
use crate::domain::{Error, Result};
use crate::keys::Key;
use rusqlite::params;
use std::path::{Path, PathBuf};

/// One archived row of a label.
struct ArchivedRow {
    room: String,
    seq: i64,
    mid: String,
    body: Option<String>,
    payload: Option<String>,
}

type Update = (String, i64, Option<String>, Option<String>);

fn open_archive(file: &Path, store_key: &Key, write: bool) -> Result<rusqlite::Connection> {
    let flags = if write {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let conn = rusqlite::Connection::open_with_flags(file, flags).map_err(be)?;
    conn.pragma_update(None, "key", store_key.sqlcipher_pragma().as_str())
        .map_err(be)?;
    Ok(conn)
}

/// What moving `label` from `old` to `new` rewrites in one archive file; an error when a sealed text opens with
/// neither key or the file cannot be read with the store key.
fn plan_archive(
    file: &Path,
    store_key: &Key,
    label: &str,
    keys: (&Key, &Key),
) -> Result<Vec<Update>> {
    let named = |e: &dyn std::fmt::Display| Error::Invalid(format!("{}: {e}", file.display()));
    let conn = open_archive(file, store_key, false).map_err(|e| named(&e))?;
    let rows: Vec<ArchivedRow> = conn
        .prepare("SELECT room, seq, mid, body, payload FROM archived WHERE label=?1")
        .and_then(|mut st| {
            st.query_map([label], |r| {
                Ok(ArchivedRow {
                    room: r.get(0)?,
                    seq: r.get(1)?,
                    mid: r.get(2)?,
                    body: r.get(3)?,
                    payload: r.get(4)?,
                })
            })?
            .collect()
        })
        .map_err(|e| named(&e))?;
    let mut plan = Vec::new();
    let mut stuck = 0;
    for row in rows {
        let mut next = [None, None];
        for (i, (field, text)) in [("body", &row.body), ("payload", &row.payload)]
            .into_iter()
            .enumerate()
        {
            let Some(text) = text else { continue };
            match move_text((&row.mid, label, field), text, keys)? {
                Moved::Keep => {}
                Moved::To(t) => next[i] = Some(t),
                Moved::Stuck => stuck += 1,
            }
        }
        let [body, payload] = next;
        if body.is_some() || payload.is_some() {
            plan.push((row.room, row.seq, body, payload));
        }
    }
    if stuck > 0 {
        return Err(named(&format!(
            "{stuck} sealed text(s) of label {label} open with neither its current key nor the new one, so the key was not changed"
        )));
    }
    Ok(plan)
}

/// The archive files that hold texts of `label` still under `old` (a file with none, or already moved, is not
/// listed). An error naming every file that cannot be checked or holds a text neither key opens; nothing is
/// changed, so the caller can refuse a rotation before it stores the new key.
pub fn resealable_archives(
    dir: &Path,
    store_key: &Key,
    label: &str,
    old: &Key,
    new: &Key,
) -> Result<Vec<PathBuf>> {
    let mut todo = Vec::new();
    let mut problems = Vec::new();
    for path in archive_files(dir)? {
        match plan_archive(&path, store_key, label, (old, new)) {
            Ok(plan) if plan.is_empty() => {}
            Ok(_) => todo.push(path),
            Err(e) => problems.push(e.to_string()),
        }
    }
    if problems.is_empty() {
        Ok(todo)
    } else {
        Err(Error::Invalid(problems.join("; ")))
    }
}

/// Move every text of `label` in the archive files of `dir` from `old` to `new`, each file through a temp copy. Safe
/// to repeat. Returns how many files were rewritten.
pub fn reseal_archives(
    dir: &Path,
    store_key: &Key,
    label: &str,
    old: &Key,
    new: &Key,
) -> Result<usize> {
    reseal_archives_hooked(dir, store_key, label, (old, new), &|_| Ok(()))
}

/// `reseal_archives` with a hook called after each file, with the count so far; an error from it stops the run there.
pub(crate) fn reseal_archives_hooked(
    dir: &Path,
    store_key: &Key,
    label: &str,
    (old, new): (&Key, &Key),
    after_each: &dyn Fn(usize) -> Result<()>,
) -> Result<usize> {
    let todo = resealable_archives(dir, store_key, label, old, new)?;
    for (done, path) in todo.iter().enumerate() {
        reseal_archive_in(path, store_key, label, (old, new))?;
        after_each(done + 1)?;
    }
    Ok(todo.len())
}

fn reseal_archive_in(file: &Path, store_key: &Key, label: &str, keys: (&Key, &Key)) -> Result<()> {
    let plan = plan_archive(file, store_key, label, keys)?;
    let mut partial = file.as_os_str().to_owned();
    partial.push(format!(".{}.partial", std::process::id()));
    let partial = PathBuf::from(partial);
    let _ = std::fs::remove_file(&partial);
    let made = (|| -> Result<()> {
        std::fs::copy(file, &partial)
            .map_err(|e| Error::Backend(format!("copying {}: {e}", file.display())))?;
        let mut conn = open_archive(&partial, store_key, true)?;
        let tx = conn.transaction().map_err(be)?;
        for (room, seq, body, payload) in &plan {
            tx.execute(
                "UPDATE archived SET body=COALESCE(?3, body), payload=COALESCE(?4, payload) WHERE room=?1 AND seq=?2",
                params![room, seq, body, payload],
            )
            .map_err(be)?;
        }
        tx.commit().map_err(be)?;
        drop(conn);
        crate::store::encrypt::check_opens(&partial, store_key)?;
        std::fs::rename(&partial, file)
            .map_err(|e| Error::Backend(format!("putting {} in place: {e}", file.display())))
    })();
    if made.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    made
}
