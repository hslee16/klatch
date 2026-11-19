//! How a member is woken, chosen from the permission mode its hooks last reported (#677, DESIGN item 57, #553).
//!
//! Claude Code holds a socket nudge to a session that bypasses permission prompts and drops it after about five
//! minutes (RESEARCH-NOTES 47.2), so a member in `bypassPermissions`, or one that has reported no mode, is woken by
//! its hooks and the wake watcher (`klatch init --wake`) and never counted on for a nudge. Only the modes that were
//! measured to prompt may get one. The mode is self-reported and as new as the last hook: this picks a wake path, it
//! is never a security control.

/// The modes measured to prompt, so a socket nudge reaches the session.
pub const PROMPTING_MODES: [&str; 5] = ["default", "acceptEdits", "plan", "auto", "dontAsk"];

/// How a member is woken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakePath {
    /// A content-free socket nudge (Tier 0, with consent) may be tried, and the hooks still deliver.
    Nudge,
    /// The hook and the wake watcher only: a nudge would be held and dropped, or may be.
    HookAndWatcher,
}

impl WakePath {
    /// The word `klatch who` shows.
    pub fn word(self) -> &'static str {
        match self {
            WakePath::Nudge => "nudge",
            WakePath::HookAndWatcher => "hook",
        }
    }
}

/// The wake path for a member whose last reported mode is `mode`. No mode (its hooks never ran) or a mode that is
/// not known to prompt, `bypassPermissions` included, gets the hook and the watcher.
pub fn wake_path(mode: Option<&str>) -> WakePath {
    match mode {
        Some(m) if PROMPTING_MODES.contains(&m) => WakePath::Nudge,
        _ => WakePath::HookAndWatcher,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_that_prompt_may_be_nudged() {
        for m in PROMPTING_MODES {
            assert_eq!(wake_path(Some(m)), WakePath::Nudge, "{m}");
        }
    }

    #[test]
    fn bypass_unknown_and_missing_modes_rely_on_the_hook_and_watcher() {
        assert_eq!(
            wake_path(Some("bypassPermissions")),
            WakePath::HookAndWatcher
        );
        assert_eq!(wake_path(Some("somethingNew")), WakePath::HookAndWatcher);
        assert_eq!(wake_path(None), WakePath::HookAndWatcher);
    }
}
