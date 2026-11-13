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
        }
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
