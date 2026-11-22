//! A world: one temporary klatch store and the real binary, driven as named members.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct World {
    pub bin: PathBuf,
    /// The store directory every command of this world uses (`KLATCH_HOME`).
    pub home: PathBuf,
    /// Holds the directory: removed with the world, also on a panic, and capped in size (#995).
    _dir: crate::test_dir::TestDir,
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
        let dir = crate::test_dir::TestDir::new("bench");
        let home = dir.to_path_buf();
        World {
            _dir: dir,
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
        // a running process reads this file at every call, so it sees the clock move (#1051)
        let _ = std::fs::write(self.home.join("klatch-now"), t.to_string());
    }

    /// Move klatch's clock forward (or back, with a negative number) from where it is.
    pub fn advance(&self, secs: i64) {
        self.set_clock(self.clock.get().unwrap_or(0) + secs);
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
            c.env("KLATCH_NOW", t.to_string())
                .env("KLATCH_NOW_FILE", self.home.join("klatch-now"));
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

    /// Issue a credential for `member` inside this world's own temporary store, as a launcher would (`klatch worker
    /// issue`), and return the path of the key bundle it wrote under the world's home. `life` is the idle timeout and
    /// the optional most seconds from issue. The credential exists only in this world: its store, its key directory
    /// and its bundle are all temporary, and nothing of the live keychain or `~/.klatch` is read or written. Run
    /// commands as the worker with `as_worker`.
    pub fn issue_worker(
        &self,
        member: &str,
        clearance: &str,
        life: (i64, Option<i64>),
    ) -> Result<PathBuf, String> {
        let out = self.home.join(format!("{member}.bundle"));
        let (ttl, max_life) = life;
        let (ttl, out_s) = (ttl.to_string(), out.to_string_lossy().into_owned());
        let mut args = vec![
            "worker",
            "issue",
            member,
            "--clearance",
            clearance,
            "--ttl",
            &ttl,
            "--parent",
            "lead",
            "--out",
            &out_s,
        ];
        let cap = max_life.map(|m| m.to_string());
        if let Some(c) = &cap {
            args.extend(["--max-life", c]);
        }
        // a launcher command asks for the admin passphrase at a terminal (#1233)
        self.set_passphrase();
        let (ok, text) = crate::pty::run_on_pty(
            self.command("launcher", &args),
            vec![crate::pty::PASS.to_string()],
        );
        if !ok {
            return Err(format!("klatch {} failed: {}", args.join(" "), text.trim()));
        }
        Ok(out)
    }

    /// Start `klatch <args>` as the worker holding `bundle`, with piped standard input and output, for a process that
    /// proves its credential once at the start and stays up (`serve --stdio`) while the scenario ends the credential.
    pub fn spawn_worker_piped(
        &self,
        bundle: &Path,
        args: &[&str],
    ) -> std::io::Result<std::process::Child> {
        let mut c = self.command("unused", args);
        c.env_remove("KLATCH_NAME")
            .env_remove("KLATCH_KEY_DIR")
            .env("KLATCH_IDENTITY_FILE", bundle)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }

    /// Run `klatch <args>` as member `who`.
    pub fn run(&self, who: &str, args: &[&str]) -> Run {
        // the launcher's commands ask for the admin passphrase at a terminal (#1233)
        if who == "launcher"
            && matches!(args.first(), Some(&("worker" | "anchor" | "run" | "judge")))
        {
            return self.launcher(args);
        }
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

    /// Set the admin passphrase of this world's store once (typed twice at a pseudo-terminal), as a person does after
    /// installing (#1233). Every admin and launcher command asks for it.
    pub fn set_passphrase(&self) {
        let marker = self.home.join("passphrase-was-set");
        if marker.exists() {
            return;
        }
        let c = self.command("admin", &["admin", "init-passphrase"]);
        let (ok, text) = crate::pty::run_on_pty(
            c,
            vec![crate::pty::PASS.to_string(), crate::pty::PASS.to_string()],
        );
        if ok {
            let _ = std::fs::write(marker, "");
        } else {
            eprintln!("could not set the bench passphrase: {text}");
        }
    }

    /// Run `klatch admin <args>` the way a person would at a terminal: a pseudo-terminal where the admin passphrase
    /// is typed when asked, then `typed` for a confirmation. The output is what the terminal showed.
    pub fn admin(&self, args: &[&str], typed: &str) -> Run {
        self.set_passphrase();
        let mut full = vec!["admin"];
        full.extend_from_slice(args);
        let c = self.command("admin", &full);
        let (ok, text) =
            crate::pty::run_on_pty(c, vec![crate::pty::PASS.to_string(), typed.to_string()]);
        Run {
            ok,
            stdout: crate::pty::without_prompts(&text),
            stderr: String::new(),
        }
    }

    /// Run `klatch admin <args>` at a pseudo-terminal for a command that runs before the store opens (`recover` with
    /// the key lost): the store cannot be read, so there is no admin passphrase to ask for, and only the typed
    /// confirmation is answered.
    pub fn admin_keyless(&self, args: &[&str], typed: &str) -> Run {
        let mut full = vec!["admin"];
        full.extend_from_slice(args);
        let c = self.command("admin", &full);
        let (ok, text) = crate::pty::run_on_pty(c, vec![typed.to_string()]);
        Run {
            ok,
            stdout: crate::pty::without_prompts(&text),
            stderr: text,
        }
    }

    /// Start `klatch admin <args>` at a pseudo-terminal without waiting, for scenarios that run several at once. The
    /// passphrase and the confirmation line are typed when it asks.
    pub fn spawn_admin(&self, args: &[&str]) -> std::io::Result<crate::pty::TtyChild> {
        self.set_passphrase();
        let mut full = vec!["admin"];
        full.extend_from_slice(args);
        crate::pty::TtyChild::start(
            self.command("admin", &full),
            vec![crate::pty::PASS.to_string(), String::new()],
        )
    }

    /// Run a launcher command (`run`, `worker`, `anchor`, `judge`) at a pseudo-terminal where the admin passphrase is
    /// typed when asked (#1233). The output is what the terminal showed.
    pub fn launcher(&self, args: &[&str]) -> Run {
        self.set_passphrase();
        let (ok, text) = crate::pty::run_on_pty(
            self.command("launcher", args),
            vec![crate::pty::PASS.to_string()],
        );
        Run {
            ok,
            stdout: crate::pty::without_prompts(&text),
            stderr: String::new(),
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

    /// Run `klatch serve --stdio --ssh-key <label>` the way an `authorized_keys` forced command does (#107), with a
    /// hostile environment: `KLATCH_NAME` names `hostile`. One process handles all the request lines; the response
    /// lines come back, or the error text when the process did not start or failed.
    pub fn serve_ssh(
        &self,
        label: &str,
        hostile: &str,
        requests: &[String],
    ) -> Result<Vec<String>, String> {
        use std::io::Write;
        let mut c = self.command(hostile, &["serve", "--stdio", "--ssh-key", label]);
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
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
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

    /// Start a klatch process with a piped standard input, for scenarios that feed it by hand or kill it.
    pub fn spawn_piped(&self, who: &str, args: &[&str]) -> std::io::Result<std::process::Child> {
        self.command(who, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }

    /// Start `klatch-server channel` as `who` (scope `bot:channel`) with piped standard input and output, on this
    /// world's store (#1078). The environment is cleared, `HOME` is the world's directory.
    pub fn spawn_channel(&self, who: &str) -> Result<std::process::Child, String> {
        let bin = crate::find_server_binary()?;
        self.calls.set(self.calls.get() + 1);
        Command::new(bin)
            .arg("channel")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("KLATCH_HOME", &self.home)
            .env("KLATCH_KEY_DIR", self.home.join("keys"))
            .env("KLATCH_NAME", who)
            .env("KLATCH_SCOPE", "bot:channel")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("start klatch-server: {e}"))
    }

    /// Run `klatch judge run --once` as the launcher with the opt-in hosted judge on (`--hosted-judge sonnet
    /// --hosted-ceiling <ceiling>`), where the `claude` binary is a local script that prints `reply` (the JSON of
    /// `claude -p --output-format json`) and never reaches a model or the network (#1051). `PATH` holds only the
    /// script's directory and the system's, so no real `claude` is ever found.
    pub fn judge_with_fake_claude(&self, ceiling: &str, reply: &str) -> Run {
        use std::os::unix::fs::PermissionsExt;
        let dir = self.home.join("fakebin");
        let script = dir.join("claude");
        let body = format!("#!/bin/sh\ncat >/dev/null\ncat <<'REPLY'\n{reply}\nREPLY\n");
        let made = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&script, body))
            .and_then(|_| {
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            });
        if let Err(e) = made {
            return Run {
                ok: false,
                stdout: String::new(),
                stderr: format!("could not make the fake claude: {e}"),
            };
        }
        let mut c = self.command(
            "launcher",
            &[
                "judge",
                "run",
                "--once",
                "--hosted-judge",
                "sonnet",
                "--hosted-ceiling",
                ceiling,
            ],
        );
        c.env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .env("HOME", &self.home);
        // the judge asks for the admin passphrase at a terminal (#1233)
        self.set_passphrase();
        let (ok, text) = crate::pty::run_on_pty(c, vec![crate::pty::PASS.to_string()]);
        Run {
            ok,
            stdout: crate::pty::without_prompts(&text),
            stderr: String::new(),
        }
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
