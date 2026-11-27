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
        /// How long the holder has held the claim, 0 when unknown (#547).
        held_secs: i64,
        /// The holder is itself waiting, directly or through others, for something the requester holds, and the
        /// requester's attempt closed that cycle: the requester yields (DESIGN section 25.12).
        deadlock: bool,
        /// On a deadlock: the members of this room on the chain, from the holder to the member that waits for the
        /// requester (#1603). Empty otherwise.
        cycle: Vec<String>,
        /// On a deadlock: the requester's own held path that the last member on the chain waits for. Empty otherwise.
        via: String,
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

/// Members named after the first one on a deadlock's chain before the rest are counted (#1603): the first member and up
/// to this many more show.
const MAX_CYCLE_NAMES: usize = 5;

/// What a refusal's reason puts before its hint: `<why>; next step: <what to do>`.
pub const NEXT_STEP: &str = "next step: ";

impl Error {
    /// The step that fixes a refusal, when its reason ends with one. A hint never holds the refused text, the label
    /// of a room the member cannot see, or another member's data (#516).
    pub fn next_step(&self) -> Option<&str> {
        if let Error::Refused { reason, .. } = self {
            return reason.split_once(NEXT_STEP).map(|(_, hint)| hint);
        }
        None
    }
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
                held_secs,
                deadlock,
                cycle,
                via,
            } => {
                write!(
                    f,
                    "refused: {path} is claimed by {owner} for another {secs_left}s"
                )?;
                if *held_secs >= crate::domain::FIRST_MILESTONE_SECS {
                    write!(
                        f,
                        " (held for {}; ask {owner} whether it is still needed)",
                        crate::domain::parked::span(*held_secs)
                    )?;
                }
                if *deadlock {
                    // The member in the cycle, which is not always the one that refused first (#1603).
                    let owner = cycle.first().unwrap_or(owner);
                    write!(
                        f,
                        "; deadlock: {owner} is waiting, directly or through others, for a path you hold, and your attempt closed the cycle, so you yield: release the claims you hold in this room, wait, then try again"
                    )?;
                    if !via.is_empty() && !cycle.is_empty() {
                        write!(f, " (the cycle: {owner} waits")?;
                        let rest = &cycle[1..];
                        if !rest.is_empty() {
                            let shown = rest.len().min(MAX_CYCLE_NAMES);
                            write!(f, " through {}", rest[..shown].join(", "))?;
                            if rest.len() > shown {
                                write!(f, " and {} more", rest.len() - shown)?;
                            }
                        }
                        write!(f, " for your path {via})")?;
                    }
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
            held_secs: 0,
            deadlock: false,
            cycle: vec![],
            via: String::new(),
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
            held_secs: 0,
            deadlock: true,
            cycle: vec!["bob".into()],
            via: "src/b".into(),
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
    fn a_refusal_hint_is_the_text_after_the_next_step_marker() {
        let e = Error::Refused {
            rule: "r".into(),
            reason: format!("it is too high; {NEXT_STEP}ask the admin"),
        };
        assert_eq!(e.next_step(), Some("ask the admin"));
    }

    #[test]
    fn a_refusal_without_the_marker_has_no_hint() {
        let e = Error::Refused {
            rule: "r".into(),
            reason: "it is too high".into(),
        };
        assert_eq!(e.next_step(), None);
    }

    #[test]
    fn only_a_refusal_has_a_hint() {
        let m = format!("x; {NEXT_STEP}y");
        assert_eq!(Error::Invalid(m.clone()).next_step(), None);
        assert_eq!(Error::NotFound(m.clone()).next_step(), None);
        assert_eq!(Error::Backend(m.clone()).next_step(), None);
        assert_eq!(Error::Busy(m).next_step(), None);
    }

    #[test]
    fn other_variants_display_their_message() {
        assert_eq!(Error::Invalid("bad".into()).to_string(), "bad");
        assert_eq!(Error::NotFound("none".into()).to_string(), "none");
        assert_eq!(Error::Backend("down".into()).to_string(), "down");
        assert_eq!(Error::Busy("locked".into()).to_string(), "locked");
    }
}
