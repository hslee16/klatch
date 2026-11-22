//! The admin passphrase, end to end (#1233): the real binary, on real pseudo-terminals and on none.
//!
//! The threat is a program of the same user (a coding agent) that drives the `klatch` command line and so can run any
//! command a person can, with or without a terminal of its own. It does not know the passphrase. Every command
//! that administers, launches or touches the key must refuse it, whichever way it comes: on a pseudo-terminal it
//! opened, with no terminal at all (detached, `setsid`), with the passphrase in the environment, an argument or a
//! pipe. The adversarial tests below run EVERY `AdminCmd` variant (a variant with no row in the table fails the
//! test that counts them), every launcher command and every keystore command.
//!
//! Not covered, on purpose and said in SECURITY.md: a process that reads the keyboard, the memory of an unlocked
//! process, or the store and its key directly.
mod common;

use common::pty::run_on_pty;
use common::*;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

const WRONG: &str = "wrong wrong wrong wrong";
const NEW: &str = "a different passphrase entirely";

/// `(variant name in kebab case, the arguments that run it)`. The command names are what `klatch admin` takes.
fn admin_table(h: &Home) -> Vec<(&'static str, Vec<String>)> {
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
        ("run-archive", vec!["run-archive".into(), f("a.jsonl"), "--run".into(), "r".into()]),
        ("retention", s(&["retention", "r", "30"])),
        ("archive", s(&["archive", "--all-rooms"])),
        ("restore-archive", vec!["restore-archive".into(), f("x.db")]),
        ("archives", s(&["archives"])),
        ("export", s(&["export", "r"])),
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
fn launcher_table(h: &Home) -> Vec<(&'static str, Vec<String>)> {
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
        ("anchor reap", s(&["anchor", "reap", "--dir", "/nonexistent-klatch-dir"])),
        ("anchor github", s(&["anchor", "github", "--file", "/dev/null"])),
        ("run create", s(&["run", "create", "r1"])),
        ("run end", s(&["run", "end", "r1"])),
        ("run list", s(&["run", "list"])),
        ("judge run", s(&["judge", "run", "--once"])),
    ]
}

