//! Golden text of every terminal refusal of the admin proof (#1480): no terminal, standard output or error or input
//! that is not the terminal, no passphrase set yet. The words are what a person reads when a program is refused in
//! their name, so a refactor of `proof::tty`, `admin::prove` or `admin::route` may not move one byte of them.
//! Throwaway home and passphrases; nothing real is touched. Regenerate on purpose only: `KLATCH_WRITE_GOLDEN=1`.
mod common;

use common::admin::*;
use common::golden::*;
use common::pty::*;
use common::*;

const SECRETS: [&str; 3] = [PASS, WRONG, NEW];

fn sheet(h: &Home) -> Sheet {
    Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary())
}

/// A table row's arguments with the home's path taken off (see `golden_admin_prompts`).
fn rel(h: &Home, args: &[String]) -> Vec<String> {
    let prefix = format!("{}/", h.0.display());
    args.iter()
        .map(|a| a.strip_prefix(&prefix).unwrap_or(a).to_string())
        .collect()
}

/// A run's whole account: what the terminal showed, and what a pipe carried (the pipe must be empty).
fn both(r: &PtyRun) -> String {
    if r.piped_stdout.is_empty() {
        r.text.clone()
    } else {
        format!("{}[pipe carried]\n{}", r.text, r.piped_stdout)
    }
}

#[test]
fn every_command_is_refused_with_no_terminal_in_the_same_words() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    for (_, args) in admin_table(&h) {
        let (ok, text) = detached(klatch_in(&h, &args), None);
        s.block(
            &format!("$ klatch admin {}", rel(&h, &args).join(" ")),
            ok,
            &text,
        );
    }
    for (_, args) in launcher_table(&h) {
        let (ok, text) = detached(klatch_raw(&h, &args), None);
        s.block(&format!("$ klatch {}", rel(&h, &args).join(" ")), ok, &text);
    }
    let (ok, text) = detached(klatch_in(&h, &owned(&["close", "lobby"])), Some(PASS));
    s.block(
        "a passphrase piped to a command with no terminal is not read",
        ok,
        &text,
    );
    s.check("refusal_no_terminal", &SECRETS);
}

#[test]
fn while_no_passphrase_is_set_every_command_says_how_to_set_one() {
    let h = Home::new();
    assert!(h.run("alice", &["join", "r"]).status.success());
    let mut s = sheet(&h);
    for (name, args) in admin_table(&h) {
        if ["init-passphrase", "reset-passphrase"].contains(&name) {
            continue;
        }
        let r = run_on_pty(klatch_in(&h, &args), &[PASS, PASS]);
        s.block(
            &format!("$ klatch admin {}", rel(&h, &args).join(" ")),
            r.ok(),
            &r.text,
        );
    }
    for (_, args) in launcher_table(&h) {
        let r = run_on_pty(klatch_raw(&h, &args), &[PASS, PASS]);
        s.block(
            &format!("$ klatch {}", rel(&h, &args).join(" ")),
            r.ok(),
            &r.text,
        );
    }
    s.check("refusal_no_passphrase_set", &SECRETS);
}

#[test]
fn a_command_that_prints_a_secret_or_stored_text_is_refused_when_its_output_is_not_the_terminal() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    let f = "a.jsonl".to_string();
    let cases: Vec<Vec<String>> = vec![
        owned(&["recovery"]),
        owned(&["export", "r"]),
        vec!["run-archive".into(), f, "--run".into(), "r".into()],
        owned(&["rotate-key"]),
        owned(&["rotate-label-key", "private"]),
        owned(&["rate-limit", "--list"]),
        owned(&["scrub-list"]),
        owned(&["review-queue"]),
        owned(&["refusals"]),
        owned(&["audit"]),
        owned(&["archives"]),
        owned(&["stats"]),
        owned(&["stats", "--json"]),
    ];
    for args in &cases {
        let r = run_on_pty_stdout_piped(klatch_in(&h, args), &[PASS, "x", "x"]);
        s.block(
            &format!("standard output is a pipe: klatch admin {}", args.join(" ")),
            r.ok(),
            &both(&r),
        );
    }
    for args in cases.iter().filter(|a| a[0] == "audit") {
        let r = run_on_pty_stderr_piped(klatch_in(&h, args), &[PASS]);
        s.block(
            "standard error is a pipe: klatch admin audit",
            r.ok(),
            &both(&r),
        );
    }
    s.check("refusal_stdout_pipe", &SECRETS);
}

#[test]
fn a_terminal_that_is_not_the_controlling_one_is_refused_in_the_same_words() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    for args in [
        owned(&["recovery"]),
        owned(&["shell"]),
        owned(&["export", "r"]),
        owned(&["audit"]),
    ] {
        let r = run_on_pty_split(klatch_in(&h, &args), &[PASS, "x", "x"]);
        s.block(
            &format!(
                "input and output on a pty of the caller: klatch admin {}",
                args.join(" ")
            ),
            r.ok(),
            &both(&r),
        );
    }
    for args in [
        owned(&["recovery"]),
        owned(&["export", "r"]),
        owned(&["audit"]),
        owned(&["shell"]),
    ] {
        let r = run_on_pty_forked_session(klatch_in(&h, &args), &[PASS, "x", "x"]);
        s.block(
            &format!(
                "a session of its own on a pty: klatch admin {}",
                args.join(" ")
            ),
            r.ok(),
            &both(&r),
        );
    }
    s.check("refusal_other_terminal", &SECRETS);
}

#[test]
fn input_that_is_a_pipe_never_supplies_a_confirmation_or_a_shell() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    let r = run_on_pty_piped(
        admin_cmd(&h, &["close", "lobby"]),
        &[PASS, "nope"],
        "lobby\n",
    );
    s.block(
        "a piped line is not the confirmation: klatch admin close lobby",
        r.ok(),
        &both(&r),
    );
    let r = run_on_pty_piped(
        admin_cmd(&h, &["shell"]),
        &[PASS],
        "admin lease-cap 100\nexit\n",
    );
    s.block(
        "the shell does not read a pipe: klatch admin shell",
        r.ok(),
        &both(&r),
    );
    let r = run_on_pty_piped(admin_cmd(&h, &["refusals"]), &[PASS], "ignored\n");
    s.block(
        "a stored text act runs with a piped stdin: klatch admin refusals",
        r.ok(),
        &both(&r),
    );
    s.check("refusal_stdin_pipe", &SECRETS);
}
