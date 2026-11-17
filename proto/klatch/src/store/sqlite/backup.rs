//! An encrypted copy of the whole store.

use super::{SqliteStore, be};
use crate::domain::{Error, Result};
use crate::keys::Key;
use rusqlite::params;
use std::path::Path;

impl SqliteStore {
    /// Write an encrypted copy of the whole store to `out` (a backup or archive), under `key`. It never overwrites an
    /// existing file, builds the copy beside the target and checks that it opens before putting it in place, so a
    /// failure leaves nothing half written.
    pub fn backup_to(&self, out: &Path, key: &Key) -> Result<()> {
        if out.exists() {
            return Err(Error::Invalid(format!(
                "{} already exists; a backup never overwrites a file",
                out.display()
            )));
        }
        let mut partial = out.as_os_str().to_owned();
        partial.push(".partial");
        let partial = std::path::PathBuf::from(partial);
        let _ = std::fs::remove_file(&partial);
        let result = (|| -> Result<()> {
            self.conn
                .execute(
                    "ATTACH DATABASE ?1 AS bk KEY ?2",
                    params![partial.to_string_lossy(), key.sqlcipher_pragma().as_str()],
                )
                .map_err(be)?;
            self.conn
                .query_row("SELECT sqlcipher_export('bk')", [], |_| Ok(()))
                .map_err(be)?;
            let version: i64 = self
                .conn
                .query_row("PRAGMA main.user_version", [], |r| r.get(0))
                .map_err(be)?;
            self.conn
                .execute_batch(&format!(
                    "PRAGMA bk.user_version = {version}; DETACH DATABASE bk"
                ))
                .map_err(be)?;
            crate::store::encrypt::check_opens(&partial, key)?;
            std::fs::rename(&partial, out)
                .map_err(|e| Error::Backend(format!("putting the backup in place: {e}")))
        })();
        if result.is_err() {
            let _ = self.conn.execute_batch("DETACH DATABASE bk");
            let _ = std::fs::remove_file(&partial);
        }
        result
    }
}
