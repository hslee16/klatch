//! A Linux box with no session bus (a server, an ssh login, a CI runner): the key lives in a 0600 file with a
//! warning, nothing prompts and nothing hangs (epic #387, #383). These run on every Linux host; they strip the bus
//! address and use a throwaway home, so the host's keyring is never asked. `proto/linux-keys/run.sh nobus` runs them
//! in a container that has `secret-tool` but no bus, and `bare` in one that has neither.
#![cfg(target_os = "linux")]

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Box_ {
    dir: PathBuf,
}

impl Box_ {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("klatch-headless-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn state_keys(&self) -> PathBuf {
        self.dir.join("state/klatch/keys")
    }

    /// The binary as a hook runs it: no terminal, no bus, no key directory, a home of its own. `extra` is added
    /// to the environment (a stale bus address, for example).
    fn run(&self, who: &str, args: &[&str], extra: &[(&str, &str)]) -> (bool, String, String) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_klatch"));
        cmd.env_remove("KLATCH_IDENTITY_FILE");
        cmd.env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env_remove("KLATCH_KEY_DIR")
            .env_remove("KLATCH_IDENTITY_FILE")
            .env_remove("DISPLAY")
            .env("HOME", self.dir.join("home"))
            .env("XDG_STATE_HOME", self.dir.join("state"))
            .env("KLATCH_HOME", self.dir.join("rooms"))
            .env("KLATCH_NAME", who)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        std::fs::create_dir_all(self.dir.join("home")).unwrap();
        let child = cmd.spawn().unwrap();
        let out = within(Duration::from_secs(60), child);
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn ok(&self, who: &str, args: &[&str]) -> String {
        let (ok, out, err) = self.run(who, args, &[]);
        assert!(ok, "{args:?} failed: {err}");
        out
    }
}

impl Drop for Box_ {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Wait for a child, and fail the test (killing it) when it has not finished: a hang is the failure to catch.
fn within(limit: Duration, mut child: std::process::Child) -> std::process::Output {
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if start.elapsed() > limit {
            child.kill().unwrap();
            panic!("klatch did not finish within {limit:?}: it hung or prompted");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn key_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| d.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default()
}

#[test]
fn with_no_bus_the_key_is_a_0600_file_in_a_0700_directory() {
    let b = Box_::new("file");
    b.ok("alice", &["join", "lobby"]);
    b.ok("alice", &["post", "lobby", "kept under a file key"]);
    let keys = key_files(&b.state_keys());
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert_eq!(std::fs::metadata(&keys[0]).unwrap().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&keys[0]).unwrap().len(), 32);
    assert_eq!(
        std::fs::metadata(b.state_keys()).unwrap().mode() & 0o777,
        0o700
    );
}

#[test]
fn a_store_made_with_the_file_key_opens_again_in_a_new_process() {
    let b = Box_::new("reopen");
    b.ok("alice", &["join", "lobby"]);
    b.ok("alice", &["post", "lobby", "first"]);
    b.ok("bob", &["join", "lobby"]);
    assert!(b.ok("bob", &["read", "lobby", "--peek"]).contains("first"));
}

#[test]
fn doctor_says_the_key_is_in_a_file_and_gives_the_warning() {
    let b = Box_::new("warn");
    b.ok("alice", &["join", "lobby"]);
    let (_, out, err) = b.run("alice", &["doctor"], &[]);
    let all = format!("{out}{err}");
    assert!(
        all.contains("no secret service found: the key is kept in a file readable only by you"),
        "{all}"
    );
    assert!(all.contains("the key is in a key file"), "{all}");
}

#[test]
fn a_hook_with_no_bus_neither_prompts_nor_hangs_nor_prints_the_warning_into_the_context() {
    let b = Box_::new("hook");
    b.ok("alice", &["join", "lobby"]);
    b.ok("bob", &["post", "lobby", "ping"]);
    let started = Instant::now();
    let out = b.ok("alice", &["hook", "--event", "PostToolUse"]);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(out.contains("ping"), "{out}");
    assert!(!out.contains("secret service"), "the warning leaked: {out}");
}

#[test]
fn a_stale_bus_address_falls_back_to_the_file_instead_of_failing() {
    // an ssh login or a cron job can inherit an address whose bus is long gone
    let b = Box_::new("stale");
    let stale = [("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus")];
    let (ok, _, err) = b.run("alice", &["join", "lobby"], &stale);
    assert!(ok, "join failed on a stale bus address: {err}");
    let (ok, _, err) = b.run("alice", &["post", "lobby", "hello"], &stale);
    assert!(ok, "post failed on a stale bus address: {err}");
    assert_eq!(key_files(&b.state_keys()).len(), 1);
}