/// The variants of `AdminCmd` as the command line names them, read from the source (the audit test does the same).
fn admin_variants() -> Vec<String> {
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

fn klatch_in(h: &Home, args: &[String]) -> Command {
    let mut full = vec!["admin".to_string()];
    full.extend(args.iter().cloned());
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    h.launcher_command(&refs)
}

fn klatch_raw(h: &Home, args: &[String]) -> Command {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    h.launcher_command(&refs)
}

/// Run detached: its own session (so no controlling terminal), no terminal on any descriptor.
fn detached(mut c: Command, stdin_bytes: Option<&str>) -> (bool, String) {
    use std::io::Write;
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    c.stdin(if stdin_bytes.is_some() { Stdio::piped() } else { Stdio::null() })
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

#[test]
fn the_table_has_a_row_for_every_admin_variant() {
    let h = Home::new();
    let table: Vec<&str> = admin_table(&h).iter().map(|(n, _)| *n).collect();
    let mut variants = admin_variants();
    variants.sort();
    let mut have: Vec<&str> = table.clone();
    have.sort();
    assert_eq!(
        variants,
        have,
        "a new AdminCmd needs a row in admin_table (and so a refusal test)"
    );
}

#[test]
fn an_agent_with_a_pseudo_terminal_and_a_wrong_passphrase_is_refused_for_every_admin_command() {
    for (name, _) in admin_table(&Home::new()) {
        let h = Home::new();
        h.set_passphrase();
        let args = admin_table(&h)
            .into_iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1;
        // typed: the wrong passphrase, then what a confirmation or a second prompt might ask for
        let r = run_on_pty(klatch_in(&h, &args), &[WRONG, WRONG, WRONG]);
        assert!(!r.ok(), "{name} was NOT refused:\n{}", r.text);
        assert!(
            r.text.contains("wrong passphrase")
                || r.text.contains("recovery code")
                || r.text.contains("passphrase"),
            "{name}: expected a passphrase refusal, got:\n{}",
            r.text
        );
        assert!(
            !r.text.contains("KLATCH-"),
            "{name} printed a recovery code:\n{}",
            r.text
        );
    }
}

#[test]
fn a_detached_process_with_no_terminal_is_refused_for_every_admin_command() {
    for (name, _) in admin_table(&Home::new()) {
        let h = Home::new();
        h.set_passphrase();
        let args = admin_table(&h)
            .into_iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1;
        let (ok, text) = detached(klatch_in(&h, &args), None);
        assert!(!ok, "{name} ran with no terminal:\n{text}");
        assert!(
            text.contains("real terminal") || text.contains("no admin passphrase"),
            "{name}: {text}"
        );
    }
}

#[test]
fn the_passphrase_in_the_environment_an_argument_or_a_pipe_is_never_read() {
    let h = Home::new();
    h.set_passphrase();
    for (name, args) in admin_table(&h) {
        // only the commands that would otherwise do something
        if ["shell", "init-passphrase", "reset-passphrase"].contains(&name) {
            continue;
        }
        let mut c = klatch_in(&h, &args);
        for var in [
            "KLATCH_ADMIN_PASSPHRASE",
            "KLATCH_PASSPHRASE",
            "KLATCH_ADMIN_PASS",
            "ADMIN_PASSPHRASE",
            "PASSPHRASE",
        ] {
            c.env(var, PASS);
        }
        let (ok, text) = detached(c, Some(PASS));
        assert!(!ok, "{name} accepted a passphrase from the environment or a pipe:\n{text}");
    }
    // an argument that looks like one is just an unknown argument
    for flag in ["--passphrase", "--admin-passphrase", "--pass"] {
        let args: Vec<String> = vec!["admin".into(), "pause".into(), flag.into(), PASS.into()];
        let (ok, text) = detached(klatch_raw(&h, &args), Some(PASS));
        assert!(!ok, "{flag}: {text}");
    }
}

#[test]
fn while_no_passphrase_is_set_every_admin_command_says_so_and_does_nothing() {
    let h = Home::new();
    // an existing install: a store with a room in it, and no passphrase
    assert!(h.run("alice", &["join", "r"]).status.success());
    for (name, args) in admin_table(&h) {
        if ["init-passphrase", "reset-passphrase"].contains(&name) {
            continue;
        }
        let r = run_on_pty(klatch_in(&h, &args), &[PASS, PASS]);
        assert!(!r.ok(), "{name}:\n{}", r.text);
        assert!(r.text.contains("init-passphrase"), "{name}: {}", r.text);
    }
}

#[test]
fn an_agent_with_a_pseudo_terminal_and_a_wrong_passphrase_is_refused_for_every_launcher_command() {
    for (name, _) in launcher_table(&Home::new()) {
        let h = Home::new();
        h.set_passphrase();
        let args = launcher_table(&h)
            .into_iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1;
        let r = run_on_pty(klatch_raw(&h, &args), &[WRONG, WRONG]);
        assert!(!r.ok(), "{name} was NOT refused:\n{}", r.text);
        assert!(r.text.contains("wrong passphrase"), "{name}: {}", r.text);
        assert!(
            !h.0.join("w.bundle").exists(),
            "{name} left a credential behind"
        );
    }
}

#[test]
fn a_detached_process_cannot_issue_a_credential_or_run_the_launcher_commands() {
    // the #834 reproduction: a process that detaches (setsid, no terminal) escaped the old process-tree check
    for (name, _) in launcher_table(&Home::new()) {
        let h = Home::new();
        h.set_passphrase();
        let args = launcher_table(&h)
            .into_iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1;
        let (ok, text) = detached(klatch_raw(&h, &args), None);
        assert!(!ok, "{name} ran detached:\n{text}");
        assert!(text.contains("real terminal"), "{name}: {text}");
        assert!(!h.0.join("w.bundle").exists(), "{name} issued a credential");
        // the same with the passphrase piped in and a bundle in the environment, as a worker would hold
        let (ok, text) = detached(klatch_raw(&h, &args), Some(PASS));
        assert!(!ok, "{name} ran detached with a pipe:\n{text}");
    }
}

#[test]
fn the_right_passphrase_does_the_act_and_the_audit_records_it_without_the_passphrase() {
    let h = Home::new();
    let r = h.admin(&["lease-cap", "100"], &[]);
    assert!(r.ok(), "{}", r.text);
    let audit = h.admin(&["audit"], &[]);
    assert!(audit.ok(), "{}", audit.text);
    assert!(audit.text.contains("lease-cap"), "{}", audit.text);
    assert!(audit.text.contains("proof-ok"), "{}", audit.text);
    assert!(audit.text.contains("passphrase-set"), "{}", audit.text);
    assert!(!audit.text.contains(PASS), "the passphrase reached the audit");
    // a wrong attempt is a row with a count and nothing else
    let bad = run_on_pty(klatch_in(&h, &["resume".to_string()]), &[WRONG]);
    assert!(!bad.ok());
    let audit = h.admin(&["audit"], &[]);
    assert!(audit.text.contains("proof-failed"), "{}", audit.text);
    assert!(audit.text.contains("failures=1"), "{}", audit.text);
    assert!(!audit.text.contains(WRONG), "a wrong attempt was kept");
    assert!(!audit.text.contains(PASS));
}

#[test]
fn three_wrong_passphrases_lock_even_the_right_one_out_for_a_while() {
    let h = Home::new();
    h.set_passphrase();
    for _ in 0..3 {
        let r = run_on_pty(klatch_in(&h, &["pause".to_string()]), &[WRONG]);
        assert!(r.text.contains("wrong passphrase"), "{}", r.text);
    }
    let r = run_on_pty(klatch_in(&h, &["pause".to_string()]), &[PASS]);
    assert!(!r.ok(), "{}", r.text);
    assert!(r.text.contains("too many wrong passphrases"), "{}", r.text);
    assert!(
        !r.text.contains("admin passphrase: "),
        "it must not even ask while locked: {}",
        r.text
    );
}

#[test]
fn nothing_the_human_typed_is_left_in_any_file_of_the_home() {
    let h = Home::new();
    assert!(h.admin(&["lease-cap", "100"], &[]).ok());
    let _ = run_on_pty(klatch_in(&h, &["resume".to_string()]), &[WRONG]);
    assert!(h.admin(&["recovery"], &[]).ok());
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&h.0, &mut files);
    for f in files {
        let bytes = std::fs::read(&f).unwrap_or_default();
        for secret in [PASS, WRONG] {
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "{} holds a typed secret",
                f.display()
            );
        }
    }
}

