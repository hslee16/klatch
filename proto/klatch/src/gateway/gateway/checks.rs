//! The shape checks every gateway call runs before it asks the guard or touches the store: who is calling, which
//! room, and the bounds on names, paths, reasons and statuses.

use crate::domain::{Error, NEXT_STEP, Principal, Result, valid_name};

pub(super) const MAX_PATH_BYTES: usize = 1024;
pub(super) const MAX_REASON_BYTES: usize = 500;

pub(super) fn check_member(who: &Principal) -> Result<()> {
    if valid_name(&who.member) {
        Ok(())
    } else {
        Err(Error::Invalid("bad member name".into()))
    }
}

pub(super) fn check_room(room: &str) -> Result<()> {
    if valid_name(room) {
        Ok(())
    } else {
        Err(Error::Invalid("bad room name".into()))
    }
}

pub(super) fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// Text of at most `max` bytes with no control character.
pub(super) fn plain_text(s: &str, max: usize) -> bool {
    s.len() <= max && !has_control(s)
}

/// An identifier: 1 to `max` characters, each alphanumeric or one of `extra`.
pub(super) fn identifier(s: &str, max: usize, extra: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || extra.contains(c);
    !s.is_empty() && s.len() <= max && s.chars().all(allowed)
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
pub(super) fn check_no_credentials(text: &str) -> Result<()> {
    match crate::domain::credential_kind(text) {
        None => Ok(()),
        Some(kind) => Err(Error::Refused {
            rule: "credentials".into(),
            reason: format!(
                "the text looks like it contains {kind}; {NEXT_STEP}remove the secret and say where it is kept instead, then post again"
            ),
        }),
    }
}

pub(super) fn check_status(status: &str) -> Result<()> {
    let ok = !status.trim().is_empty() && status.chars().count() <= 80 && !has_control(status);
    invalid(
        ok,
        "bad status: one line of 1 to 80 characters, no control characters".into(),
    )
}

pub(super) fn check_event_name(event: &str) -> Result<()> {
    invalid(identifier(event, 40, ""), "bad hook event name".into())
}

pub(super) fn check_session_id(session_id: &str) -> Result<()> {
    invalid(identifier(session_id, 80, "-_."), "bad session id".into())
}

pub(super) fn check_path(path: &str) -> Result<()> {
    let ok = !path.is_empty() && plain_text(path, MAX_PATH_BYTES);
    invalid(
        ok,
        format!("bad claim path: 1 to {MAX_PATH_BYTES} bytes, no control characters"),
    )
}

pub(super) fn check_reason(reason: &str) -> Result<()> {
    invalid(
        plain_text(reason, MAX_REASON_BYTES),
        format!("bad claim reason: at most {MAX_REASON_BYTES} bytes, no control characters"),
    )
}
