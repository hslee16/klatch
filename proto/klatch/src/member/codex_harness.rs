//! The Codex record at the edge (#884): the SessionStart hook writes it, and a join asks whether the process tree
//! verifies it. Decided by the owner on 2026-11-24: a verified Codex is recorded and shown as `codex`, no capability
//! is granted, klatch never asserts Codex hook trust, the walk runs only where a join is judged, and the writer is an
//! extra step of the existing SessionStart hook (its command line is unchanged).
//!
//! Reads only the process table and klatch's own registry directory. Never the Codex directory, a Codex config, a
//! credential, or the session id from the environment as proof: the id names the member, the record decides.

use super::identity::Caller;
use crate::harness::{
    CodexRecorded, OwnRegistry, PsTable, SessionFacts, codex_registry_dir, this_pid_domain,
    this_uid, verified_session, write_session_record,
};
use std::path::PathBuf;

/// klatch's home directory as `db_path` finds it (`$KLATCH_HOME`, else `$HOME/.klatch`), without making it.
fn klatch_home() -> Option<PathBuf> {
    match std::env::var_os("KLATCH_HOME") {
        Some(p) => Some(PathBuf::from(p)),
        None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".klatch")),
    }
}

/// The SessionStart hook's extra step runs here, before anything joins. The harness a join from this caller is judged by when it runs in a Codex session, else `None` (a Claude Code
/// session, a typed name and a key bundle keep the harness they had). It has the levels of the generic floor whatever
/// the walk finds; the walk, run on the first time the name is asked for, only decides whether the join is shown as
/// `codex`. Any doubt is `generic`.
pub fn codex_harness(caller: &Caller, hook_event: Option<&str>) -> Option<CodexRecorded> {
    if hook_event == Some("SessionStart") {
        record_codex_session(caller);
    }
    let session = caller.codex_session.clone()?;
    Some(CodexRecorded::new(Box::new(move || {
        let Some(home) = klatch_home() else {
            return false;
        };
        let registry = OwnRegistry {
            dir: codex_registry_dir(&home),
            uid: this_uid(),
        };
        verified_session(
            std::process::id(),
            &session,
            (&PsTable, &registry, this_pid_domain()),
        )
    })))
}

/// The session a hook may record: the one the call acts for, and only when the hook's own standard input names that
/// very session (the environment's thread id alone never does).
fn recordable<'a>(session: Option<&'a str>, stdin: Option<&str>) -> Option<&'a str> {
    session.filter(|s| Some(*s) == stdin)
}

/// The SessionStart hook's extra step: record the Codex process above this hook. Only for a hook whose own standard
/// input names the session the call acts for; never an error (a hook must not fail a session), and nothing is
/// written when no `codex` process is found above it.
fn record_codex_session(caller: &Caller) {
    let (Some(session), Some(home)) = (
        recordable(
            caller.codex_session.as_deref(),
            caller.stdin_session.as_deref(),
        ),
        klatch_home(),
    ) else {
        return;
    };
    let facts = SessionFacts {
        session_id: session,
        cwd: caller.session_dir.as_deref().unwrap_or(""),
        name: Some(&caller.principal.member),
    };
    let _ = write_session_record(
        &codex_registry_dir(&home),
        std::process::id(),
        &facts,
        (&PsTable, this_pid_domain()),
    );
}

#[cfg(test)]
mod tests {
    use super::recordable;

    #[test]
    fn only_the_session_the_hooks_own_input_names_is_recorded() {
        assert_eq!(recordable(Some("a"), Some("a")), Some("a"));
        assert_eq!(recordable(Some("a"), Some("b")), None);
        assert_eq!(recordable(Some("a"), None), None, "a thread id alone");
        assert_eq!(recordable(None, Some("a")), None);
        assert_eq!(recordable(None, None), None);
    }
}
