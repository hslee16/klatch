use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The caller supplied something unacceptable (bad name, for example).
    Invalid(String),
    /// Another member holds an overlapping claim.
    Conflict {
        path: String,
        owner: String,
        secs_left: i64,
    },
    /// A policy refused the operation. `rule` names the rule that did (for the audit record); `reason` is
    /// safe to show and never contains the content that triggered it.
    Refused { rule: String, reason: String },
    /// The thing asked for does not exist.
    NotFound(String),
    /// The storage backend failed.
    Backend(String),
    /// The store is locked by another writer past the wait limit. Transient: retrying later can succeed.
    Busy(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(m) | Error::NotFound(m) | Error::Backend(m) | Error::Busy(m) => {
                write!(f, "{m}")
            }
            Error::Refused { rule, reason } => write!(f, "refused ({rule}): {reason}"),
            Error::Conflict {
                path,
                owner,
                secs_left,
            } => {
                write!(
                    f,
                    "refused: {path} is claimed by {owner} for another {secs_left}s"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_message_names_holder_and_time() {
        let e = Error::Conflict {
            path: "src/a".into(),
            owner: "bob".into(),
            secs_left: 42,
        };
        assert_eq!(
            e.to_string(),
            "refused: src/a is claimed by bob for another 42s"
        );
    }

    #[test]
    fn a_policy_refusal_names_its_rule_and_reason() {
        let e = Error::Refused {
            rule: "label.room_below_message".into(),
            reason: "label too high".into(),
        };
        assert_eq!(
            e.to_string(),
            "refused (label.room_below_message): label too high"
        );
    }

    #[test]
    fn other_variants_display_their_message() {
        assert_eq!(Error::Invalid("bad".into()).to_string(), "bad");
        assert_eq!(Error::NotFound("none".into()).to_string(), "none");
        assert_eq!(Error::Backend("down".into()).to_string(), "down");
        assert_eq!(Error::Busy("locked".into()).to_string(), "locked");
    }
}