#[test]
fn the_shell_asks_once_and_a_second_process_still_asks() {
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(
        klatch_in(&h, &["shell".to_string()]),
        &[PASS, "admin lease-cap 100", "admin lease-cap 200", "exit"],
    );
    assert!(r.ok(), "{}", r.text);
    assert_eq!(
        r.text.matches("admin passphrase: ").count(),
        1,
        "one passphrase for two commands:\n{}",
        r.text
    );
    assert!(!r.text.contains("error"), "{}", r.text);
    // the unlock died with that process: the next klatch asks again
    let again = run_on_pty(klatch_in(&h, &["lease-cap".to_string(), "300".to_string()]), &[PASS]);
    assert!(again.ok(), "{}", again.text);
    assert_eq!(again.text.matches("admin passphrase: ").count(), 1);
}

#[test]
fn the_shell_runs_only_the_five_families_and_refuses_a_wrong_passphrase_at_the_start() {
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(
        klatch_in(&h, &["shell".to_string()]),
        &[PASS, "post r hello", "admin shell", "exit"],
    );
    assert!(r.text.contains("only admin, worker, anchor, run and judge"), "{}", r.text);
    assert!(r.text.contains("already in the shell"), "{}", r.text);
    let bad = run_on_pty(klatch_in(&h, &["shell".to_string()]), &[WRONG, "admin lease-cap 100", "exit"]);
    assert!(!bad.ok());
    assert!(!bad.text.contains("klatch> "), "no prompt without the proof");
}

#[test]
fn init_passphrase_needs_two_matching_long_enough_entries() {
    let h = Home::new();
    let c = || h.launcher_command(&["admin", "init-passphrase"]);
    let short = run_on_pty(c(), &["short", "short"]);
    assert!(!short.ok());
    assert!(short.text.contains("at least"), "{}", short.text);
    let differ = run_on_pty(c(), &[PASS, NEW]);
    assert!(!differ.ok());
    assert!(differ.text.contains("differ"), "{}", differ.text);
    // nothing was set by either, so an admin act still says so
    let r = run_on_pty(klatch_in(&h, &["pause".to_string()]), &[PASS]);
    assert!(r.text.contains("init-passphrase"), "{}", r.text);
    let ok = run_on_pty(c(), &[PASS, PASS]);
    assert!(ok.ok(), "{}", ok.text);
    let after = run_on_pty(klatch_in(&h, &["lease-cap".to_string(), "100".to_string()]), &[PASS]);
    assert!(after.ok(), "{}", after.text);
}

