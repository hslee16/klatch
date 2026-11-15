//! Retention and archive (#69, #70). Old messages of a room move into an encrypted archive file and leave a tombstone
//! behind (same room, seq, id, sender and time; the text replaced), so `seq` stays gapless and a reference to the
//! message still resolves. A restore puts the text back from the archive. The archive is written under the store key
//! with SQLCipher, so no plaintext copy of old messages exists at any point; sealed-label text is copied as the
//! ciphertext it is stored as.

use super::{SqliteStore, be};
use crate::domain::{Error, Result};
use crate::keys::Key;
use rusqlite::params;
use std::path::Path;

/// What a tombstone says in place of the text.
pub const TOMBSTONE: &str = "[archived: restore it with `klatch admin restore-archive`]";
const KIND: &str = "archived";

impl SqliteStore {
    /// Every room that has at least one message.
    pub fn message_rooms(&self) -> Result<Vec<String>> {
        self.rows(
            "SELECT DISTINCT room FROM messages ORDER BY room",
            [],
            |r| r.get(0),
        )
    }

    /// Move the messages of `room` older than `cutoff` (unix seconds) into a new encrypted archive file `out` and
    /// leave tombstones. Returns how many moved; with none, no file is written. Never overwrites a file, builds the
    /// archive beside the target and checks it opens before putting it in place, and changes the live rows only after
    /// the archive is safely there.
    pub fn archive_before(&self, room: &str, cutoff: i64, out: &Path, key: &Key) -> Result<usize> {
        let due: i64 = self
            .conn
            .query_row(
                "SELECT count(*) FROM messages WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3",
                params![room, cutoff, KIND],
                |r| r.get(0),
            )
            .map_err(be)?;
        if due == 0 {
            return Ok(0);
        }
        if out.exists() {
            return Err(Error::Invalid(format!(
                "{} already exists; an archive never overwrites a file",
                out.display()
            )));
        }
        let mut partial = out.as_os_str().to_owned();
        partial.push(".partial");
        let partial = std::path::PathBuf::from(partial);
        let _ = std::fs::remove_file(&partial);
        let copied = (|| -> Result<usize> {
            self.conn
                .execute(
                    "ATTACH DATABASE ?1 AS ar KEY ?2",
                    params![partial.to_string_lossy(), key.sqlcipher_pragma().as_str()],
                )
                .map_err(be)?;
            self.conn
                .execute_batch(
                    "CREATE TABLE ar.archived(room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER, attribution TEXT NOT NULL, label TEXT NOT NULL, kind TEXT, payload TEXT, PRIMARY KEY(room,seq));",
                )
                .map_err(be)?;
            let n = self
                .conn
                .execute(
                    "INSERT INTO ar.archived SELECT room,seq,mid,sender,member,body,ts,attribution,label,kind,payload FROM main.messages WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3",
                    params![room, cutoff, KIND],
                )
                .map_err(be)?;
            self.conn.execute_batch("DETACH DATABASE ar").map_err(be)?;
            crate::store::encrypt::check_opens(&partial, key)?;
            std::fs::rename(&partial, out)
                .map_err(|e| Error::Backend(format!("putting the archive in place: {e}")))?;
            Ok(n)
        })();
        let copied = match copied {
            Ok(n) => n,
            Err(e) => {
                let _ = self.conn.execute_batch("DETACH DATABASE ar");
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
        };
        self.immediate(|| {
            self.exec(
                "UPDATE messages SET body=?4, payload=NULL, kind=?3 WHERE room=?1 AND ts<?2 AND COALESCE(kind,'')<>?3",
                params![room, cutoff, KIND, TOMBSTONE],
            )
            .map(|_| ())
        })?;
        Ok(copied)
    }

    /// Put archived messages back from `file`: a live row is restored only when it is a tombstone with the same room,
    /// seq and id, so an archive from another store or an edited row changes nothing. Returns how many came back; the
    /// archive file is kept.
    pub fn restore_archive(&self, file: &Path, key: &Key) -> Result<usize> {
        crate::store::encrypt::check_opens(file, key)?;
        self.conn
            .execute(
                "ATTACH DATABASE ?1 AS ar KEY ?2",
                params![file.to_string_lossy(), key.sqlcipher_pragma().as_str()],
            )
            .map_err(be)?;
        let result = self.immediate(|| {
            self.exec(
                "UPDATE main.messages SET
                   body=(SELECT a.body FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq),
                   payload=(SELECT a.payload FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq),
                   kind=(SELECT a.kind FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq)
                 WHERE kind=?1 AND body=?2 AND EXISTS
                   (SELECT 1 FROM ar.archived a WHERE a.room=messages.room AND a.seq=messages.seq AND a.mid=messages.mid)",
                params![KIND, TOMBSTONE],
            )
        });
        let _ = self.conn.execute_batch("DETACH DATABASE ar");
        result
    }
}
