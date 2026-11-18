//! A member's kind (DESIGN section 32 item 95, #301): what sort of participant it is, so a requester can choose a
//! helper (a human is slower than a bot; a service may answer only fixed asks). Display only: no rule reads it, and
//! it never changes who may do what. The vocabulary is closed. A coding session's kind comes from klatch's own
//! session record, never from a claim; a bot, a service or a human declares its own.

/// The policy key prefix a member's declared kind is stored under (`kind:NAME`), written only for the caller's name.
pub const KIND_KEY: &str = "kind:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Session,
    Bot,
    Service,
    Human,
}

impl MemberKind {
    /// The kind a word names, exactly as written in lower case.
    pub fn parse(word: &str) -> Option<MemberKind> {
        match word {
            "session" => Some(MemberKind::Session),
            "bot" => Some(MemberKind::Bot),
            "service" => Some(MemberKind::Service),
            "human" => Some(MemberKind::Human),
            _ => None,
        }
    }

    /// The word shown and stored.
    pub fn word(self) -> &'static str {
        match self {
            MemberKind::Session => "session",
            MemberKind::Bot => "bot",
            MemberKind::Service => "service",
            MemberKind::Human => "human",
        }
    }

    /// Whether a member may declare this kind. `session` is not declarable: it is derived from a session record.
    pub fn declarable(self) -> bool {
        self != MemberKind::Session
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_is_closed_and_exact() {
        for (word, kind) in [
            ("session", MemberKind::Session),
            ("bot", MemberKind::Bot),
            ("service", MemberKind::Service),
            ("human", MemberKind::Human),
        ] {
            assert_eq!(MemberKind::parse(word), Some(kind));
            assert_eq!(kind.word(), word);
        }
        for bad in [
            "", "Bot", " bot", "bot ", "robot", "bot\n", "admin", "agent",
        ] {
            assert_eq!(MemberKind::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_a_session_record_makes_a_session() {
        assert!(!MemberKind::Session.declarable());
        for k in [MemberKind::Bot, MemberKind::Service, MemberKind::Human] {
            assert!(k.declarable());
        }
    }
}
