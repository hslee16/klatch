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
//!
//! This file holds the refusals of every command and the proof itself; the passphrase life cycle and recovery are in
//! `admin_proof_passphrase.rs`, the terminal-sharing cases and signals in `admin_proof_terminal.rs`.
mod common;

use common::admin::*;
use common::pty::*;
use common::*;

#[test]
fn the_table_has_a_row_for_every_admin_variant() {
    let h = Home::new();
    let table: Vec<&str> = admin_table(&h).iter().map(|(n, _)| *n).collect();
    let mut variants = admin_variants();
    variants.sort();
    let mut have: Vec<&str> = table.clone();
    have.sort();
    assert_eq!(
        variants, have,
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
            text.contains("terminal") || text.contains("no admin passphrase"),
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
        assert!(
            !ok,
            "{name} accepted a passphrase from the environment or a pipe:\n{text}"
        );
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
    assert_has(
        &audit.text,
        &["lease-cap@@
    @@H|audit|proof-ok@@
    @@H|audit|passphrase-set"],
    );
    assert!(
        !audit.text.contains(PASS),
        "the passphrase reached the audit"
    );
    // a wrong attempt is a row with a count and nothing else
    let bad = run_on_pty(admin_cmd(&h, &["resume"]), &[WRONG]);
    assert!(!bad.ok());
    let audit = h.admin(&["audit"], &[]);
    assert_has(
        &audit.text,
        &["proof-failed@@
    @@H|audit|failures=1"],
    );
    assert!(!audit.text.contains(WRONG), "a wrong attempt was kept");
    assert!(!audit.text.contains(PASS));
}

#[test]
fn stats_on_the_terminal_with_the_passphrase_prints_counts_and_is_audited() {
    let h = Home::new();
    let r = h.admin(&["stats"], &[]);
    assert!(r.ok(), "{}", r.text);
    assert_has(
        &r.text,
        &["klatch stats (counts only@@
    @@H|r|worker credentials: 0 issued"],
    );
    let bad = h.admin(&["stats", "--since", "yesterday"], &[]);
    assert!(!bad.ok() && bad.text.contains("YYYY-MM-DD"), "{}", bad.text);
    let audit = h.admin(&["audit"], &[]);
    assert_has(&audit.text, &["stats"]);
}

#[test]
fn three_wrong_passphrases_lock_even_the_right_one_out_for_a_while() {
    let h = Home::new();
    h.set_passphrase();
    let r = repeated_on_one_pty(admin_cmd(&h, &["pause"]), 4, &[WRONG, WRONG, WRONG, PASS]);
    assert_has(&r.text, &["too many wrong passphrases"]);
    assert_eq!(
        r.text.matches("admin passphrase to approve").count(),
        3,
        "it must not even ask while locked: {}",
        r.text
    );
}

#[test]
fn a_split_error_line_is_not_taken_for_a_prompt() {
    // the child writes "Error: " and the message in two writes; a read can end between them (seen on Linux CI)
    assert!(!is_prompt("admin passphrase to approve `x`: \nError: "));
    assert!(!is_prompt("Error: "));
    assert!(is_prompt("admin passphrase to approve `x`: "));
    assert!(is_prompt(
        "Error: wrong passphrase; nothing was done\nadmin passphrase to approve `x`: "
    ));
    assert!(is_prompt("klatch> "));
}

#[test]
fn a_pty_of_its_own_gets_its_own_failure_count_and_cannot_lock_another_terminal() {
    let h = Home::new();
    h.set_passphrase();
    // an agent's pty fails three times and is locked
    let r = repeated_on_one_pty(admin_cmd(&h, &["resume"]), 4, &[WRONG, WRONG, WRONG]);
    assert_has(&r.text, &["too many wrong passphrases"]);
    // the human's terminal (another pty) is asked and its passphrase is accepted
    let r = run_on_pty(admin_cmd(&h, &["resume"]), &[PASS]);
    assert!(
        r.text.contains("admin passphrase to approve") && !r.text.contains("too many"),
        "{}",
        r.text
    );
    assert!(r.ok(), "{}", r.text);
}

#[test]
fn nothing_the_human_typed_is_left_in_any_file_of_the_home() {
    let h = Home::new();
    assert!(h.admin(&["lease-cap", "100"], &[]).ok());
    let _ = run_on_pty(admin_cmd(&h, &["resume"]), &[WRONG]);
    assert!(h.admin(&["recovery"], &["recovery"]).ok());
    for f in files_under(&h.0) {
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
fn the_unlock_window_is_shown_and_set_with_the_proof_and_is_bounded() {
    let h = Home::new();
    let shown = h.admin(&["unlock-window"], &[]);
    assert_has(&shown.text, &["60 seconds"]);
    assert!(h.admin(&["unlock-window", "120"], &[]).ok());
    assert!(
        h.admin(&["unlock-window"], &[])
            .text
            .contains("120 seconds")
    );
    let bad = h.admin(&["unlock-window", "5"], &[]);
    assert!(!bad.ok(), "{}", bad.text);
    assert!(h.admin(&["unlock-window", "--off"], &[]).ok());
    assert!(h.admin(&["unlock-window"], &[]).text.contains("60 seconds"));
}

#[test]
fn the_destructive_acts_still_ask_for_the_target_typed_back_after_the_proof() {
    let h = Home::new();
    let no = h.admin(&["close", "lobby"], &["nope"]);
    assert!(!no.ok(), "{}", no.text);
    assert_has(&no.text, &["not confirmed"]);
    let yes = h.admin(&["close", "lobby"], &["lobby"]);
    assert!(yes.ok(), "{}", yes.text);
}

#[test]
fn a_member_cannot_take_the_name_the_proof_makes() {
    let h = Home::new();
    let o = h.run("human", &["join", "r"]);
    assert!(!o.status.success(), "a member named human joined");
}
