//! The values a claim or a post hands the gateway: a claim's terms and a message's handoff.

use crate::domain::{Error, Result};

/// What a claim asks for: the path, how long it should last, why, and whether it is shared with every scope.
#[derive(Debug, Clone, Copy)]
pub struct ClaimSpec<'a> {
    pub path: &'a str,
    pub ttl: i64,
    pub reason: &'a str,
    pub shared: bool,
    /// The work item this claim is for, released when the item is reported done (#125).
    pub anchor: Option<&'a str>,
}

impl<'a> ClaimSpec<'a> {
    pub fn new(path: &'a str, ttl: i64, reason: &'a str) -> Self {
        Self {
            path,
            ttl,
            reason,
            shared: false,
            anchor: None,
        }
    }

    /// The same claim anchored to a work item.
    pub fn anchored(self, anchor: &'a str) -> Self {
        Self {
            anchor: Some(anchor),
            ..self
        }
    }

    /// The same claim, shared with every scope when `shared`.
    pub fn shared(self, shared: bool) -> Self {
        Self { shared, ..self }
    }
}

/// A handoff on a message: a short kind and an optional opaque payload. klatch never reads them.
pub struct Handoff<'a> {
    pub kind: &'a str,
    pub payload: Option<&'a str>,
}

impl Handoff<'_> {
    pub(super) fn check(&self) -> Result<()> {
        if !crate::domain::valid_kind(self.kind) {
            return Err(Error::Invalid(
                "a handoff kind is 1 to 40 characters of letters, digits, dot, dash and underscore"
                    .into(),
            ));
        }
        if let Some(p) = self.payload {
            if p.is_empty() || p.contains('\u{0}') {
                return Err(Error::Invalid(
                    "a payload is text, not empty and without NUL".into(),
                ));
            }
            if p.len() > crate::domain::MAX_PAYLOAD_BYTES {
                return Err(Error::Invalid(format!(
                    "a payload is at most {} bytes",
                    crate::domain::MAX_PAYLOAD_BYTES
                )));
            }
        }
        Ok(())
    }
}
