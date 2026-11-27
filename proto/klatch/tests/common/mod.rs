//! Shared helpers of the CLI integration tests: a throwaway KLATCH_HOME and ways to run the real binary.
#![allow(dead_code)]

use klatch::protocol::{ServerFrame, decode_server_line};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::AtomicU32;

pub use pty::PtyRun;

#[path = "../../src/store/test_dir.rs"]
pub mod test_dir;
use test_dir::TestDir;

pub mod admin;
pub mod golden;
pub mod pty;

/// The terminal text without the passphrase prompt lines (they are the terminal's, not the command's output).
pub fn without_prompts(text: &str) -> String {
    text.split_inclusive('\n')
        .filter(|l| !l.starts_with("admin passphrase to approve"))
        .collect()
}

/// Assert that `text` contains every needle; the failure names the missing one and shows the whole text.
#[track_caller]
pub fn assert_has(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(text.contains(needle), "missing {needle:?} in: {text}");
    }
}

/// Assert that `text` contains none of the needles; the failure names the one found and shows the whole text.
#[track_caller]
pub fn assert_lacks(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(!text.contains(needle), "unexpected {needle:?} in: {text}");
    }
}

/// Run in a session of its own, so the child has no controlling terminal. A person who runs `cargo test` at a
/// terminal would otherwise be asked for the admin passphrase by a test that runs an admin or launcher command
/// without a terminal of its own (#1233): such a test means to be refused, and `/dev/tty` would let it ask.
pub trait Detach {
    fn detached(&mut self) -> &mut Self;
}

