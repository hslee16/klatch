//! The admin passphrase when the terminal is shared or not the right one (#1233, #1271, #1278): a program that pipes,
//! owns a pty of its own or leads a session of its own, and what a signal leaves behind. The refusals are in
//! `admin_proof.rs`.
mod common;

use common::admin::*;
use common::pty::*;
use common::*;

#[test]
fn a_pipe_never_supplies_the_confirmation_even_when_the_terminal_is_shared() {
    // an agent started from the human's terminal inherits it as the controlling terminal and pipes its own input
    let h = Home::new();
    h.set_passphrase();
    let r = common::pty::run_on_pty_piped(
        admin_cmd(&h, &["close", "lobby"]),
        &[PASS, "nope"],
        "lobby\n",
    );
    assert!(!r.ok(), "the piped line confirmed the act:\n{}", r.text);
    assert_has(&r.text, &["not confirmed"]);
    // and the human, who is shown the prompt, is told exactly what the passphrase would approve
    assert_has(&r.text, &["admin close lobby"]);
}

#[test]
fn the_shell_refuses_piped_input_even_when_the_terminal_is_shared() {
    let h = Home::new();
    h.set_passphrase();
    let r = common::pty::run_on_pty_piped(
        admin_cmd(&h, &["shell"]),
        &[PASS],
        "admin lease-cap 100\nexit\n",
    );
    assert!(!r.ok(), "{}", r.text);
    assert_has(&r.text, &["terminal"]);
    assert!(
        !r.text.contains("admin passphrase"),
        "the human must not be asked to unlock a shell that reads a pipe:\n{}",
        r.text
    );
    assert_lacks(&r.text, &["klatch> "]);
}

#[test]
fn secrets_are_never_printed_to_a_pipe_even_on_a_shared_terminal() {
    // `klatch admin recovery | cat` from a program sharing the human's terminal: refused before any prompt
    let h = Home::new();
    h.set_passphrase();
    let f = h.0.join("a.jsonl").to_string_lossy().into_owned();
    let cases: Vec<Vec<String>> = vec![
        vec!["recovery".into()],
        vec!["export".into(), "r".into()],
        vec!["run-archive".into(), f, "--run".into(), "r".into()],
        vec!["rotate-key".into()],
        vec!["rotate-label-key".into(), "private".into()],
    ];
    for args in cases {
        let r = common::pty::run_on_pty_stdout_piped(klatch_in(&h, &args), &[PASS, "x", "x"]);
        assert!(!r.ok(), "{args:?} printed to a pipe:\n{}", r.text);
        assert!(r.piped_stdout.is_empty(), "{args:?}: {}", r.piped_stdout);
        assert!(r.text.contains("standard output"), "{args:?}: {}", r.text);
        assert!(
            !r.text.contains("admin passphrase to approve"),
            "{args:?}: the human was asked to approve a command that cannot show them its answer:\n{}",
            r.text
        );
    }
}

#[test]
fn text_from_arguments_cannot_dress_up_a_prompt() {
    let h = Home::new();
    h.set_passphrase();
    let hostile = "lobby\u{202e}live\u{1b}[31m\u{200b}red\r\nadmin passphrase: ";
    let r = run_on_pty(
        klatch_in(&h, &["close".to_string(), hostile.to_string()]),
        &[WRONG, "x"],
    );
    for bad in ['\u{202e}', '\u{1b}', '\u{200b}'] {
        assert!(
            !r.text.contains(bad),
            "{bad:?} reached the terminal: {:?}",
            r.text
        );
    }
}

#[test]
fn a_pty_the_caller_owns_on_stdin_and_stdout_is_not_the_controlling_terminal() {
    // the human's terminal (A) is the controlling one; the agent gave fd 0 and 1 a pty of its own (B) and reads it
    let h = Home::new();
    h.set_passphrase();
    for args in [
        vec!["recovery".to_string()],
        vec!["shell".to_string()],
        vec!["export".to_string(), "r".to_string()],
        vec!["audit".to_string()],
    ] {
        let r = common::pty::run_on_pty_split(klatch_in(&h, &args), &[PASS, "x", "x"]);
        assert!(!r.ok(), "{args:?} ran:\n{}\n{}", r.text, r.piped_stdout);
        assert!(
            !r.text.contains("admin passphrase to approve"),
            "{args:?}: the human was asked to approve a command whose output goes to someone else's pty:\n{}",
            r.text
        );
        assert!(
            !r.piped_stdout.contains("KLATCH-"),
            "{args:?}: {}",
            r.piped_stdout
        );
        assert!(r.text.contains("terminal"), "{args:?}: {}", r.text);
    }
}

#[test]
fn stored_text_acts_run_when_stdin_is_a_pipe_and_the_terminal_is_the_controlling_one() {
    // `klatch admin audit < file` or a pipe into it: stdout and stderr are the person's own terminal, so the prompt
    // (on /dev/tty) and the output are one device; it must not be refused as "stdout is not your terminal" (#1285)
    let h = Home::new();
    h.set_passphrase();
    for args in [vec!["audit".to_string()], vec!["refusals".to_string()]] {
        let r = common::pty::run_on_pty_piped(klatch_in(&h, &args), &[PASS], "ignored\n");
        assert!(r.ok(), "{args:?} was refused:\n{}", r.text);
        assert!(
            r.text.contains("admin passphrase to approve"),
            "{args:?}: {}",
            r.text
        );
    }
}

