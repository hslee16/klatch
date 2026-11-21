//! A refusal that a session loop makes before any gateway operation runs (#1084). The session of an ssh forced command
//! refuses a request that claims an identity (rule `member-fixed`); the request never reaches an operation, so nothing
//! else would keep the refusal. Here it is kept the way every other refusal is: a row in the refusals and a look at the
//! breaker.

use super::*;

/// The operation a session-level refusal is kept under. Never the method the request named: that text came from the
/// peer, and the row must carry nothing a peer chose.
const SESSION_OPERATION: &str = "session";

impl<S: Store> Gateway<S> {
    /// Keep a `member-fixed` refusal for `who`, the member the transport fixed for this session, and count it toward
    /// the breaker. It records nothing about the request: not the field, not the name that was claimed. Failing to
    /// keep the row never undoes the refusal.
    pub fn note_member_fixed(&self, who: &Principal) {
        let e = Error::Refused {
            rule: "member-fixed".into(),
            reason: String::new(),
        };
        self.note_refusal(who, ("", SESSION_OPERATION), &e);
    }
}
