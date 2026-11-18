//! A session bus whose keyring is locked and cannot ask for the password (no display: an ssh login to a desktop, a
//! lingering user service). klatch must fail promptly and say so, never wait on a dialog and never lose a key
//! (epic #387, #383). Ignored by default; `proto/linux-keys/run.sh locked` runs it.
#![cfg(target_os = "linux")]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "needs a session bus with a locked keyring; run proto/linux-keys/run.sh locked"]
fn a_locked_keyring_fails_fast_with_a_reason_and_leaves_no_key_file() {
    assert!(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
        "run proto/linux-keys/run.sh locked"
    );
    let dir = std::env::temp_dir().join(format!("klatch-locked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let started = Instant::now();
    let o = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_KEY_DIR")
        .env_remove("KLATCH_IDENTITY_FILE")
        .env_remove("DISPLAY")
        .env("HOME", dir.join("home"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("KLATCH_HOME", dir.join("rooms"))
        .env("KLATCH_NAME", "alice")
        .args(["join", "lobby"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "it waited on the keyring"
    );
    assert!(!o.status.success(), "a store was made with no keyring");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("secret-tool")
            || err.contains("keyring is locked")
            || err.contains("no default collection"),
        "no reason given: {err}"
    );
    assert!(
        !dir.join("state/klatch/keys").exists(),
        "a key file was written behind the keyring's back"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
