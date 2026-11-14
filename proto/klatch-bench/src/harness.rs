//! A fake harness: stand-ins for Claude Code sessions with a real process tree and real registry records, so the
//! parts of klatch that depend on process ancestry and the session registry can be tested without a model.
//!
//! A fake session is a shell started under the name `claude` (the liveness check requires that program name),
//! running a small loop that executes the scripts the test sends it. Its registry record is written to a
//! temporary `HOME/.claude/sessions/<pid>.json`, in the format Claude Code uses. A child session is started by a
//! parent's shell, so the two form a real parent and child in the process table.

use crate::World;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// The directory layout and the shared settings of one fake harness.
pub struct FakeHarness {
    root: PathBuf,
    home: PathBuf,
    fake_claude: PathBuf,
    klatch: PathBuf,
    klatch_home: PathBuf,
    sessions: std::cell::RefCell<Vec<u32>>,
    roots: std::cell::RefCell<Vec<Child>>,
}

/// One running fake session.
pub struct Session<'a> {
    harness: &'a FakeHarness,
    pub id: u64,
    pub pid: u32,
    pub session_id: String,
    pub name: String,
    cwd: PathBuf,
    seq: std::cell::Cell<u64>,
}

fn domain() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

/// `ps` output for one field of a pid, with the time zone fixed to UTC and whitespace collapsed (the form the
/// registry records).
fn ps(pid: u32, field: &str) -> Option<String> {
    let out = Command::new("ps")
        .env("TZ", "UTC")
        .args(["-o", field, "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn wait_for(path: &Path, what: &str) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!("timed out waiting for {what} ({})", path.display()))
}

/// Who a fake session is: the id and name its registry record carries and the directory it started in.
#[derive(Clone, Copy)]
struct Identity<'a> {
    session_id: &'a str,
    name: &'a str,
    cwd: &'a Path,
}

impl FakeHarness {
    pub fn new(world: &World) -> Result<FakeHarness, String> {
        let root = world
            .home
            .join(format!("harness-{}", NEXT.fetch_add(1, Ordering::SeqCst)));
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".claude").join("sessions"))
            .map_err(|e| e.to_string())?;
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
        let fake_claude = bin.join("claude");
        // A symlink, not a copy: macOS kills a copied system binary, and the program name `ps` reports follows the
        // name it was started under (so the liveness check sees `claude`).
        std::os::unix::fs::symlink("/bin/sh", &fake_claude)
            .map_err(|e| format!("link /bin/sh as claude: {e}"))?;
        std::fs::create_dir_all(root.join("run")).map_err(|e| e.to_string())?;
        Ok(FakeHarness {
            root,
            home,
            fake_claude,
            klatch: world.bin.clone(),
            klatch_home: world.home.clone(),
            sessions: Default::default(),
            roots: Default::default(),
        })
    }

    fn registry(&self) -> PathBuf {
        self.home.join(".claude").join("sessions")
    }

    /// The shell loop a fake session runs: it records its pid, then runs each script file the test drops in its
    /// inbox, in name order, until told to stop.
    fn loop_script(&self, id: u64) -> String {
        let run = self.root.join("run");
        format!(
            r#"echo $$ > '{run}/pid-{id}'
hook() {{ printf '{{"session_id":"%s"}}' "$2" | "$KLATCH" --discover hook --event "$1"; }}
while [ ! -f '{run}/stop-{id}' ]; do
  for f in $(ls '{run}'/inbox-{id}-*.sh 2>/dev/null | sort); do
    . "$f"
    mv "$f" "$f.done"
  done
  sleep 0.02
done"#,
            run = run.display()
        )
    }

    fn env(&self, c: &mut Command) {
        c.env_clear()
            .env("HOME", &self.home)
            .env("KLATCH_HOME", &self.klatch_home)
            .env("KLATCH", &self.klatch)
            .env("FAKE_CLAUDE", &self.fake_claude)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }

    fn register(&self, pid: u32, who: &Identity) -> Result<(), String> {
        let Identity {
            session_id,
            name,
            cwd,
        } = *who;
        let start =
            ps(pid, "lstart=").ok_or("could not read the start time of the fake session")?;
        let record = serde_json::json!({
            "pid": pid, "sessionId": session_id, "cwd": cwd.to_string_lossy(),
            "name": name, "procStart": start, "pidDomain": domain(),
        });
        std::fs::write(
            self.registry().join(format!("{pid}.json")),
            record.to_string(),
        )
        .map_err(|e| e.to_string())
    }

    /// Start a top-level session (like an interactive Claude Code), with its registry record written.
    pub fn start(&self, session_id: &str, name: &str) -> Result<Session<'_>, String> {
        self.start_in(session_id, name, &self.root.clone())
    }

    /// `start`, with a working directory (the directory the registry record names, which klatch derives a
    /// coding session's scope from).
    pub fn start_in(
        &self,
        session_id: &str,
        name: &str,
        cwd: &Path,
    ) -> Result<Session<'_>, String> {
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let mut c = Command::new(&self.fake_claude);
        c.arg("-c").arg(self.loop_script(id)).current_dir(cwd);
        self.env(&mut c);
        let child = c
            .spawn()
            .map_err(|e| format!("start a fake session: {e}"))?;
        self.roots.borrow_mut().push(child);
        self.attach(
            id,
            &Identity {
                session_id,
                name,
                cwd,
            },
        )
    }

    fn attach(&self, id: u64, who: &Identity) -> Result<Session<'_>, String> {
        let Identity {
            session_id,
            name,
            cwd,
        } = *who;
        let pidfile = self.root.join("run").join(format!("pid-{id}"));
        wait_for(&pidfile, "the fake session to start")?;
        let pid: u32 = std::fs::read_to_string(&pidfile)
            .map_err(|e| e.to_string())?
            .trim()
            .parse()
            .map_err(|_| "bad pid file")?;
        self.register(pid, who)?;
        self.sessions.borrow_mut().push(pid);
        Ok(Session {
            harness: self,
            id,
            pid,
            session_id: session_id.into(),
            name: name.into(),
            cwd: cwd.to_path_buf(),
            seq: std::cell::Cell::new(0),
        })
    }
}

