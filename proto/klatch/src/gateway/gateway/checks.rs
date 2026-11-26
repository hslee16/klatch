//! The shape checks every gateway call runs before it asks the guard or touches the store: who is calling, which
//! room, and the bounds on names, paths, reasons and statuses.
//!
//! Each kind of value has a type of its own (`Room`, `ClaimPath`, `Status`, ...), built from a `&str` with `.into()`,
//! so a check cannot be handed a string meant for another check. The check functions take `impl Into<..>`, so a
//! caller that holds a `&str` writes `check_room(room)?` as before.

use crate::domain::{Error, NEXT_STEP, Principal, Result, valid_name};

pub(super) const MAX_PATH_BYTES: usize = 1024;
pub(super) const MAX_REASON_BYTES: usize = 500;

/// A room name, not yet checked.
pub(super) struct Room<'a>(&'a str);
/// A member's one-line status, not yet checked.
pub(super) struct Status<'a>(&'a str);
/// A hook event name, not yet checked.
pub(super) struct EventName<'a>(&'a str);
/// A coding-agent session id, not yet checked.
pub(super) struct SessionId<'a>(&'a str);
/// A claim path, not yet checked.
pub(super) struct ClaimPath<'a>(&'a str);
/// The reason given with a claim, not yet checked.
pub(super) struct ClaimReason<'a>(&'a str);
/// Free text, not yet checked.
pub(super) struct Text<'a>(&'a str);

macro_rules! from_str {
    ($($kind:ident),*) => {$(
        impl<'a> From<&'a str> for $kind<'a> {
            fn from(s: &'a str) -> Self {
                $kind(s)
            }
        }
    )*};
}
from_str!(
    Room,
    Status,
    EventName,
    SessionId,
    ClaimPath,
    ClaimReason,
    Text
);

impl Text<'_> {
    fn has_control(&self) -> bool {
        self.0.chars().any(char::is_control)
    }

    /// At most `max` bytes with no control character.
    fn plain(&self, max: usize) -> bool {
        self.0.len() <= max && !self.has_control()
    }

    /// 1 to `max` characters, each alphanumeric or one of `extra`.
    fn identifier(&self, max: usize, extra: &str) -> bool {
        let allowed = |c: char| c.is_ascii_alphanumeric() || extra.contains(c);
        !self.0.is_empty() && self.0.len() <= max && self.0.chars().all(allowed)
    }
}

/// An audit action filter must be a well formed action name (`store::valid_audit_action`).
pub(super) fn check_audit_action(action: &str) -> Result<()> {
    if crate::store::valid_audit_action(action) {
        Ok(())
    } else {
        Err(Error::Invalid(
            "an audit action is 1 to 64 of a-z, 0-9, -, _, : and .".into(),
        ))
    }
}

pub(super) fn check_member(who: &Principal) -> Result<()> {
    if valid_name(&who.member) {
        Ok(())
    } else {
        Err(Error::Invalid("bad member name".into()))
    }
}

pub(super) fn check_room<'a>(room: impl Into<Room<'a>>) -> Result<()> {
    if valid_name(room.into().0) {
        Ok(())
    } else {
        Err(Error::Invalid("bad room name".into()))
    }
}

/// Text of at most `max` bytes with no control character.
pub(super) fn plain_text<'a>(s: impl Into<Text<'a>>, max: usize) -> bool {
    s.into().plain(max)
}

/// An identifier: 1 to `max` characters, each alphanumeric or one of `extra`.
pub(super) fn identifier<'a>(s: impl Into<Text<'a>>, max: usize, extra: &str) -> bool {
    s.into().identifier(max, extra)
}

pub(super) fn invalid(unless: bool, what: String) -> Result<()> {
    if unless {
        Ok(())
    } else {
        Err(Error::Invalid(what))
    }
}

