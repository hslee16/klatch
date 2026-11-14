//! Shared helpers of the CLI integration tests: a throwaway KLATCH_HOME and ways to run the real binary.
#![allow(dead_code)]

use klatch_protocol::{ServerFrame, decode_server_line};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

pub static N: AtomicU32 = AtomicU32::new(0);

pub struct Home(pub PathBuf);

impl Home {
    pub fn new() -> Self {
        let d = std::env::temp_dir().join(format!(
            "klatch-cli-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        Home(d)
    }
    pub fn run(&self, who: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_klatch"))
            .env("KLATCH_HOME", &self.0)
            .env("KLATCH_NAME", who)
            .args(args)
            .output()
            .unwrap()
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
    pub fn spawn(&self, who: &str, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_klatch"))
            .env("KLATCH_HOME", &self.0)
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

impl Drop for Home {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
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
        .env("KLATCH_HOME", &home.0)
        .env_remove("KLATCH_NAME")
        .env_remove("KLATCH_DISCOVER")
        .env("HOME", fake_home)
        .env("CLAUDE_CODE_MESSAGING_TOKEN", "TOKEN-SENTINEL-d41d8cd98f")
        .args(args)
        .output()
        .unwrap()
}

pub struct Setup {
    root: PathBuf,
}

impl Setup {
    pub fn new() -> Self {
        let d = std::env::temp_dir().join(format!(
            "klatch-init-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
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
        use std::io::Write;
        let mut child = Command::new(env!("CARGO_BIN_EXE_klatch"))
            .current_dir(self.project())
            .env_clear()
            .env("HOME", self.home())
            .arg("init")
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

impl Drop for Setup {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
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
    use std::io::Write;
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.env_clear()
        .env("KLATCH_HOME", &h.0)
        .env("HOME", &h.0)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(n) = name {
        c.env("KLATCH_NAME", n);
    }
    let mut child = c.spawn().unwrap();
    let payload = format!(r#"{{"session_id":"{sid}","hook_event_name":"x"}}"#);
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
    c.env("KLATCH_HOME", &h.0)
        .env("KLATCH_NAME", "bob")
        .args(["who", "r", "--all"]);
    if let Some(n) = now {
        c.env("KLATCH_NOW", n);
    }
    String::from_utf8_lossy(&c.output().unwrap().stdout).into_owned()
}

pub fn as_scope(h: &Home, who: &str, scope: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env("KLATCH_HOME", &h.0)
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
