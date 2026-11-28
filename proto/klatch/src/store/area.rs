//! One trait per area of the store seam; `Store` (in the parent module) is all of them. A new store method goes into the area it belongs to.

mod asks;
mod claims;
mod credentials;
#[cfg(test)]
mod default_tests;
#[cfg(test)]
mod feature_tests;
mod import;
mod members;
mod messages;
pub use import::ImportStore;
mod policy;
mod reviews;
mod rooms;
#[cfg(test)]
mod security_defaults_tests;

/// The answer of a default body for a feature a backend must provide for the checks to mean anything (#485): an
/// error that says what is missing, never the empty answer a check would read as "all clear".
pub(crate) fn unsupported<T>(what: &str) -> crate::domain::Result<T> {
    Err(crate::domain::Error::Backend(format!(
        "this store does not keep {what}"
    )))
}

#[cfg(test)]
pub use crate::domain::{MAX_AUDIT_TARGET, TRUNCATED};
pub use asks::AskStore;
pub use claims::ClaimStore;
pub use credentials::CredentialStore;
pub use members::MemberStore;
pub use messages::{MessageStore, Shown};
pub use policy::{Census, PolicyStore, audit_target, is_proof_key, normalize_key};
pub use reviews::ReviewStore;
pub use rooms::RoomStore;
