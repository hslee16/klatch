//! What the coding agent on this machine supports, found by asking it (#482, DESIGN D9 "a version probe gates the
//! feature"). The one feature that depends on the harness version today is the wake watcher: it relies on a hook
//! flag, `asyncRewake`, which a Claude Code too old to have it ignores. A probe that says the harness is too old
//! turns the watcher off and `klatch doctor` says why; a probe that cannot tell (no `claude` on the path, an answer
//! that is not a version) turns nothing off, since "unknown" is never "unsupported" (the rule of the liveness probe).
//! Nothing is written and no environment is read.

use std::process::{Command, Stdio};
use std::time::Duration;

/// A harness version, compared field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The oldest Claude Code on which `asyncRewake` was seen to wake an idle session (RESEARCH-NOTES 47.3 ran 2.1.319
/// to 2.1.325). An older one may work too; nothing there was measured, so the watcher is not trusted on it.
pub const ASYNC_REWAKE_FLOOR: Version = Version(2, 1, 319);

/// How long `claude --version` may take.
const PATIENCE: Duration = Duration::from_secs(5);

/// The first version-looking word of `text` (`2.1.325 (Claude Code)` gives 2.1.325): three dot separated numbers, the
/// last one may carry a suffix such as `-beta`.
pub fn parse_version(text: &str) -> Option<Version> {
    text.split_whitespace().find_map(|word| {
        let mut parts = word.trim_start_matches('v').splitn(3, '.');
        let lead = |s: Option<&str>| {
            let s = s?;
            let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok().filter(|_| !digits.is_empty())
        };
        let major = lead(parts.next())?;
        let minor = lead(parts.next())?;
        let patch = lead(parts.next())?;
        Some(Version(major, minor, patch))
    })
}

/// Whether the wake watcher can be relied on, and why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    /// The harness answered with a version at or above the floor.
    Supported(Version),
    /// The harness answered with a version below the floor: the feature is off.
    Unsupported(Version),
    /// The probe could not tell, with the reason. The feature stays on.
    Unknown(String),
}

impl Support {
    /// Whether the wake watcher may run: only a probe that answered "too old" turns it off.
    pub fn wake_enabled(&self) -> bool {
        !matches!(self, Support::Unsupported(_))
    }

    /// One sentence for doctor and for the watcher's own refusal.
    pub fn say(&self) -> String {
        match self {
            Support::Supported(v) => format!(
                "claude {v} supports asyncRewake (the oldest measured is {ASYNC_REWAKE_FLOOR}), so the wake hook is on"
            ),
            Support::Unsupported(v) => format!(
                "claude {v} is older than {ASYNC_REWAKE_FLOOR}, the oldest on which asyncRewake was measured, so the wake hook is off (hooks still deliver messages at the next prompt)"
            ),
            Support::Unknown(why) => format!("{why}, so the wake hook is left as it is"),
        }
    }
}

/// What the probe's answer (`claude --version` output, or `None` when it did not answer) says about the wake watcher.
pub fn wake_support(answer: Option<&str>) -> Support {
    match answer {
        None => Support::Unknown("claude --version did not answer".into()),
        Some(text) => match parse_version(text) {
            Some(v) if v >= ASYNC_REWAKE_FLOOR => Support::Supported(v),
            Some(v) => Support::Unsupported(v),
            None => Support::Unknown("claude --version did not print a version".into()),
        },
    }
}

/// Run `claude --version` (no login, no model call, no write) with a time limit. `None` when `claude` is not on the
/// path, did not answer in time or failed to run.
pub fn probe_claude_version() -> Option<String> {
    crate::sync::within(PATIENCE, || {
        Command::new("claude")
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    })
    .flatten()
}

/// The wake watcher's support on this machine: the probe, then its reading.
pub fn wake_support_here() -> Support {
    wake_support(probe_claude_version().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_found_in_the_words_claude_prints() {
        assert_eq!(
            parse_version("2.1.325 (Claude Code)"),
            Some(Version(2, 1, 325))
        );
        assert_eq!(parse_version("claude v2.1.9"), Some(Version(2, 1, 9)));
        assert_eq!(parse_version("2.1.325-beta.1"), Some(Version(2, 1, 325)));
        assert_eq!(parse_version("2.1"), None);
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn versions_compare_numerically_not_as_text() {
        assert!(Version(2, 1, 1000) > Version(2, 1, 319));
        assert!(Version(2, 10, 0) > Version(2, 9, 999));
    }

    #[test]
    fn only_a_version_below_the_floor_turns_the_wake_hook_off() {
        assert_eq!(
            wake_support(Some("2.1.318 (Claude Code)")),
            Support::Unsupported(Version(2, 1, 318))
        );
        assert!(!wake_support(Some("2.1.318")).wake_enabled());
        assert!(wake_support(Some("2.1.319")).wake_enabled());
        assert!(wake_support(Some("3.0.0")).wake_enabled());
    }

    #[test]
    fn a_probe_that_cannot_tell_leaves_the_hook_on() {
        assert!(matches!(wake_support(None), Support::Unknown(_)));
        assert!(matches!(wake_support(Some("garbage")), Support::Unknown(_)));
        assert!(wake_support(None).wake_enabled());
        assert!(wake_support(Some("garbage")).wake_enabled());
    }

    #[test]
    fn the_sentence_names_the_version_the_floor_and_what_still_works() {
        let s = wake_support(Some("2.0.1")).say();
        assert!(s.contains("2.0.1") && s.contains("2.1.319") && s.contains("off"));
        assert!(s.contains("next prompt"));
        assert!(!s.contains('\u{2014}'));
    }
}
