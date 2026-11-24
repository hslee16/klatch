//! #884: a Codex session is shown as `codex` (and not as the generic floor) only when the process tree verifies the
//! record its trusted SessionStart hook wrote in klatch's own registry directory. It grants no capability: a room that
//! requires one still refuses it. Every test uses a home of its own and a program named `codex` that is a copy of the
//! shell; none touches the real store, the real `~/.codex` or a real Codex.
#![cfg(unix)]
mod common;

use common::*;
use klatch::keys::KeySource;
use klatch::store::SqliteStore;
use klatch::store::areas::*;
use std::process::{Command, Stdio};

fn require(h: &Home, room: &str, spec: &str) {
    assert!(h.run("seed", &["join", "seed-room"]).status.success());
    let key = klatch::keys::FileKeys::new(&h.0.join("keys"))
        .get(&klatch::keys::store_key_name(&h.0))
        .unwrap()
        .expect("the binary made a key");
    let store = SqliteStore::open(&h.0.join("rooms.db"), &key).unwrap();
    store
        .policy_set(&format!("room:{room}:require"), Some(spec), 1)
        .unwrap();
}

fn registry(h: &Home) -> std::path::PathBuf {
    h.0.join("harness").join("codex")
}

/// The klatch command line as a script inside a session runs it: a home of its own, no typed name, the thread id.
fn klatch(h: &Home, thread: &str, args: &str) -> String {
    format!(
        "/usr/bin/env -u KLATCH_IDENTITY_FILE -u KLATCH_NAME HOME='{}' KLATCH_HOME='{}' KLATCH_KEY_DIR='{}' CODEX_THREAD_ID='{thread}' '{}' {args}",
        h.0.join("home").display(),
        h.0.display(),
        h.0.join("keys").display(),
        env!("CARGO_BIN_EXE_klatch")
    )
}

fn hook(h: &Home, sid: &str) -> String {
    let json = format!(
        r#"{{"session_id":"{sid}","cwd":"/work/x","hook_event_name":"SessionStart","source":"startup"}}"#
    );
    format!(
        "printf '%s' '{json}' | {}",
        klatch(h, sid, "--codex hook --event SessionStart")
    )
}

/// Run `script` as a program called `program` (a copy of the shell, so `ps` names it that); its output and success.
fn run_as(h: &Home, program: &str, script: &str) -> (bool, String) {
    let dir = h.0.join(format!("bin-{program}"));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join(program);
    let _ = std::fs::remove_file(&bin);
    std::os::unix::fs::symlink("/bin/sh", &bin).unwrap();
    let o = Command::new(&bin)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
    )
}

const SID: &str = "019a1b2c-3d4e-7f80-9a0b-1c2d3e4f5a6b";

fn gated(h: &Home, spec: &str) {
    require(h, "gated", spec);
    std::fs::create_dir_all(h.0.join("home")).unwrap();
}

fn records(h: &Home) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(registry(h))
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn the_session_start_hook_in_a_codex_process_writes_a_record_and_the_join_is_shown_as_codex() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let script = format!(
        "{}; {}",
        hook(&h, SID),
        klatch(&h, SID, "--codex join gated")
    );
    let (ok, text) = run_as(&h, "codex", &script);
    assert!(
        !ok,
        "a verified Codex must not satisfy a capability room: {text}"
    );
    assert!(text.contains("codex lacks wake"), "shown as codex: {text}");
    let recs = records(&h);
    assert_eq!(recs.len(), 1, "{recs:?}");
    let text = std::fs::read_to_string(registry(&h).join(&recs[0])).unwrap();
    assert!(text.contains(SID) && text.contains("procStart"), "{text}");
    assert!(
        !h.0.join("home").join(".codex").exists(),
        "never Codex's own state"
    );
}

#[test]
fn a_verified_codex_session_is_granted_no_capability() {
    let h = Home::new();
    for (room, cap) in [
        ("g1", "headless-worker"),
        ("g2", "session-identity"),
        ("g3", "pre-action-enforcement"),
    ] {
        require(&h, room, &format!("refuse:{cap}"));
    }
    std::fs::create_dir_all(h.0.join("home")).unwrap();
    let script = [
        hook(&h, SID),
        format!("{} || echo REFUSED-g1", klatch(&h, SID, "--codex join g1")),
        format!("{} || echo REFUSED-g2", klatch(&h, SID, "--codex join g2")),
        format!("{} || echo REFUSED-g3", klatch(&h, SID, "--codex join g3")),
    ]
    .join("; ");
    let (_, text) = run_as(&h, "codex", &script);
    for room in ["g1", "g2", "g3"] {
        assert!(text.contains(&format!("REFUSED-{room}")), "{room}: {text}");
    }
    assert!(text.contains("codex lacks"), "{text}");
}

