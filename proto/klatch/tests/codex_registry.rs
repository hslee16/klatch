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
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Command, Stdio};

fn require(h: &Home, (room, spec): (&str, &str)) {
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
fn klatch(h: &Home, (thread, args): (&str, &str)) -> String {
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
        klatch(h, (sid, "--codex hook --event SessionStart"))
    )
}

/// Run `script` as a program called `program` (a copy of the shell, so `ps` names it that); its output and success.
/// A script that does not finish in `BOUND` is killed (only the process started here) and fails the test.
fn run_as(h: &Home, (program, script): (&str, &str)) -> (bool, String) {
    use std::time::{Duration, Instant};
    const BOUND: Duration = Duration::from_secs(60);
    let dir = h.0.join(format!("bin-{program}"));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join(program);
    let _ = std::fs::remove_file(&bin);
    std::os::unix::fs::symlink("/bin/sh", &bin).unwrap();
    let mut child = Command::new(&bin)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > BOUND {
            let _ = child.kill();
            // release anything stuck reading a FIFO of the registry, so no process is left behind
            for entry in std::fs::read_dir(registry(h))
                .into_iter()
                .flatten()
                .flatten()
            {
                let _ = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(entry.path());
            }
            panic!("the script did not finish within {BOUND:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let o = child.wait_with_output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
    )
}

/// The join failed and was judged as the floor (shown as `generic`, which lacks `wake`).
fn assert_floor((ok, text): &(bool, String)) {
    assert!(!ok && text.contains("generic lacks wake"), "{text}");
}

const SID: &str = "019a1b2c-3d4e-7f80-9a0b-1c2d3e4f5a6b";

fn gated(h: &Home, spec: &str) {
    require(h, ("gated", spec));
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
        klatch(&h, (SID, "--codex join gated"))
    );
    let (ok, text) = run_as(&h, ("codex", &script));
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
        require(&h, (room, &format!("refuse:{cap}")));
    }
    std::fs::create_dir_all(h.0.join("home")).unwrap();
    let script = [
        hook(&h, SID),
        format!(
            "{} || echo REFUSED-g1",
            klatch(&h, (SID, "--codex join g1"))
        ),
        format!(
            "{} || echo REFUSED-g2",
            klatch(&h, (SID, "--codex join g2"))
        ),
        format!(
            "{} || echo REFUSED-g3",
            klatch(&h, (SID, "--codex join g3"))
        ),
    ]
    .join("; ");
    let (_, text) = run_as(&h, ("codex", &script));
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
        klatch(&h, (SID, "--codex join gated"))
    );
    let (ok, text) = run_as(&h, ("codex", &script));
    assert!(ok, "{text}");
    assert!(text.contains("joined gated"), "{text}");
}

#[test]
fn a_codex_thread_id_alone_is_the_generic_floor() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let (ok, text) = run_as(&h, ("codex", &klatch(&h, (SID, "--codex join gated"))));
    assert_floor(&(ok, text));
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
            klatch(&h, (SID, "--codex join gated"))
        );
        let (ok, text) = run_as(&h, (program, &script));
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
        klatch(&h, (SID, "--codex join gated"))
    );
    let (ok, text) = run_as(&h, ("node", &script));
    assert_floor(&(ok, text));
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
        klatch(&h, (SID, "--codex join gated"))
    );
    let (ok, text) = run_as(&h, ("codex", &script));
    assert!(!ok && text.contains("codex lacks wake"), "{text}");
}

#[test]
fn a_record_of_another_session_is_not_enough() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let script = format!(
        "{}; {}",
        hook(&h, SID),
        klatch(&h, ("another-session", "--codex join gated"))
    );
    let (ok, text) = run_as(&h, ("codex", &script));
    assert_floor(&(ok, text));
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
        klatch(&h, (SID, "--codex hook --event SessionStart"))
    );
    let _ = run_as(&h, ("codex", &script));
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
        klatch(&h, (SID, "--codex join gated"))
    );
    let (ok, text) = run_as(&h, ("codex", &script));
    assert_floor(&(ok, text));
}

#[test]
fn a_missing_registry_is_the_floor_and_is_not_made_by_a_join() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let (ok, text) = run_as(&h, ("codex", &klatch(&h, (SID, "--codex join gated"))));
    assert_floor(&(ok, text));
    assert!(!registry(&h).exists());
}

#[test]
fn hostile_entries_in_the_registry_do_not_stop_the_hook() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let r = registry(&h);
    std::fs::create_dir_all(&r).unwrap();
    let fifo = std::ffi::CString::new(r.join("999.json").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::fs::write(r.join("997.json"), vec![b'x'; 200 << 10]).unwrap();
    std::fs::create_dir(r.join("996.json")).unwrap();
    let big = h.0.join("big");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(16 << 20)
        .unwrap();
    std::os::unix::fs::symlink(&big, r.join("998.json")).unwrap();
    let script = format!(
        "{}; {}",
        hook(&h, SID),
        klatch(&h, (SID, "--codex join gated"))
    );
    let (ok, text) = run_as(&h, ("codex", &script));
    assert!(!ok && text.contains("codex lacks wake"), "{text}");
    assert!(
        std::fs::read_dir(&r).unwrap().count() >= 5,
        "the hostile entries are left alone"
    );
}

