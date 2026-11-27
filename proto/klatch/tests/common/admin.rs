//! Shared by the admin passphrase tests (`admin_proof*.rs`, #1233): the command tables, the ways to run a command
//! with and without a terminal, and the helpers that recover a code or repeat a command on one terminal.
#![allow(dead_code)]

use super::pty::{PtyRun, run_on_pty};
use super::{Detach, Home};
use std::process::{Command, Stdio};

pub const WRONG: &str = "wrong wrong wrong wrong";
pub const NEW: &str = "a different passphrase entirely";

/// `(variant name in kebab case, the arguments that run it)`. The command names are what `klatch admin` takes.
pub fn admin_table(h: &Home) -> Vec<(&'static str, Vec<String>)> {
    let f = |n: &str| h.0.join(n).to_string_lossy().into_owned();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    vec![
        ("pause", s(&["pause"])),
        ("resume", s(&["resume"])),
        ("close", s(&["close", "r"])),
        ("reopen", s(&["reopen", "r"])),
        ("remove", s(&["remove", "m", "--room", "r"])),
        ("allow", s(&["allow", "m", "--room", "r"])),
        ("label", s(&["label", "r", "private"])),
        ("clear", s(&["clear", "m", "internal"])),
        ("grantor", s(&["grantor", "r", "m"])),
        ("ssh-key", s(&["ssh-key", "laptop", "m"])),
        ("room-clear", s(&["room-clear", "r", "m", "internal"])),
        ("librarian", s(&["librarian", "m"])),
        ("reviewer", s(&["reviewer", "m"])),
        ("facilitator", s(&["facilitator", "m"])),
        ("integrator", s(&["integrator", "m"])),
        ("admin-agent", s(&["admin-agent", "m"])),
        ("room-creation", s(&["room-creation", "granted"])),
        ("grant-rooms", s(&["grant-rooms", "m"])),
        ("rate-limit", s(&["rate-limit", "30", "60"])),
        ("lease-cap", s(&["lease-cap", "100"])),
        ("lease-grace", s(&["lease-grace", "60"])),
        ("room-stale", s(&["room-stale", "r", "120"])),
        ("room-ask-lease", s(&["room-ask-lease", "r", "100"])),
        ("room-ask-routing", s(&["room-ask-routing", "r", "off"])),
        ("room-require", s(&["room-require", "r", "--off"])),
        ("cap", s(&["cap", "internal"])),
        ("chain", s(&["chain", "a,b"])),
        ("canary", s(&["canary", "r"])),
        ("canary-retire", s(&["canary-retire", "r"])),
        ("scrub-add", s(&["scrub-add", "n", "quokka"])),
        ("scrub-remove", s(&["scrub-remove", "n"])),
        ("scrub-list", s(&["scrub-list"])),
        ("recovery", s(&["recovery"])),
        ("recover", s(&["recover", "KLATCH-AAAA"])),
        ("seal-label", s(&["seal-label", "private"])),
        ("rotate-key", s(&["rotate-key"])),
        ("rotate-label-key", s(&["rotate-label-key", "private"])),
        ("backup", vec!["backup".into(), f("backup.db")]),
        ("restore", vec!["restore".into(), f("backup.db")]),
        (
            "run-archive",
            vec![
                "run-archive".into(),
                f("a.jsonl"),
                "--run".into(),
                "r".into(),
            ],
        ),
        ("retention", s(&["retention", "r", "30"])),
        ("archive", s(&["archive", "--all-rooms"])),
        ("restore-archive", vec!["restore-archive".into(), f("x.db")]),
        ("archives", s(&["archives"])),
        ("export", s(&["export", "r"])),
        (
            "stats",
            s(&["stats", "--room", "lobby", "--since", "2026-01-01"]),
        ),
        ("init-passphrase", s(&["init-passphrase"])),
        ("reset-passphrase", s(&["reset-passphrase"])),
        ("unlock-window", s(&["unlock-window", "60"])),
        ("shell", s(&["shell"])),
        ("refusals", s(&["refusals"])),
        ("review-queue", s(&["review-queue"])),
        ("audit", s(&["audit"])),
    ]
}