/// Refuse text that carries a credential (R20): a room is a log other agents and people read, not a place for secrets.
/// The refusal names the kind of credential and never the text.
pub(super) fn check_no_credentials<'a>(text: impl Into<Text<'a>>) -> Result<()> {
    match crate::domain::credential_kind(text.into().0) {
        None => Ok(()),
        Some(kind) => Err(Error::Refused {
            rule: "credentials".into(),
            reason: format!(
                "the text looks like it contains {kind}; {NEXT_STEP}remove the secret and say where it is kept instead, then post again"
            ),
        }),
    }
}

pub(super) fn check_status<'a>(status: impl Into<Status<'a>>) -> Result<()> {
    let status = status.into().0;
    let ok =
        !status.trim().is_empty() && status.chars().count() <= 80 && !Text(status).has_control();
    invalid(
        ok,
        "bad status: one line of 1 to 80 characters, no control characters".into(),
    )
}

pub(super) fn check_event_name<'a>(event: impl Into<EventName<'a>>) -> Result<()> {
    invalid(
        Text(event.into().0).identifier(40, ""),
        "bad hook event name".into(),
    )
}

pub(super) fn check_session_id<'a>(session_id: impl Into<SessionId<'a>>) -> Result<()> {
    invalid(
        Text(session_id.into().0).identifier(80, "-_."),
        "bad session id".into(),
    )
}

pub(super) fn check_path<'a>(path: impl Into<ClaimPath<'a>>) -> Result<()> {
    let path = Text(path.into().0);
    let ok = !path.0.is_empty() && path.plain(MAX_PATH_BYTES);
    invalid(
        ok,
        format!("bad claim path: 1 to {MAX_PATH_BYTES} bytes, no control characters"),
    )
}

pub(super) fn check_reason<'a>(reason: impl Into<ClaimReason<'a>>) -> Result<()> {
    invalid(
        Text(reason.into().0).plain(MAX_REASON_BYTES),
        format!("bad claim reason: at most {MAX_REASON_BYTES} bytes, no control characters"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_name_is_checked_as_before() {
        assert!(check_room("lobby").is_ok());
        assert!(check_room("bad name").is_err());
        assert!(check_room("").is_err());
    }

    #[test]
    fn a_path_is_one_to_the_limit_bytes_without_control_characters() {
        assert!(check_path("src/a.rs").is_ok());
        assert!(check_path("").is_err());
        assert!(check_path("a\nb").is_err());
        assert!(check_path("x".repeat(MAX_PATH_BYTES).as_str()).is_ok());
        assert!(check_path("x".repeat(MAX_PATH_BYTES + 1).as_str()).is_err());
    }

    #[test]
    fn a_reason_may_be_empty_but_not_long_or_multiline() {
        assert!(check_reason("").is_ok());
        assert!(check_reason("refactor").is_ok());
        assert!(check_reason("a\u{7}b").is_err());
        assert!(check_reason("x".repeat(MAX_REASON_BYTES + 1).as_str()).is_err());
    }

    #[test]
    fn a_status_is_one_visible_line_of_at_most_80_characters() {
        assert!(check_status("working").is_ok());
        assert!(check_status("   ").is_err());
        assert!(check_status("two\nlines").is_err());
        assert!(check_status("é".repeat(80).as_str()).is_ok());
        assert!(check_status("é".repeat(81).as_str()).is_err());
    }

    #[test]
    fn event_names_and_session_ids_are_identifiers_with_their_own_limits() {
        assert!(check_event_name("PreToolUse").is_ok());
        assert!(check_event_name("bad event").is_err());
        assert!(check_event_name("x".repeat(41).as_str()).is_err());
        assert!(check_session_id("abc-1_2.3").is_ok());
        assert!(check_session_id("bad id").is_err());
        assert!(check_session_id("x".repeat(81).as_str()).is_err());
    }

    #[test]
    fn text_with_a_credential_is_refused_without_repeating_it() {
        assert!(check_no_credentials("plain words").is_ok());
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let e = check_no_credentials(secret).unwrap_err();
        assert!(!format!("{e:?}").contains(secret));
    }
}
