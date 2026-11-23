//! The machine-derived member status (#1403): what the opt-in hooks write to a member's status field from activity,
//! instead of the member posting `working:` and `parked:` lines. It is state, not a log line: it posts no message,
//! carries no tool name, command or content, and never replaces a status a member set itself.

/// The most often a machine status is written for one member in one room: a burst of tool calls is one write. Also
/// how often an unchanged `working (auto)` is refreshed, so the age shown beside it stays about right.
pub const MACHINE_STATUS_WINDOW_SECS: i64 = 60;

/// The status `SessionEnd` writes in each room (`done session ended`). It is the machine's own, so a session that
/// resumes under the same name is `working (auto)` again at its first tool call instead of staying done.
pub const SESSION_END_STATUS: &str = "done session ended";

/// What a hook saw, as a status class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineClass {
    /// A tool call arrived after the member was idle (PostToolUse).
    Working,
    /// A turn ended with nothing left running (Stop).
    Idle,
    /// A turn ended with background tasks or crons still running (Stop): a declared long wait.
    Parked,
}

impl MachineClass {
    /// The closed word a caller names the class by.
    pub fn word(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Parked => "parked",
        }
    }

    /// The class named by `word`, or none.
    pub fn from_word(word: &str) -> Option<Self> {
        [Self::Working, Self::Idle, Self::Parked]
            .into_iter()
            .find(|c| c.word() == word)
    }

    /// The status text: the word with the machine mark, `working (auto)`.
    pub fn text(self) -> &'static str {
        match self {
            Self::Working => "working (auto)",
            Self::Idle => "idle (auto)",
            Self::Parked => "parked (auto)",
        }
    }
}

/// The status texts the machine owns and may replace: its own three and the one `SessionEnd` writes. Anything else
/// in the status field was set by the member (or a human) and wins.
pub fn machine_owned() -> [&'static str; 4] {
    [
        MachineClass::Working.text(),
        MachineClass::Idle.text(),
        MachineClass::Parked.text(),
        SESSION_END_STATUS,
    ]
}

/// Whether `status` is machine-derived, one of the texts of `MachineClass`.
pub fn is_machine_status(status: &str) -> bool {
    [
        MachineClass::Working,
        MachineClass::Idle,
        MachineClass::Parked,
    ]
    .iter()
    .any(|c| c.text() == status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_have_closed_words_and_marked_text() {
        for c in [
            MachineClass::Working,
            MachineClass::Idle,
            MachineClass::Parked,
        ] {
            assert_eq!(MachineClass::from_word(c.word()), Some(c));
            assert!(c.text().ends_with(" (auto)") && is_machine_status(c.text()));
        }
        assert_eq!(MachineClass::from_word("done"), None);
        assert_eq!(MachineClass::from_word("working (auto)"), None);
        assert!(!is_machine_status("parked: waiting on CI"));
    }
}
