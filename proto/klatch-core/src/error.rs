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
        /// The holder is itself waiting, directly or through others, for something the requester holds, and the
        /// requester's attempt closed that cycle: the requester yields (DESIGN section 25.12).
        deadlock: bool,
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
                deadlock,
            } => {
                write!(
                    f,
                    "refused: {path} is claimed by {owner} for another {secs_left}s"
                )?;
                if *deadlock {
                    write!(
                        f,
                        "; deadlock: {owner} is waiting, directly or through others, for a path you hold, and your attempt closed the cycle, so you yield: release the claims you hold in this room, wait, then try again"
                    )?;
                }
                Ok(())
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
            deadlock: false,
        };
        assert_eq!(
            e.to_string(),
            "refused: src/a is claimed by bob for another 42s"
        );
    }

    #[test]
    fn a_deadlock_conflict_says_so_and_who_yields() {
        let e = Error::Conflict {
            path: "src/a".into(),
            owner: "bob".into(),
            secs_left: 42,
            deadlock: true,
        };
        let m = e.to_string();
        assert!(
            m.starts_with("refused: src/a is claimed by bob for another 42s"),
            "{m}"
        );
        assert!(
            m.contains("deadlock")
                && m.contains("you yield")
                && m.contains("release the claims you hold"),
            "{m}"
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
