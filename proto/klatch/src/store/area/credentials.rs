//! The `CredentialStore` area of the store seam; `Store` in the parent module combines every area.

use crate::domain::Result;
use crate::store::{CredentialRecord, NewCredential};

pub trait CredentialStore {
    /// Register a credential for a member. A member that holds a live one (not revoked, not expired) cannot be given
    /// another until it is revoked or lapses. The default registers nothing.
    fn issue_credential(&self, _credential: &NewCredential, _now: i64) -> Result<()> {
        Err(crate::domain::Error::Invalid(
            "this store keeps no credentials".into(),
        ))
    }

    fn credential(&self, _member: &str) -> Result<Option<CredentialRecord>> {
        Ok(None)
    }

    /// Revoke a member's credential; false when there is none or it was already revoked.
    fn revoke_credential(&self, _member: &str, _now: i64) -> Result<bool> {
        Ok(false)
    }

    fn credentials(&self) -> Result<Vec<CredentialRecord>> {
        Ok(Vec::new())
    }
}