impl Session<'_> {
    /// Run a shell script inside this session's shell and wait until it has run. The script may use `hook EVENT
    /// SESSION_ID` and `$KLATCH`.
    pub fn run(&self, script: &str) -> Result<(), String> {
        let n = self.seq.get();
        self.seq.set(n + 1);
        let run = self.harness.root.join("run");
        let file = run.join(format!("inbox-{}-{n:04}.sh", self.id));
        let tmp = run.join(format!(".tmp-{}-{n}", self.id));
        std::fs::write(&tmp, script).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
        let done = PathBuf::from(format!("{}.done", file.display()));
        wait_for(&done, "the fake session to run a script")
    }

    /// Run one hook event as this session, with its own session id on standard input, and return what it printed.
    pub fn hook(&self, event: &str) -> Result<String, String> {
        self.hook_with_id(event, &self.session_id)
    }

    /// Run a hook event with a given session id on standard input (the id the harness says the hook is for).
    pub fn hook_with_id(&self, event: &str, session_id: &str) -> Result<String, String> {
        let out = self
            .harness
            .root
            .join("run")
            .join(format!("out-{}-{}", self.id, self.seq.get()));
        self.run(&format!(
            "hook {event} {session_id} > '{}' 2>&1",
            out.display()
        ))?;
        std::fs::read_to_string(&out).map_err(|e| e.to_string())
    }

    /// Run a hook event with no session id on standard input at all.
    pub fn hook_without_id(&self, event: &str) -> Result<String, String> {
        let out = self
            .harness
            .root
            .join("run")
            .join(format!("out-{}-{}", self.id, self.seq.get()));
        self.run(&format!(
            "\"$KLATCH\" --discover hook --event {event} < /dev/null > '{}' 2>&1",
            out.display()
        ))?;
        std::fs::read_to_string(&out).map_err(|e| e.to_string())
    }

    /// Run `klatch --discover <args>` inside this session's shell, as the session itself would (its own process
    /// tree, its own registry record). Returns whether it succeeded and what it printed (output and errors).
    pub fn klatch(&self, args: &[&str]) -> Result<(bool, String), String> {
        let n = self.seq.get();
        let out = self
            .harness
            .root
            .join("run")
            .join(format!("kout-{}-{n}", self.id));
        let code = self
            .harness
            .root
            .join("run")
            .join(format!("kcode-{}-{n}", self.id));
        let quoted: Vec<String> = args
            .iter()
            .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
            .collect();
        self.run(&format!(
            "\"$KLATCH\" --discover {} < /dev/null > '{}' 2>&1; echo $? > '{}'",
            quoted.join(" "),
            out.display(),
            code.display()
        ))?;
        let ok = std::fs::read_to_string(&code)
            .map_err(|e| e.to_string())?
            .trim()
            == "0";
        Ok((
            ok,
            std::fs::read_to_string(&out).map_err(|e| e.to_string())?,
        ))
    }

    /// Start a child session from this session's shell (like a headless run started from an interactive one).
    pub fn spawn_child(&self, session_id: &str, name: &str) -> Result<Session<'_>, String> {
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let script = self.harness.loop_script(id).replace('\'', "'\\''");
        self.run(&format!("\"$FAKE_CLAUDE\" -c '{script}' &"))?;
        let who = Identity {
            session_id,
            name,
            cwd: &self.cwd,
        };
        self.harness.attach(id, &who)
    }

    /// The harness removing this session's registry record (what happens as a real session exits).
    pub fn drop_record(&self) {
        std::fs::remove_file(self.harness.registry().join(format!("{}.json", self.pid))).ok();
    }

    /// The harness giving this session a new name in its record.
    pub fn rename(&self, name: &str) -> Result<(), String> {
        let who = Identity {
            session_id: &self.session_id,
            name,
            cwd: &self.cwd,
        };
        self.harness.register(self.pid, &who)
    }
}

impl Drop for FakeHarness {
    fn drop(&mut self) {
        for pid in self.sessions.borrow().iter() {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
        }
        for c in self.roots.borrow_mut().iter_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        std::fs::remove_dir_all(&self.root).ok();
    }
}
