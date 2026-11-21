//! #877: a room that requires a harness capability admits a genuine Claude Code session that has it, and nothing else.
//! The gateway learns the harness from evidence it checked itself: the session found above the process through the
//! harness registry and a live process whose program is `claude` (the same walk that names the member). A typed name,
//! a remembered session, a key bundle and Codex are not that evidence, so each of them is the generic floor.
//! Every test uses a home of its own and a registry of its own; none touches the real store or the real harness.
#![cfg(unix)]
mod common;

use common::*;
use klatch::keys::KeySource;
use klatch::store::SqliteStore;
use klatch::store::areas::*;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const REQUIRE: &str = "refuse:context-injection,pre-action-enforcement";

fn domain() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

fn lstart(pid: u32) -> String {
    let out = Command::new("ps")
        .env("TZ", "UTC")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Put the policy key the human's `klatch admin room-require` writes (admin needs a terminal, which a test has not).
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

/// A program named `claude` (a symlink to the shell) running `script`, with a registry record for it under `fake`.
fn session(h: &Home, fake: &Path, script: &str, sid: &str) -> Child {
    let bin = h.0.join("claude");
    let _ = std::fs::remove_file(&bin);
    std::os::unix::fs::symlink("/bin/sh", &bin).unwrap();
    // The script waits for the record to be written, so klatch never looks before the registry has it.
    let ready = h.0.join(format!("ready-{sid}"));
    let script = format!(
        "while [ ! -f '{}' ]; do sleep 0.02; done; {script}",
        ready.display()
    );
    let child = Command::new(&bin)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let dir = fake.join(".claude").join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let pid = child.id();
    for _ in 0..100 {
        if !lstart(pid).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let record = format!(
        r#"{{"pid":{pid},"sessionId":"{sid}","cwd":"/w","name":"claude-joiner","procStart":"{}","pidDomain":"{}"}}"#,
        lstart(pid),
        domain()
    );
    std::fs::write(dir.join(format!("{pid}.json")), record).unwrap();
    std::fs::write(ready, "").unwrap();
    child
}

/// The klatch command line as the shell script of a session runs it, with the environment a real session has: a
/// home of its own, no typed name unless `name` gives one.
fn klatch_in_session(h: &Home, fake: &Path, name: Option<&str>, args: &str) -> String {
    let name = name.map_or("-u KLATCH_NAME".to_string(), |n| format!("KLATCH_NAME={n}"));
    format!(
        "env -u KLATCH_IDENTITY_FILE {name} HOME='{}' KLATCH_HOME='{}' KLATCH_KEY_DIR='{}' '{}' {args}",
        fake.display(),
        h.0.display(),
        h.0.join("keys").display(),
        env!("CARGO_BIN_EXE_klatch")
    )
}

fn finish(child: Child) -> (bool, String) {
    let o = child.wait_with_output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
    )
}

fn fake_home(h: &Home) -> PathBuf {
    let f = h.0.join("fake-home");
    std::fs::create_dir_all(&f).unwrap();
    f
}

#[test]
fn a_genuine_claude_code_session_joins_a_room_that_requires_what_it_has() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let fake = fake_home(&h);
    let s = session(
        &h,
        &fake,
        &klatch_in_session(&h, &fake, None, "--discover join gated"),
        "sess-gate-1",
    );
    let (ok, text) = finish(s);
    assert!(ok, "a real session was refused: {text}");
    assert!(text.contains("joined gated"), "{text}");
}

#[test]
fn a_typed_name_is_the_generic_floor_even_inside_a_real_session() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let fake = fake_home(&h);
    let s = session(
        &h,
        &fake,
        &klatch_in_session(&h, &fake, Some("typed"), "--discover join gated"),
        "sess-gate-2",
    );
    let (ok, text) = finish(s);
    assert!(!ok, "a typed name was admitted: {text}");
    assert!(
        text.contains("harness-capability") || text.contains("requires"),
        "{text}"
    );
    assert!(text.contains("generic"), "{text}");
}

#[test]
fn a_caller_with_no_session_above_it_is_the_generic_floor() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let err = h.fail("alice", &["join", "gated"]);
    assert!(err.contains("generic") && err.contains("requires"), "{err}");
}

#[test]
fn a_room_that_requires_nothing_still_admits_a_typed_name() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    assert!(h.run("alice", &["join", "open"]).status.success());
}

