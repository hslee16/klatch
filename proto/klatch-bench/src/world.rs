//! A world: one temporary klatch store and the real binary, driven as named members.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct World {
    pub bin: PathBuf,
    /// The store directory every command of this world uses (`KLATCH_HOME`).
    pub home: PathBuf,
    calls: Cell<u64>,
    clock: Cell<Option<i64>>,
    damaged: Cell<bool>,
}

/// What one klatch command did.
pub struct Run {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// What a Codex process hands klatch: a hook's JSON on standard input, or the thread id of a command it runs.
pub enum CodexInput<'a> {
    Hook(&'a str),
    Thread(&'a str),
}

impl World {
    pub fn new(bin: &Path) -> World {
        let home = std::env::temp_dir().join(format!(
            "klatch-bench-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&home).expect("create the world's store directory");
        World {
            bin: bin.to_path_buf(),
            home,
            calls: Cell::new(0),
            clock: Cell::new(None),
            damaged: Cell::new(false),
        }
    }

    /// Whether the scenario made a store (an empty file is a database that was never written).
    pub fn has_store(&self) -> bool {
        std::fs::metadata(self.home.join("rooms.db")).is_ok_and(|m| m.len() > 0)
    }

    /// A scenario that breaks the store or an archive on purpose says so (and why), so the end-of-scenario check does
    /// not fail it for the damage it made. Each use is a place where the check does not run.
    pub fn expect_damage(&self, _why: &str) {
        self.damaged.set(true);
    }

    /// Examine this world's store without changing it (#621): SQLite's page check, a gapless `seq` in every room, and
    /// an archive entry behind every tombstone. `Ok` also when the scenario never made a store.
    pub fn check_integrity(&self) -> Result<(), String> {
        use klatch::keys::{FileKeys, KeySource, store_key_name};
        if self.damaged.get() {
            return Ok(());
        }
        let db = self.home.join("rooms.db");
        if !self.has_store() {
            return Ok(());
        }
        let key = FileKeys::new(&self.home.join("keys"))
            .get(&store_key_name(&self.home))
            .map_err(|e| format!("integrity: reading the store key: {e}"))?
            .ok_or("integrity: the world has a store but no key to open it")?;
        let problems = klatch::store::check_store(&db, &key)
            .map_err(|e| format!("integrity: the store could not be examined: {e}"))?;
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!("integrity: {}", problems.join("; ")))
        }
    }

    /// Fix klatch's clock for every later command of this world (unix seconds); needs the test-clock binary.
    pub fn set_clock(&self, t: i64) {
        self.clock.set(Some(t));
    }

    /// Move klatch's clock forward (or back, with a negative number) from where it is.
    pub fn advance(&self, secs: i64) {
        self.clock.set(Some(self.clock.get().unwrap_or(0) + secs));
    }

    /// How many klatch processes this world has started.
    pub fn calls(&self) -> u64 {
        self.calls.get()
    }

    fn command(&self, who: &str, args: &[&str]) -> Command {
        self.calls.set(self.calls.get() + 1);
        let mut c = Command::new(&self.bin);
        c.env_clear()
            .env("KLATCH_HOME", &self.home)
            .env("KLATCH_KEY_DIR", self.home.join("keys"))
            .env("KLATCH_NAME", who)
            .args(args)
            .stdin(Stdio::null());
        if let Some(t) = self.clock.get() {
            c.env("KLATCH_NOW", t.to_string());
        }
        c
    }

    /// Run `klatch --codex <args>` as a Codex session would: no typed name, no registry (the home holds none), the
    /// hook's JSON on standard input, or the thread id as `CODEX_THREAD_ID` for a command (#341).
    pub fn codex(&self, args: &[&str], input: CodexInput) -> Run {
        use std::io::Write;
        let (payload, thread) = match input {
            CodexInput::Hook(json) => (json, None),
            CodexInput::Thread(id) => ("", Some(id)),
        };
        self.calls.set(self.calls.get() + 1);
        let mut c = Command::new(&self.bin);
        c.env_remove("KLATCH_IDENTITY_FILE");
        c.env_clear()
            .env("KLATCH_HOME", &self.home)
            .env("KLATCH_KEY_DIR", self.home.join("keys"))
            .env("HOME", &self.home)
            .current_dir(&self.home)
            .arg("--codex")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(t) = thread {
            c.env("CODEX_THREAD_ID", t);
        }
        let run = c.spawn().and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(payload.as_bytes())?;
            }
            child.wait_with_output()
        });
        match run {
            Ok(o) => Run {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Run {
                ok: false,
                stdout: String::new(),
                stderr: format!("could not run {}: {e}", self.bin.display()),
            },
        }
    }