impl Detach for Command {
    fn detached(&mut self) -> &mut Self {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches nothing of the parent.
        unsafe {
            self.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        self
    }
}

pub static N: AtomicU32 = AtomicU32::new(0);

/// The admin passphrase the tests set (#1233). It is typed at a pseudo-terminal, the only way the product reads one.
pub const PASS: &str = "correct horse battery staple";

/// A throwaway KLATCH_HOME (removed when dropped, also when the test panics; #995). `.0` is the path, `.1` holds it.
pub struct Home(pub PathBuf, TestDir);

impl Home {
    pub fn new() -> Self {
        let dir = TestDir::new("cli");
        Home(dir.to_path_buf(), dir)
    }
    pub fn run(&self, who: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_klatch"))
            .detached()
            .env_remove("KLATCH_IDENTITY_FILE")
            .env("KLATCH_HOME", &self.0)
            .env("KLATCH_KEY_DIR", self.0.join("keys"))
            .env("KLATCH_NAME", who)
            .args(args)
            .output()
            .unwrap()
    }
    /// `run` from the directory `cwd` with `home` as HOME (what klatch reads to find hooks, #1416).
    pub fn run_at(
        &self,
        who: &str,
        args: &[&str],
        cwd: &std::path::Path,
        home: &std::path::Path,
    ) -> Output {
        let mut c = self.command_at(who, args, cwd, home);
        c.output().unwrap()
    }
    pub fn command_at(
        &self,
        who: &str,
        args: &[&str],
        cwd: &std::path::Path,
        home: &std::path::Path,
    ) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
        c.detached()
            .current_dir(cwd)
            .env_remove("KLATCH_IDENTITY_FILE")
            .env("HOME", home)
            .env("KLATCH_HOME", &self.0)
            .env("KLATCH_KEY_DIR", self.0.join("keys"))
            .env("KLATCH_NAME", who)
            .args(args);
        c
    }
    /// Run with `stdin` as standard input, as Claude Code runs a hook.
    pub fn run_stdin(&self, who: &str, args: &[&str], stdin: &str) -> Output {
        let mut child = self.spawn(who, args);
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    /// The command for a launcher: no name, and a home with no harness registry, so no coding-agent session is found
    /// above the process (the tests themselves may run inside one).
    pub fn launcher_command(&self, args: &[&str]) -> Command {
        let fake = self.0.join("fake-home");
        std::fs::create_dir_all(&fake).unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
        c.env_remove("KLATCH_NAME")
            .env_remove("KLATCH_IDENTITY_FILE")
            .env("HOME", &fake)
            .env("KLATCH_HOME", &self.0)
            .env("KLATCH_KEY_DIR", self.0.join("keys"))
            .args(args);
        c
    }
    /// Set the admin passphrase of this home once (typed twice at a pseudo-terminal), as a person does after
    /// installing. Idempotent.
    pub fn set_passphrase(&self) {
        let marker = self.0.join("passphrase-was-set");
        if marker.exists() {
            return;
        }
        let r = pty::run_on_pty(
            self.launcher_command(&["admin", "init-passphrase"]),
            &[PASS, PASS],
        );
        assert!(r.ok(), "init-passphrase failed:\n{}", r.text);
        std::fs::write(marker, "").unwrap();
    }
    /// Run as a launcher would, at a terminal where the person types the admin passphrase when asked (and `typed`
    /// after that, for a confirmation). The output is what the terminal showed.
    pub fn launcher_typing(&self, args: &[&str], typed: &[&str]) -> PtyRun {
        self.launcher_typing_env(args, typed, &[])
    }
    /// `launcher_typing` with extra environment (a test-clock build reads `KLATCH_NOW_FILE`).
    pub fn launcher_typing_env(
        &self,
        args: &[&str],
        typed: &[&str],
        env: &[(&str, &std::path::Path)],
    ) -> PtyRun {
        self.set_passphrase();
        let mut lines = vec![PASS];
        lines.extend_from_slice(typed);
        let mut command = self.launcher_command(args);
        for (k, v) in env {
            command.env(k, v);
        }
        let mut run = pty::run_on_pty(command, &lines);
        // the prompt is the terminal's, not the command's output
        run.text = without_prompts(&run.text);
        run
    }
    /// `launcher_typing` with nothing more typed, as a plain `Output`.
    pub fn launcher(&self, args: &[&str]) -> Output {
        self.launcher_typing(args, &[]).into_output()
    }
    /// `klatch admin ...` at a terminal, typing the passphrase and then `typed` for a confirmation.
    pub fn admin(&self, args: &[&str], typed: &[&str]) -> PtyRun {
        let mut full = vec!["admin"];
        full.extend_from_slice(args);
        self.launcher_typing(&full, typed)
    }
    /// Run as a worker holding a key bundle: only the path is given, and no key directory or name.
    pub fn worker(&self, bundle: &std::path::Path, args: &[&str]) -> Output {
        self.worker_with_env(bundle, args, &[])
    }
    /// `worker` with extra environment (a test-clock build reads `KLATCH_NOW_FILE`, so a test can move time without
    /// sleeping).
    pub fn worker_with_env(
        &self,
        bundle: &std::path::Path,
        args: &[&str],
        env: &[(&str, &std::path::Path)],
    ) -> Output {
        let fake = self.0.join("fake-home");
        std::fs::create_dir_all(&fake).unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
        c.detached()
            .env_remove("KLATCH_NAME")
            .env_remove("KLATCH_KEY_DIR")
            .env("HOME", &fake)
            .env("KLATCH_HOME", &self.0)
            .env("KLATCH_IDENTITY_FILE", bundle)
            .args(args);
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().unwrap()
    }
    pub fn spawn_at(
        &self,
        who: &str,
        args: &[&str],
        cwd: &std::path::Path,
        home: &std::path::Path,
    ) -> Child {
        self.command_at(who, args, cwd, home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
    pub fn spawn(&self, who: &str, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_klatch"))
            .detached()
            .env_remove("KLATCH_IDENTITY_FILE")
            .env("KLATCH_HOME", &self.0)
            .env("KLATCH_KEY_DIR", self.0.join("keys"))
            .env("KLATCH_NAME", who)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
    pub fn ok(&self, who: &str, args: &[&str]) -> String {
        let o = self.run(who, args);
        assert!(
            o.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }
    pub fn fail(&self, who: &str, args: &[&str]) -> String {
        let o = self.run(who, args);
        assert!(!o.status.success(), "{args:?} unexpectedly succeeded");
        String::from_utf8_lossy(&o.stderr).into_owned()
    }
}

impl Home {
    /// Run `klatch serve --stdio` with the given input lines and decode every output line.
    pub fn serve(&self, who: &str, lines: &[&str]) -> Vec<ServerFrame> {
        let mut child = self.spawn(who, &["serve", "--stdio"]);
        {
            let stdin = child.stdin.as_mut().unwrap();
            for l in lines {
                writeln!(stdin, "{l}").unwrap();
            }
        }
        drop(child.stdin.take());
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|l| {
                decode_server_line(l)
                    .unwrap_or_else(|e| panic!("stdout is not a frame: {l}: {e:?}"))
            })
            .collect()
    }
}

pub fn request(id: &str, method: &str, params: &str) -> String {
    format!(r#"{{"v":1,"type":"request","id":"{id}","method":"{method}","params":{params}}}"#)
}

impl Home {
    /// Run `klatch stream ...` to completion and decode every output line.
    pub fn stream(&self, who: &str, args: &[&str]) -> Vec<ServerFrame> {
        let mut full = vec!["stream"];
        full.extend_from_slice(args);
        let out = self.ok_output(who, &full);
        out.lines()
            .map(|l| {
                decode_server_line(l)
                    .unwrap_or_else(|e| panic!("stdout is not a frame: {l}: {e:?}"))
            })
            .collect()
    }

    pub fn ok_output(&self, who: &str, args: &[&str]) -> String {
        self.ok(who, args)
    }
}

pub fn seqs_of(f: &[ServerFrame]) -> Vec<u64> {
    f.iter()
        .filter_map(|x| {
            if let ServerFrame::Message { seq, .. } = x {
                Some(*seq)
            } else {
                None
            }
        })
        .collect()
}

// Discovery (opt-in): the caller found from the process tree and the harness registry.

pub fn without_a_name(home: &Home, fake_home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env("KLATCH_HOME", &home.0)
        .env("KLATCH_KEY_DIR", home.0.join("keys"))
        .env_remove("KLATCH_NAME")
        .env_remove("KLATCH_DISCOVER")
        .env("HOME", fake_home)
        .env("CLAUDE_CODE_MESSAGING_TOKEN", "TOKEN-SENTINEL-d41d8cd98f")
        .args(args)
        .output()
        .unwrap()
}

pub struct Setup {
    root: TestDir,
}

impl Setup {
    pub fn new() -> Self {
        let d = TestDir::new("init");
        std::fs::create_dir_all(d.join("project")).unwrap();
        std::fs::create_dir_all(d.join("home")).unwrap();
        Setup { root: d }
    }
    pub fn project(&self) -> PathBuf {
        self.root.join("project")
    }
    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    pub fn project_settings(&self) -> PathBuf {
        self.project().join(".claude/settings.local.json")
    }
    pub fn user_settings(&self) -> PathBuf {
        self.home().join(".claude/settings.json")
    }
    pub fn init(&self, args: &[&str], stdin: &str) -> std::process::Output {
        self.klatch("init", args, stdin)
    }
    /// Run one klatch subcommand in the throwaway project with the throwaway home and an empty environment.
    pub fn klatch(&self, sub: &str, args: &[&str], stdin: &str) -> std::process::Output {
        use std::io::Write;
        let mut child = Command::new(env!("CARGO_BIN_EXE_klatch"))
            .env_remove("KLATCH_IDENTITY_FILE")
            .current_dir(self.project())
            .env_clear()
            .env("HOME", self.home())
            .arg(sub)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    pub fn read(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }
}

pub fn out(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

pub fn hook_cmds(v: &serde_json::Value, event: &str) -> Vec<String> {
    v["hooks"][event]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["hooks"].as_array().unwrap().clone())
        .map(|h| h["command"].as_str().unwrap().to_string())
        .collect()
}

pub fn notice_of(out: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(out.trim()).ok()?;
    v["systemMessage"].as_str().map(String::from)
}

pub fn hook_as_session(h: &Home, name: Option<&str>, sid: &str, args: &[&str]) -> Output {
    let payload = format!(r#"{{"session_id":"{sid}","hook_event_name":"x"}}"#);
    hook_with_payload(h, name, &payload, args)
}

/// Run the hook with an exact JSON payload on standard input, as Claude Code does.
pub fn hook_with_payload(h: &Home, name: Option<&str>, payload: &str, args: &[&str]) -> Output {
    use std::io::Write;
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.env_remove("KLATCH_IDENTITY_FILE");
    c.env_clear()
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("HOME", &h.0)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(n) = name {
        c.env("KLATCH_NAME", n);
    }
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

pub fn who_age_with_now(h: &Home, now: Option<&str>) -> String {
    h.ok("alice", &["join", "r"]);
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.env_remove("KLATCH_IDENTITY_FILE");
    c.env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", "bob")
        .args(["who", "r", "--all"]);
    if let Some(n) = now {
        c.env("KLATCH_NOW", n);
    }
    String::from_utf8_lossy(&c.output().unwrap().stdout).into_owned()
}

pub fn as_scope(h: &Home, who: &str, scope: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", who)
        .args(["--member-scope", scope])
        .args(args)
        .output()
        .unwrap()
}

pub fn read_only(home: &Home, on: bool) {
    use std::os::unix::fs::PermissionsExt;
    let (dir, file) = if on { (0o555, 0o444) } else { (0o755, 0o644) };
    for e in std::fs::read_dir(&home.0).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(file)).unwrap();
        }
    }
    std::fs::set_permissions(&home.0, std::fs::Permissions::from_mode(dir)).unwrap();
}