/// A `ps` that logs each call and runs the real one: how many times, and what, a command looks at the process table.
fn counting_ps(h: &Home) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let shim = h.0.join("shim");
    std::fs::create_dir_all(&shim).unwrap();
    let log = h.0.join("ps.log");
    std::fs::write(
        shim.join("ps"),
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nexec /bin/ps \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(shim.join("ps"), std::fs::Permissions::from_mode(0o755)).unwrap();
    (shim, log)
}

fn with_path(cmd: String, shim: &std::path::Path) -> String {
    cmd.replacen(
        "HOME=",
        &format!(
            "KLATCH_TEST_SEAMS=1 KLATCH_TEST_PS='{}' HOME=",
            shim.join("ps").display()
        ),
        1,
    )
}

fn ps_calls(log: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

#[test]
fn the_walk_is_lazy_and_runs_once() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let (shim, log) = counting_ps(&h);
    let admitted = with_path(klatch(&h, (SID, "--codex join open-room")), &shim);
    let (ok, text) = run_as(&h, ("codex", &admitted));
    assert!(ok, "{text}");
    assert!(
        ps_calls(&log).is_empty(),
        "a join that is admitted made a walk: {:?}",
        ps_calls(&log)
    );
    let refused = with_path(klatch(&h, (SID, "--codex join gated")), &shim);
    let (ok, text) = run_as(&h, ("codex", &refused));
    assert!(!ok && text.contains("lacks wake"), "{text}");
    let calls = ps_calls(&log);
    assert!(
        !calls.is_empty(),
        "a refused join looks at the process table"
    );
    assert_eq!(
        calls.len(),
        1,
        "one walk reads the whole process table once: {calls:?}"
    );
}

#[test]
fn the_ps_stand_in_is_not_read_without_the_second_variable() {
    // a debug binary run for real must not take KLATCH_TEST_PS by default (#1629): the shim stays unused
    let h = Home::new();
    gated(&h, "refuse:wake");
    let (shim, log) = counting_ps(&h);
    let cmd = klatch(&h, (SID, "--codex join gated")).replacen(
        "HOME=",
        &format!("KLATCH_TEST_PS='{}' HOME=", shim.join("ps").display()),
        1,
    );
    let (ok, text) = run_as(&h, ("codex", &cmd));
    assert!(!ok && text.contains("lacks wake"), "{text}");
    assert!(
        ps_calls(&log).is_empty(),
        "the stand-in ps was run without KLATCH_TEST_SEAMS: {:?}",
        ps_calls(&log)
    );
}

#[test]
fn a_live_sibling_codex_that_is_not_an_ancestor_does_not_verify_the_caller() {
    // a perfect record for another live codex process (same session id) beside the caller, not above it
    let h = Home::new();
    gated(&h, "refuse:wake");
    let sibling_bin = h.0.join("bin-codex").join("codex");
    std::fs::create_dir_all(sibling_bin.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&sibling_bin);
    std::os::unix::fs::symlink("/bin/sh", &sibling_bin).unwrap();
    let forge_script = h.0.join("forge.sh");
    std::fs::write(
        &forge_script,
        format!("{}\nsleep 8 >/dev/null 2>&1 & wait\n", forge(&h, SID)),
    )
    .unwrap();
    let script = format!(
        "'{bin}' '{f}' & S=$!; i=0; while [ ! -e '{r}'/$S.json ] && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; {join}; R=$?; kill -9 $S; exit $R",
        bin = sibling_bin.display(),
        f = forge_script.display(),
        r = registry(&h).display(),
        join = klatch(&h, (SID, "--codex join gated")),
    );
    let out = run_as(&h, ("node", &script));
    assert_floor(&out);
    assert!(!records(&h).is_empty(), "the sibling wrote its record");
}

#[test]
fn no_environment_variable_names_the_program_or_the_machine() {
    let h = Home::new();
    gated(&h, "refuse:wake");
    let extra = "CODEX_PROGRAM=node KLATCH_CODEX_PROGRAM=node KLATCH_PROGRAM=node KLATCH_HARNESS=codex KLATCH_PID_DOMAIN=linux PROGRAM=node ";
    let cmd = klatch(&h, (SID, "--codex join gated")).replacen(
        "CODEX_THREAD_ID=",
        &format!("{extra}CODEX_THREAD_ID="),
        1,
    );
    let script = format!("{}; {cmd}", forge(&h, SID));
    assert_floor(&run_as(&h, ("node", &script)));
}

#[test]
fn the_codex_modules_read_the_environment_only_for_klatchs_home() {
    let record = include_str!("../src/harness/codex_record.rs");
    let own = include_str!("../src/harness/own_records.rs");
    let edge = include_str!("../src/member/codex_harness.rs");
    for (name, src) in [("codex_record.rs", record), ("own_records.rs", own)] {
        assert!(
            !src.contains("env::") && !src.contains("getenv"),
            "{name} reads the environment"
        );
    }
    let reads: Vec<&str> = edge.lines().filter(|l| l.contains("env::")).collect();
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert!(
        reads
            .iter()
            .all(|l| l.contains("\"KLATCH_HOME\"") || l.contains("\"HOME\"")),
        "{reads:?}"
    );
    assert!(record.contains("pub const CODEX_PROGRAM: &str = \"codex\""));
}