#[test]
fn a_hook_for_the_session_that_started_it_joins_and_a_hook_for_another_session_does_not() {
    let h = Home::new();
    require(&h, "lobby", REQUIRE);
    let fake = fake_home(&h);
    // SessionStart for the session the registry names: the lobby requires what Claude Code has, so it joins.
    let cmd = format!(
        "printf '%s' '{{\"session_id\":\"sess-hook-1\",\"hook_event_name\":\"SessionStart\"}}' | {}",
        klatch_in_session(&h, &fake, None, "--discover hook --event SessionStart")
    );
    let (ok, text) = finish(session(&h, &fake, &cmd, "sess-hook-1"));
    assert!(ok, "{text}");
    assert!(
        text.contains("joined lobby") || text.contains("back in lobby"),
        "{text}"
    );
    // The same registry record, but the hook was started for a different session id: that is the remembered path,
    // which is a typed-name grade of evidence, so it is the generic floor and the lobby refuses it.
    let cmd = format!(
        "printf '%s' '{{\"session_id\":\"sess-other\",\"hook_event_name\":\"SessionStart\"}}' | {}",
        klatch_in_session(&h, &fake, None, "--discover hook --event SessionStart")
    );
    let (_, text) = finish(session(&h, &fake, &cmd, "sess-hook-2"));
    assert!(!text.contains("joined lobby"), "{text}");
}

#[test]
fn a_wake_requirement_is_met_only_by_a_measured_version() {
    let h = Home::new();
    require(&h, "gated", "refuse:wake");
    let fake = fake_home(&h);
    // No `claude` on the PATH of the process, so the version probe cannot tell: wake is not satisfied.
    let cmd = klatch_in_session(&h, &fake, None, "--discover join gated");
    let cmd = format!("PATH=/usr/bin:/bin; export PATH; {cmd}");
    let (ok, text) = finish(session(&h, &fake, &cmd, "sess-wake-1"));
    assert!(!ok, "{text}");
    assert!(text.contains("wake"), "{text}");
}

#[test]
fn a_wake_requirement_is_met_by_a_claude_new_enough_to_have_it() {
    let h = Home::new();
    require(&h, "gated", "refuse:wake");
    let fake = fake_home(&h);
    let bin_dir = h.0.join("fakebin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let claude = bin_dir.join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\nprintf '%s\\n' '2.1.325 (Claude Code)'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cmd = klatch_in_session(&h, &fake, None, "--discover join gated");
    let cmd = format!(
        "PATH='{}':/usr/bin:/bin; export PATH; {cmd}",
        bin_dir.display()
    );
    let (ok, text) = finish(session(&h, &fake, &cmd, "sess-wake-2"));
    assert!(ok, "{text}");
}

/// `klatch serve --stdio` in a session, fed one `join gated` request on standard input.
fn serve_join(h: &Home, fake: &Path, name: Option<&str>, sid: &str) -> (bool, String) {
    let frame = request("j", "join", r#"{"room":"gated"}"#);
    let serve = klatch_in_session(h, fake, name, "--discover serve --stdio");
    let cmd = format!("printf '%s\\n' '{frame}' | {serve}");
    finish(session(h, fake, &cmd, sid))
}

#[test]
fn serve_stdio_in_a_genuine_claude_code_session_joins_a_room_that_requires_what_it_has() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let fake = fake_home(&h);
    let (_, text) = serve_join(&h, &fake, None, "sess-serve-1");
    assert!(text.contains(r#""ok":true"#), "{text}");
    assert!(!text.contains("generic"), "{text}");
}

#[test]
fn serve_stdio_with_a_typed_name_is_the_generic_floor_even_inside_a_real_session() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let fake = fake_home(&h);
    let (_, text) = serve_join(&h, &fake, Some("typed"), "sess-serve-2");
    assert!(
        !text.contains(r#""ok":true"#),
        "a typed name was admitted: {text}"
    );
    assert!(text.contains("generic"), "{text}");
}

/// `klatch mcp --stdio` in a session, fed `initialize` and one `klatch_join` tool call on standard input.
fn mcp_join(h: &Home, fake: &Path, name: Option<&str>, sid: &str) -> (bool, String) {
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"klatch_join","arguments":{"room":"gated"}}}"#;
    let mcp = klatch_in_session(h, fake, name, "--discover mcp --stdio");
    let cmd = format!("printf '%s\\n%s\\n' '{init}' '{call}' | {mcp}");
    finish(session(h, fake, &cmd, sid))
}

#[test]
fn mcp_stdio_in_a_genuine_claude_code_session_joins_a_room_that_requires_what_it_has() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let fake = fake_home(&h);
    let (_, text) = mcp_join(&h, &fake, None, "sess-mcp-1");
    assert!(text.contains(r#""id":2"#), "{text}");
    assert!(!text.contains(r#""isError":true"#), "{text}");
    assert!(!text.contains("generic"), "{text}");
}

#[test]
fn mcp_stdio_with_a_typed_name_is_the_generic_floor_even_inside_a_real_session() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let fake = fake_home(&h);
    let (_, text) = mcp_join(&h, &fake, Some("typed"), "sess-mcp-2");
    assert!(text.contains(r#""isError":true"#), "{text}");
    assert!(text.contains("generic"), "{text}");
}

#[test]
fn serve_stdio_with_no_session_above_it_is_the_generic_floor() {
    let h = Home::new();
    require(&h, "gated", REQUIRE);
    let frame = request("j", "join", r#"{"room":"gated"}"#);
    let frames = h.serve("alice", &[&frame]);
    let text = format!("{frames:?}");
    assert!(text.contains("generic"), "{text}");
}