    /// A launcher for running many workers at once from threads: it holds only what a command needs, so unlike the
    /// world it can be shared.
    pub fn worker_launcher(&self) -> WorkerLauncher {
        WorkerLauncher {
            bin: self.bin.clone(),
            home: self.home.clone(),
            now: self.clock.get(),
        }
    }

    /// Run `klatch <args>` as a worker holding the key bundle at `bundle`: no name and no key directory, only the
    /// path of the bundle, as a launcher would hand it.
    pub fn as_worker(&self, bundle: &Path, args: &[&str]) -> Run {
        let mut c = self.command("unused", args);
        c.env_remove("KLATCH_NAME")
            .env_remove("KLATCH_KEY_DIR")
            .env("KLATCH_IDENTITY_FILE", bundle);
        match c.output() {
            Ok(o) => Run {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Run {
                ok: false,
                stdout: String::new(),
                stderr: format!("could not start {}: {e}", self.bin.display()),
            },
        }
    }

    /// Run `klatch <args>` as member `who`.
    pub fn run(&self, who: &str, args: &[&str]) -> Run {
        match self.command(who, args).output() {
            Ok(o) => Run {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Run {
                ok: false,
                stdout: String::new(),
                stderr: format!("could not start {}: {e}", self.bin.display()),
            },
        }
    }

    /// Run `klatch admin <args>` the way a person would at a terminal (the test binary stands in for the terminal),
    /// typing `typed` when it asks for confirmation.
    pub fn admin(&self, args: &[&str], typed: &str) -> Run {
        use std::io::Write;
        let mut full = vec!["admin"];
        full.extend_from_slice(args);
        let mut c = self.command("admin", &full);
        c.env("KLATCH_ADMIN_TEST_TERMINAL", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let Ok(mut child) = c.spawn() else {
            return Run {
                ok: false,
                stdout: String::new(),
                stderr: "could not start klatch".into(),
            };
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = writeln!(stdin, "{typed}");
        }
        match child.wait_with_output() {
            Ok(o) => Run {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Run {
                ok: false,
                stdout: String::new(),
                stderr: e.to_string(),
            },
        }
    }

    /// Run a command that must succeed; its output, or an error naming the command and what it said.
    pub fn ok(&self, who: &str, args: &[&str]) -> Result<String, String> {
        let r = self.run(who, args);
        if r.ok {
            Ok(r.stdout)
        } else {
            Err(format!(
                "{who}: klatch {args:?} failed: {}",
                r.stderr.trim()
            ))
        }
    }

    /// Run a command that must be refused; what it said on standard error.
    pub fn refused(&self, who: &str, args: &[&str]) -> Result<String, String> {
        let r = self.run(who, args);
        if r.ok {
            Err(format!(
                "{who}: klatch {args:?} succeeded but should have been refused: {}",
                r.stdout.trim()
            ))
        } else {
            Ok(r.stderr)
        }
    }

    /// Speak the wire protocol (`serve --stdio`) as member `who`: send the request lines, return the response
    /// lines. One process handles all of them, which is how a scenario seeds thousands of messages quickly.
    pub fn serve(&self, who: &str, requests: &[String]) -> Result<Vec<String>, String> {
        use std::io::Write;
        let mut c = self.command(who, &["serve", "--stdio"]);
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().map_err(|e| format!("start serve: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let body: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(body.as_bytes());
        });
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let _ = writer.join();
        if !out.status.success() {
            return Err(format!(
                "serve failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(String::from)
            .collect())
    }

    /// A `post` request line for the wire protocol.
    pub fn post_request(id: usize, room: &str, body: &str) -> String {
        serde_json::json!({"v": 1, "type": "request", "id": id.to_string(), "method": "post",
            "params": {"room": room, "body": body}})
        .to_string()
    }

    /// A `read` request line (with `peek`, so it joins the member without moving a cursor).
    pub fn read_request(id: usize, room: &str) -> String {
        serde_json::json!({"v": 1, "type": "request", "id": id.to_string(), "method": "read",
            "params": {"room": room, "peek": true}})
        .to_string()
    }

    /// Start `klatch admin <args>` at the test terminal without waiting, for scenarios that run several at once. The
    /// confirmation line is already typed.
    pub fn spawn_admin(&self, args: &[&str]) -> std::io::Result<std::process::Child> {
        use std::io::Write;
        let mut full = vec!["admin"];
        full.extend_from_slice(args);
        let mut child = self
            .command("admin", &full)
            .env("KLATCH_ADMIN_TEST_TERMINAL", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = writeln!(stdin);
        }
        Ok(child)
    }

    /// Start a klatch process with a piped standard input, for scenarios that feed it by hand or kill it.
    pub fn spawn_piped(&self, who: &str, args: &[&str]) -> std::io::Result<std::process::Child> {
        self.command(who, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }

    /// Start a klatch process and return it, for concurrent scenarios.
    pub fn spawn(&self, who: &str, args: &[&str]) -> std::io::Result<std::process::Child> {
        self.command(who, args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
}

/// Runs commands as credentialed workers; cheap to clone and safe to share between threads.
#[derive(Clone)]
pub struct WorkerLauncher {
    bin: PathBuf,
    home: PathBuf,
    now: Option<i64>,
}

impl WorkerLauncher {
    /// Run `klatch <args>` as the worker whose key bundle is at `bundle`.
    pub fn run(&self, bundle: &Path, args: &[&str]) -> Run {
        let mut c = Command::new(&self.bin);
        c.env_clear()
            .env("KLATCH_HOME", &self.home)
            .env("KLATCH_IDENTITY_FILE", bundle)
            .args(args)
            .stdin(Stdio::null());
        if let Some(t) = self.now {
            c.env("KLATCH_NOW", t.to_string());
        }
        match c.output() {
            Ok(o) => Run {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Run {
                ok: false,
                stdout: String::new(),
                stderr: format!("could not start {}: {e}", self.bin.display()),
            },
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.home).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world with one room of three messages, made by the real binary.
    fn world_with_messages() -> World {
        let w = World::new(&crate::find_binary().expect("the klatch binary"));
        for i in 1..=3 {
            w.ok("alice", &["post", "lobby", &format!("message {i}")])
                .unwrap();
        }
        w
    }

    /// Run `sql` against the world's store with its key, the only way in.
    fn tamper(w: &World, sql: &str) {
        use klatch::keys::{FileKeys, KeySource, store_key_name};
        let key = FileKeys::new(&w.home.join("keys"))
            .get(&store_key_name(&w.home))
            .unwrap()
            .unwrap();
        let c = rusqlite::Connection::open(w.home.join("rooms.db")).unwrap();
        c.pragma_update(None, "key", key.sqlcipher_pragma().as_str())
            .unwrap();
        c.execute_batch(sql).unwrap();
    }

    #[test]
    fn a_world_with_no_store_passes() {
        let w = World::new(&PathBuf::from("unused"));
        assert_eq!(w.check_integrity(), Ok(()));
    }

    #[test]
    fn a_store_made_by_the_binary_passes() {
        assert_eq!(world_with_messages().check_integrity(), Ok(()));
    }

    #[test]
    fn a_missing_seq_fails_the_check() {
        let w = world_with_messages();
        tamper(&w, "DELETE FROM messages WHERE seq=2");
        let e = w.check_integrity().unwrap_err();
        assert!(e.contains("seq of room lobby"), "{e}");
    }

    #[test]
    fn a_tombstone_with_no_archive_fails_the_check() {
        let w = world_with_messages();
        tamper(&w, "UPDATE messages SET kind='archived' WHERE seq=1");
        let e = w.check_integrity().unwrap_err();
        assert!(e.contains("tombstone lobby#1"), "{e}");
    }

    #[test]
    fn a_scenario_that_damages_the_store_on_purpose_is_not_failed_for_it() {
        let w = world_with_messages();
        tamper(&w, "DELETE FROM messages WHERE seq=2");
        w.expect_damage("test");
        assert_eq!(w.check_integrity(), Ok(()));
    }

    #[test]
    fn a_store_whose_key_is_gone_cannot_be_examined_and_fails() {
        let w = world_with_messages();
        std::fs::remove_dir_all(w.home.join("keys")).unwrap();
        assert!(w.check_integrity().is_err());
    }
}