#[test]
fn the_first_passphrase_cannot_be_set_without_a_terminal() {
    let h = Home::new();
    let (ok, text) = detached(h.launcher_command(&["admin", "init-passphrase"]), Some(PASS));
    assert!(!ok, "{text}");
    assert!(text.contains("terminal"), "{text}");
    // and so nothing is set
    let r = run_on_pty(klatch_in(&h, &["pause".to_string()]), &[PASS]);
    assert!(r.text.contains("init-passphrase"), "{}", r.text);
}

#[test]
fn changing_the_passphrase_needs_the_old_one_and_then_only_the_new_one_works() {
    let h = Home::new();
    h.set_passphrase();
    let c = || h.launcher_command(&["admin", "init-passphrase"]);
    let bad = run_on_pty(c(), &[WRONG, NEW, NEW]);
    assert!(!bad.ok(), "{}", bad.text);
    let ok = run_on_pty(c(), &[PASS, NEW, NEW]);
    assert!(ok.ok(), "{}", ok.text);
    let old = run_on_pty(klatch_in(&h, &["lease-cap".to_string(), "100".to_string()]), &[PASS]);
    assert!(!old.ok(), "the old passphrase still works:\n{}", old.text);
    let new = run_on_pty(klatch_in(&h, &["lease-cap".to_string(), "100".to_string()]), &[NEW]);
    assert!(new.ok(), "{}", new.text);
}

#[test]
fn the_recovery_code_resets_a_forgotten_passphrase_and_a_wrong_code_does_not() {
    let h = Home::new();
    let shown = h.admin(&["recovery"], &[]);
    assert!(shown.ok(), "{}", shown.text);
    let code = shown
        .text
        .lines()
        .find(|l| l.trim_start().starts_with("KLATCH-"))
        .unwrap_or_else(|| panic!("no recovery code in:\n{}", shown.text))
        .trim()
        .to_string();
    let c = || h.launcher_command(&["admin", "reset-passphrase"]);
    let bad = run_on_pty(c(), &["KLATCH-AAAA-AAAA", NEW, NEW]);
    assert!(!bad.ok(), "{}", bad.text);
    let ok = run_on_pty(c(), &[&code, NEW, NEW]);
    assert!(ok.ok(), "{}", ok.text);
    let old = run_on_pty(klatch_in(&h, &["lease-cap".to_string(), "100".to_string()]), &[PASS]);
    assert!(!old.ok());
    let new = run_on_pty(klatch_in(&h, &["lease-cap".to_string(), "100".to_string()]), &[NEW]);
    assert!(new.ok(), "{}", new.text);
    // without a terminal it is refused
    let (ok, text) = detached(c(), Some(&code));
    assert!(!ok, "{text}");
}

#[test]
fn the_unlock_window_is_shown_and_set_with_the_proof_and_is_bounded() {
    let h = Home::new();
    let shown = h.admin(&["unlock-window"], &[]);
    assert!(shown.text.contains("300 seconds"), "{}", shown.text);
    assert!(h.admin(&["unlock-window", "60"], &[]).ok());
    assert!(h.admin(&["unlock-window"], &[]).text.contains("60 seconds"));
    let bad = h.admin(&["unlock-window", "5"], &[]);
    assert!(!bad.ok(), "{}", bad.text);
    assert!(h.admin(&["unlock-window", "--off"], &[]).ok());
    assert!(h.admin(&["unlock-window"], &[]).text.contains("300 seconds"));
}

#[test]
fn the_destructive_acts_still_ask_for_the_target_typed_back_after_the_proof() {
    let h = Home::new();
    let no = h.admin(&["close", "lobby"], &["nope"]);
    assert!(!no.ok(), "{}", no.text);
    assert!(no.text.contains("not confirmed"), "{}", no.text);
    let yes = h.admin(&["close", "lobby"], &["lobby"]);
    assert!(yes.ok(), "{}", yes.text);
}

#[test]
fn a_member_cannot_take_the_name_the_proof_makes() {
    let h = Home::new();
    let o = h.run("human", &["join", "r"]);
    assert!(!o.status.success(), "a member named human joined");
}