#[test]
fn a_capability_the_floor_has_is_still_met_and_nothing_else_changes() {
    let h = Home::new();
    gated(&h, "refuse:pull-path");
    let script = format!(
        "{}; {}",
        hook(&h, SID),
        klatch(&h, SID, "--codex join gated")
    );
    let (ok, text) = run_as(&h, "codex", &script);
    assert!(ok, "{text}");
    assert!(text.contains("joined gated"), "{text}");
}

#[test]
fn a_codex_thread_id_alone_is_the_generic_floor() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let (ok, text) = run_as(&h, "codex", &klatch(&h, SID, "--codex join gated"));
    assert!(!ok && text.contains("generic lacks wake"), "{text}");
    assert!(records(&h).is_empty());
}

#[test]
fn the_hook_outside_a_codex_process_writes_no_record_and_the_join_is_the_floor() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    for program in ["sh", "node", "claude", "codex-helper"] {
        let script = format!(
            "{}; {}",
            hook(&h, SID),
            klatch(&h, SID, "--codex join gated")
        );
        let (ok, text) = run_as(&h, program, &script);
        assert!(
            !ok && text.contains("generic lacks wake"),
            "{program}: {text}"
        );
        assert!(records(&h).is_empty(), "{program}: {:?}", records(&h));
    }
}

/// A record a process of the same user writes by hand for its own pid, with the right start time.
fn forge(h: &Home, sid: &str) -> String {
    format!(
        "S=$(TZ=UTC ps -o lstart= -p $$ | tr -s ' ' | sed 's/^ //;s/ $//'); mkdir -p '{r}'; \
         printf '{{\"pid\":%s,\"sessionId\":\"{sid}\",\"cwd\":\"/w\",\"procStart\":\"%s\",\"pidDomain\":\"{d}\"}}' $$ \"$S\" > '{r}'/$$.json",
        r = registry(h).display(),
        d = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        }
    )
}

#[test]
fn a_forged_record_for_a_process_that_is_not_codex_is_not_enough() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let script = format!(
        "{}; {}",
        forge(&h, SID),
        klatch(&h, SID, "--codex join gated")
    );
    let (ok, text) = run_as(&h, "node", &script);
    assert!(!ok && text.contains("generic lacks wake"), "{text}");
    assert!(!records(&h).is_empty(), "the forgery was written");
}

#[test]
fn a_forged_record_for_a_codex_process_is_believed_as_the_claude_registry_believes_it() {
    // the documented grade (same-user evidence): the forger is inside a live codex process of its own user
    let h = Home::new();
    gated(&h, "refuse:wake");
    let script = format!(
        "{}; {}",
        forge(&h, SID),
        klatch(&h, SID, "--codex join gated")
    );
    let (ok, text) = run_as(&h, "codex", &script);
    assert!(!ok && text.contains("codex lacks wake"), "{text}");
}

#[test]
fn a_record_of_another_session_is_not_enough() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let script = format!(
        "{}; {}",
        hook(&h, SID),
        klatch(&h, "another-session", "--codex join gated")
    );
    let (ok, text) = run_as(&h, "codex", &script);
    assert!(!ok && text.contains("generic lacks wake"), "{text}");
}

#[test]
fn a_hook_whose_input_id_is_not_plain_writes_no_record() {
    // an id with anything but letters, digits, '-' and '_' is ignored (the member comes from the thread id), so the
    // hook's own input does not name the session the call acts for and nothing is recorded
    let h = Home::new();
    gated(&h, "refuse:wake");
    let json = r#"{"session_id":"bad id!","cwd":"/w","hook_event_name":"SessionStart"}"#;
    let script = format!(
        "printf '%s' '{json}' | {}",
        klatch(&h, SID, "--codex hook --event SessionStart")
    );
    let _ = run_as(&h, "codex", &script);
    assert!(records(&h).is_empty(), "{:?}", records(&h));
}

#[test]
fn a_record_that_others_can_write_is_not_believed() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let script = format!(
        "{}; chmod 666 '{}'/*.json; {}",
        hook(&h, SID),
        registry(&h).display(),
        klatch(&h, SID, "--codex join gated")
    );
    let (ok, text) = run_as(&h, "codex", &script);
    assert!(!ok && text.contains("generic lacks wake"), "{text}");
}

#[test]
fn a_missing_registry_is_the_floor_and_is_not_made_by_a_join() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let (ok, text) = run_as(&h, "codex", &klatch(&h, SID, "--codex join gated"));
    assert!(!ok && text.contains("generic lacks wake"), "{text}");
    assert!(!registry(&h).exists());
}