/// The launcher commands (#834) as `(name, arguments)`.
pub fn launcher_table(h: &Home) -> Vec<(&'static str, Vec<String>)> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let out = h.0.join("w.bundle").to_string_lossy().into_owned();
    vec![
        (
            "worker issue",
            vec![
                "worker".into(),
                "issue".into(),
                "w-1".into(),
                "--clearance".into(),
                "internal".into(),
                "--out".into(),
                out,
            ],
        ),
        ("worker revoke", s(&["worker", "revoke", "w-1"])),
        ("worker list", s(&["worker", "list"])),
        ("anchor done", s(&["anchor", "done", "a"])),
        (
            "anchor reap",
            s(&["anchor", "reap", "--dir", "/nonexistent-klatch-dir"]),
        ),
        (
            "anchor github",
            s(&["anchor", "github", "--file", "/dev/null"]),
        ),
        ("run create", s(&["run", "create", "r1"])),
        ("run end", s(&["run", "end", "r1"])),
        ("run list", s(&["run", "list"])),
        ("judge run", s(&["judge", "run", "--once"])),
    ]
}

/// The variants of `AdminCmd` as the command line names them, read from the source (the audit test does the same).
pub fn admin_variants() -> Vec<String> {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/admin/mod.rs"),
    )
    .unwrap();
    let body = src
        .split("pub enum AdminCmd {")
        .nth(1)
        .unwrap()
        .split("\n}\n")
        .next()
        .unwrap();
    let mut out = Vec::new();
    for line in body.lines() {
        // a variant starts at exactly four spaces of indent with a capital letter
        let Some(rest) = line.strip_prefix("    ") else {
            continue;
        };
        if !rest.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let mut kebab = String::new();
        for (i, c) in name.chars().enumerate() {
            if c.is_ascii_uppercase() && i > 0 {
                kebab.push('-');
            }
            kebab.push(c.to_ascii_lowercase());
        }
        out.push(kebab);
    }
    out
}

pub fn klatch_in(h: &Home, args: &[String]) -> Command {
    let mut full = vec!["admin".to_string()];
    full.extend(args.iter().cloned());
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    h.launcher_command(&refs)
}

/// `klatch admin ARGS` from string slices.
pub fn admin_cmd(h: &Home, args: &[&str]) -> Command {
    let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    klatch_in(h, &owned)
}

pub fn klatch_raw(h: &Home, args: &[String]) -> Command {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    h.launcher_command(&refs)
}

/// Owned arguments from string slices.
pub fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// Run detached: its own session (so no controlling terminal), no terminal on any descriptor.
pub fn detached(mut c: Command, stdin_bytes: Option<&str>) -> (bool, String) {
    use std::io::Write;
    c.detached();
    c.stdin(if stdin_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    if let (Some(text), Some(mut stdin)) = (stdin_bytes, child.stdin.take()) {
        let _ = writeln!(stdin, "{text}");
        let _ = writeln!(stdin, "{text}");
    }
    let o = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    (o.status.success(), text)
}

/// The recovery code of a home, shown with the passphrase and the typed confirmation.
pub fn recovery_code_of(h: &Home) -> String {
    let shown = h.admin(&["recovery"], &["recovery"]);
    assert!(shown.ok(), "{}", shown.text);
    shown
        .text
        .lines()
        .find(|l| l.trim_start().starts_with("KLATCH-"))
        .unwrap_or_else(|| panic!("no recovery code in:\n{}", shown.text))
        .trim()
        .to_string()
}

/// Run `cmd` `times` times in a row inside ONE shell on ONE pty, so every run shares the terminal (a new pty is a
/// new terminal and has its own failure count, #1274).
pub fn repeated_on_one_pty(cmd: Command, times: usize, typed: &[&str]) -> PtyRun {
    let mut sh = Command::new("sh");
    sh.arg("-c")
        .arg(format!("for i in $(seq {times}); do \"$0\" \"$@\"; done"))
        .arg(cmd.get_program())
        .args(cmd.get_args());
    for (k, v) in cmd.get_envs() {
        match v {
            Some(v) => sh.env(k, v),
            None => sh.env_remove(k),
        };
    }
    if let Some(d) = cmd.get_current_dir() {
        sh.current_dir(d);
    }
    run_on_pty(sh, typed)
}

/// What a signal from outside does to the terminal during the passphrase prompt (#1278): the terminal must be left
/// with echo and signal keys on, and the process must still end on that signal.
pub fn terminal_after_signal(sig: i32) {
    let h = Home::new();
    h.set_passphrase();
    let args = admin_table(&h).into_iter().next().unwrap().1;
    let (ended_on_signal, lflag) = super::pty::run_on_pty_signalled(klatch_in(&h, &args), sig);
    assert!(ended_on_signal, "the process did not end on signal {sig}");
    assert!(
        lflag & libc::ECHO != 0 && lflag & libc::ISIG != 0,
        "signal {sig} left the terminal with echo or signal keys off (lflag {lflag:#x})"
    );
}

/// Every file under `dir`, at any depth.
pub fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}
