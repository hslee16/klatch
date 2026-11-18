//! The checks about the machine around klatch: Codex, the coding-agent session this runs in, and the harness registry
//! with its liveness probe.

use super::{Report, Status};
use klatch::harness::PsTable;
use std::path::Path;

/// Codex: whether klatch's hooks in its config are trusted (an untrusted hook is skipped silently, #544) and whether
/// `codex queue` exists (#545). Reads the Codex files and runs `codex --version` and `codex queue --help`, which need
/// no login; writes nothing.
pub(super) fn check_codex(r: &mut Report, home: Option<&Path>, project_dir: &Path) {
    let found = crate::codex::codex_home(std::env::var_os("CODEX_HOME"), home)
        .map(|dir| crate::codex::hook_findings(&dir, Some(project_dir)))
        .unwrap_or_default()
        .into_iter()
        .chain(std::iter::once(crate::codex::queue_finding()));
    for f in found {
        let status = if f.ok { Status::Ok } else { Status::Warn };
        r.add(status, f.line, f.fix.as_deref());
    }
}

/// The harness probe (#482): what `claude --version` says about the features that depend on the harness version.
/// A version below the floor is a warning (the wake hook is off, the rest works); no `claude` on the path is a plain
/// ok line; an answer that is no version is a warning that turns nothing off.
pub(super) fn check_harness(r: &mut Report) {
    use klatch::harness::{Support, probe_claude_version, wake_support};
    let answer = probe_claude_version();
    let support = wake_support(answer.as_deref());
    match (&support, answer) {
        (Support::Supported(_), _) => r.add(Status::Ok, format!("harness: {}", support.say()), None),
        (Support::Unsupported(_), _) => r.add(
            Status::Warn,
            format!("harness: {}", support.say()),
            Some("upgrade Claude Code (the wake hook turns itself on again once the version is high enough)"),
        ),
        (Support::Unknown(_), None) => r.add(
            Status::Ok,
            "harness: claude is not on the PATH or did not answer `claude --version` (nothing to check; the wake hook is left as it is)",
            None,
        ),
        (Support::Unknown(_), Some(_)) => r.add(
            Status::Warn,
            format!("harness: {}", support.say()),
            Some("run `claude --version` yourself; klatch reads the first x.y.z in its answer"),
        ),
    }
}

pub(super) fn check_session(r: &mut Report) {
    match crate::identity::discover_session() {
        Ok(found) => r.add(
            Status::Ok,
            format!(
                "session: this process runs in the session {}",
                found.name.as_deref().unwrap_or(&found.session_id)
            ),
            None,
        ),
        Err(_) => r.add(
            Status::Ok,
            "session: not inside a coding-agent session (nothing to check)",
            None,
        ),
    }
}

pub(super) fn check_registry(r: &mut Report) {
    let Some((registry, domain)) = crate::identity::registry_and_domain() else {
        return;
    };
    if !registry.dir.exists() {
        r.add(
            Status::Ok,
            format!(
                "registry: no harness registry at {} (Claude Code has not run here)",
                registry.dir.display()
            ),
            None,
        );
        return;
    }
    let found = klatch::harness::list(&registry, &PsTable, domain);
    r.add(
        Status::Ok,
        format!(
            "registry: {} live session(s) (klatch sessions lists them)",
            found.live.len()
        ),
        None,
    );
    check_liveness(r, &found.live, domain);
}

/// Which layers of the liveness probe (DESIGN section 43) can be read on this machine, tried on the live sessions.
fn check_liveness(r: &mut Report, live: &[klatch::harness::Record], domain: &str) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let host = klatch::harness::SysHost::standard(Path::new(&home));
    let test = klatch::harness::self_test(live, &PsTable, domain, &host);
    if test.sessions == 0 {
        r.add(
            Status::Ok,
            "liveness: no live session to try the probe on",
            None,
        );
        return;
    }
    let works: Vec<String> = test
        .layers
        .iter()
        .map(|l| format!("{} {}/{}", l.layer, l.readable, test.sessions))
        .collect();
    let line = format!("liveness probe layers readable: {}", works.join(", "));
    let broken = test.broken();
    if broken.is_empty() {
        r.add(Status::Ok, line, None);
        return;
    }
    r.add(
        Status::Warn,
        format!(
            "{line}; unreadable for every session: {} (a Claude Code change to the registry or transcript layout?)",
            broken.join(", ")
        ),
        Some(
            "nothing breaks: an unreadable layer counts as unknown, never as a dead session, and leases fall back on their time limits; report the layer on the klatch tracker (#311)",
        ),
    );
}
