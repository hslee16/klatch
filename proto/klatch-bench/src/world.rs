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
}

/// What one klatch command did.
pub struct Run {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
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
            .env("KLATCH_NAME", who)
            .args(args)
            .stdin(Stdio::null());
        if let Some(t) = self.clock.get() {
            c.env("KLATCH_NOW", t.to_string());
        }
        c
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

impl Drop for World {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.home).ok();
    }
}