#[test]
fn a_refused_guard_says_how_to_run_it() {
    let h = Home::new();
    h.set_passphrase();
    let r = common::pty::run_on_pty_stdout_piped(admin_cmd(&h, &["audit"]), &[PASS]);
    assert!(!r.ok());
    assert!(
        r.text.contains("standard error"),
        "the refusal should name what to check: {}",
        r.text
    );
}

#[test]
fn rate_limit_list_is_refused_to_a_pipe() {
    let h = Home::new();
    h.set_passphrase();
    let args = vec!["rate-limit".to_string(), "--list".to_string()];
    let r = common::pty::run_on_pty_stdout_piped(klatch_in(&h, &args), &[PASS, "x"]);
    assert!(!r.ok(), "printed to a pipe:\n{}", r.text);
    assert!(r.piped_stdout.is_empty(), "{}", r.piped_stdout);
    assert_lacks(&r.text, &["admin passphrase to approve"]);
}

#[test]
fn everything_that_prints_stored_text_is_refused_to_a_pipe() {
    let h = Home::new();
    h.set_passphrase();
    let cases: Vec<Vec<String>> = vec![
        vec!["scrub-list".into()],
        vec!["review-queue".into()],
        vec!["refusals".into()],
        vec!["audit".into()],
        vec!["archives".into()],
        vec!["stats".into()],
        vec!["stats".into(), "--json".into()],
    ];
    for args in cases {
        let r = common::pty::run_on_pty_stdout_piped(klatch_in(&h, &args), &[PASS, "x"]);
        assert!(!r.ok(), "{args:?} printed to a pipe:\n{}", r.text);
        assert!(r.piped_stdout.is_empty(), "{args:?}: {}", r.piped_stdout);
        assert!(
            !r.text.contains("admin passphrase to approve"),
            "{args:?}: {}",
            r.text
        );
    }
}

#[test]
fn a_pty_in_a_session_of_its_own_on_stdout_is_not_where_the_prompt_is() {
    // the child leads a session on pty P (stdout, stderr) while standard input is the human's own terminal H: the
    // prompt would appear on H and the answer go to whoever holds P
    let h = Home::new();
    h.set_passphrase();
    for args in [
        vec!["recovery".to_string()],
        vec!["export".to_string(), "r".to_string()],
        vec!["audit".to_string()],
    ] {
        let r = common::pty::run_on_pty_forked_session(klatch_in(&h, &args), &[PASS, "x", "x"]);
        assert!(!r.ok(), "{args:?} ran:\n{}\n{}", r.text, r.piped_stdout);
        assert!(
            !r.text.contains("admin passphrase to approve"),
            "{args:?}: the human was asked to approve a command whose answer goes to another session's pty:\n{}",
            r.text
        );
        assert!(
            !r.piped_stdout.contains("KLATCH-"),
            "{args:?}: {}",
            r.piped_stdout
        );
        assert!(
            r.piped_stdout.contains("terminal"),
            "{args:?}: {}",
            r.piped_stdout
        );
    }
}

#[test]
fn the_shell_needs_input_and_output_on_one_terminal() {
    let h = Home::new();
    h.set_passphrase();
    let r = common::pty::run_on_pty_forked_session(admin_cmd(&h, &["shell"]), &[PASS]);
    assert!(!r.ok(), "{}\n{}", r.text, r.piped_stdout);
    assert_lacks(&r.text, &["admin passphrase to approve"]);
}

#[test]
fn standard_error_must_be_the_terminal_or_nothing_for_the_acts_that_print_stored_text() {
    let h = Home::new();
    h.set_passphrase();
    let r = common::pty::run_on_pty_stderr_piped(admin_cmd(&h, &["audit"]), &[PASS]);
    assert!(!r.ok(), "{}\n{}", r.text, r.piped_stdout);
    assert_lacks(&r.text, &["admin passphrase to approve"]);
}

#[test]
fn a_recovery_code_is_hidden_whatever_sits_before_the_subcommand_and_a_room_named_recover_is_not() {
    let code = "KLATCH-HIDE-ME-PLEASE";
    let shapes: Vec<Vec<String>> = vec![
        // a global flag between the family and the subcommand
        vec!["admin", "--as-member", "bob", "recover", code],
        // a flag value that is itself a family name
        vec!["--as-member", "admin", "admin", "recover", code],
        // flags and `--` after it
        vec!["admin", "recover", "--label", "private", "--", code],
    ]
    .into_iter()
    .map(|v| v.into_iter().map(String::from).collect())
    .collect();
    for args in shapes {
        // a home of its own each time: three wrong passphrases in one would lock the rest out
        let h = Home::new();
        h.set_passphrase();
        let r = run_on_pty(klatch_raw(&h, &args), &[WRONG, WRONG]);
        assert!(
            !r.text.contains("HIDE-ME"),
            "{args:?} showed the code:\n{}",
            r.text
        );
        assert!(r.text.contains("admin recover"), "{args:?}: {}", r.text);
    }
    // under another command the word is just a name
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(admin_cmd(&h, &["close", "recover"]), &[WRONG]);
    assert_has(&r.text, &["admin close recover"]);
}

#[test]
fn a_sigterm_during_the_passphrase_prompt_leaves_the_terminal_with_echo_on() {
    terminal_after_signal(libc::SIGTERM);
}

#[test]
fn a_sighup_during_the_passphrase_prompt_leaves_the_terminal_with_echo_on() {
    terminal_after_signal(libc::SIGHUP);
}

#[test]
fn a_sigint_during_the_passphrase_prompt_leaves_the_terminal_with_echo_on() {
    terminal_after_signal(libc::SIGINT);
}
