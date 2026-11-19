//! The SQLite side of the `CredentialStore` area.

use super::super::{SqliteStore, credential_from_row};
use crate::domain::Result;
use crate::store::CredentialStore;
use rusqlite::params;

impl CredentialStore for SqliteStore {
    fn issue_credential(&self, c: &crate::store::NewCredential, now: i64) -> Result<()> {
        self.do_issue_credential(c, now)
    }

    fn credential(&self, member: &str) -> Result<Option<crate::store::CredentialRecord>> {
        Ok(self
            .rows(
                "SELECT member,token_hash,clearance,parent,issued,expires,revoked,idle,cap FROM credentials WHERE member=?1",
                params![member],
                credential_from_row,
            )?
            .into_iter()
            .next())
    }

    fn revoke_credential(&self, member: &str, _now: i64) -> Result<bool> {
        Ok(self.exec(
            "UPDATE credentials SET revoked=1 WHERE member=?1 AND revoked=0",
            params![member],
        )? > 0)
    }

    fn renew_credential(&self, member: &str, now: i64, step: i64) -> Result<Option<i64>> {
        self.do_renew_credential(member, now, step)
    }

    fn credentials(&self) -> Result<Vec<crate::store::CredentialRecord>> {
        self.rows(
            "SELECT member,token_hash,clearance,parent,issued,expires,revoked,idle,cap FROM credentials ORDER BY member",
            params![],
            credential_from_row,
        )
    }
}
