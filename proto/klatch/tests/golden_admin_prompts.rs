//! Golden text of the admin passphrase prompts (#1480): what a person sees when a program asks them to approve each
//! admin and launcher command, and the refusal when the passphrase is wrong. Characterisation of the behaviour that
//! went through five security reviews; a refactor of `proof`, `admin::prove` or `admin::route` must leave every byte
//! as it is. A throwaway home and throwaway passphrases, typed through a pty as the other admin tests do; nothing
//! here reads a real store or the Keychain. Regenerate on purpose only: `KLATCH_WRITE_GOLDEN=1`.
mod common;

use common::admin::*;
use common::golden::*;
use common::pty::*;
use common::*;

const SECRETS: [&str; 3] = [PASS, WRONG, NEW];

fn sheet(h: &Home) -> Sheet {
    Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary())
}

/// The arguments with the home's path taken off (a relative file name), so a path of any length (it varies from
/// machine to machine) cannot move where the prompt cuts a long word.
fn relative(h: &Home, args: &[String]) -> Vec<String> {
    let prefix = format!("{}/", h.0.display());
    args.iter()
        .map(|a| a.strip_prefix(&prefix).unwrap_or(a).to_string())
        .collect()
}

/// The command line a block is about, as a person would type it.
fn line(prefix: &str, args: &[String]) -> String {
    format!("$ klatch {prefix}{}", args.join(" "))
}

/// Run every row of `table` on a pty with a wrong passphrase into the golden `name`: as `klatch admin ...` when
/// `prefix` is `admin `, else as a launcher command.
fn wrong_passphrase_for(
    h: &Home,
    table: Vec<(&'static str, Vec<String>)>,
    prefix: &str,
    name: &str,
) {
    let mut s = sheet(h);
    for (_, args) in table {
        let args = relative(h, &args);
        let cmd = if prefix.is_empty() {
            klatch_raw(h, &args)
        } else {
            klatch_in(h, &args)
        };
        let r = run_on_pty(cmd, &[WRONG]);
        s.block(&line(prefix, &args), r.ok(), &r.text);
    }
    s.check(name, &SECRETS);
}

#[test]
fn every_admin_and_launcher_command_asks_with_its_own_words_and_refuses_a_wrong_passphrase() {
    let h = Home::new();
    h.set_passphrase();
    let table = admin_table(&h);
    wrong_passphrase_for(&h, table, "admin ", "admin_prompts_wrong_passphrase");
    let table = launcher_table(&h);
    wrong_passphrase_for(&h, table, "", "launcher_prompts_wrong_passphrase");
}

/// The words after the command, cut and cleaned for the prompt (#1283): printable ASCII only, a long word shows its
/// head and tail with the count unseen, a long command is cut at a word.
#[test]
fn the_prompt_shows_arguments_cleaned_and_cut_to_head_and_tail() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    let word = |n: usize| -> String { (0..n).map(|i| char::from(b'a' + (i % 26) as u8)).collect() };
    let many: Vec<String> = std::iter::once("scrub-add".to_string())
        .chain((0..40).map(|i| format!("w{i:02}-padding")))
        .collect();
    let cases: Vec<(&str, Vec<String>)> = vec![
        (
            "escape, bidi override, zero width, CRLF and a fake prompt in an argument",
            owned(&[
                "close",
                "lobby\u{202e}live\u{1b}[31m\u{200b}red\r\nadmin passphrase: ",
            ]),
        ),
        (
            "non-ASCII letters",
            owned(&["close", "caf\u{e9}-\u{65e5}\u{672c}"]),
        ),
        (
            "a word of exactly 48 characters is shown whole",
            vec!["close".into(), word(48)],
        ),
        (
            "a word of 49 characters is cut",
            vec!["close".into(), word(49)],
        ),
        (
            "a word of 300 characters is cut",
            vec!["close".into(), word(300)],
        ),
        (
            "a space inside a word is kept",
            owned(&["close", "two words"]),
        ),
        ("forty words are cut at a word", many),
        (
            "text before the family word",
            owned(&["--as-member", "bob", "close", "lobby"]),
        ),
    ];
    for (label, args) in cases {
        let r = run_on_pty(klatch_in(&h, &args), &[WRONG, "x"]);
        s.block(
            &format!("{label}\n{}", line("admin ", &args)),
            r.ok(),
            &r.text,
        );
    }
    let long = word(120);
    let w = h.launcher_command(&["worker", "revoke", &long]);
    let r = run_on_pty(w, &[WRONG]);
    s.block("a launcher command with a long name", r.ok(), &r.text);
    s.check("admin_prompts_arguments", &SECRETS);
}

#[test]
fn a_recovery_code_given_as_an_argument_is_never_shown_in_the_prompt() {
    let code = "KLATCH-HIDE-ME-PLEASE";
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    for args in [
        owned(&["recover", code]),
        owned(&["--as-member", "bob", "recover", code]),
        owned(&["recover", "--label", "private", "--", code]),
        owned(&["close", "recover"]),
    ] {
        let r = run_on_pty(klatch_in(&h, &args), &[WRONG, WRONG]);
        let shown = line("admin ", &args).replace(code, "<code>");
        s.block(&shown, r.ok(), &r.text);
    }
    let mut forbidden = SECRETS.to_vec();
    forbidden.push("HIDE-ME");
    s.check("admin_prompts_recovery_code_hidden", &forbidden);
}

#[test]
fn the_shell_prompt_names_every_command_it_runs_and_asks_once() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    let r = run_on_pty(admin_cmd(&h, &["shell"]), &[WRONG]);
    s.block("shell, wrong passphrase", r.ok(), &r.text);
    let r = run_on_pty(
        admin_cmd(&h, &["shell"]),
        &[
            PASS,
            "admin lease-cap 100",
            "worker revoke w-1",
            "post r hello",
            "admin shell",
            "frobnicate",
            "exit",
        ],
    );
    s.block("shell, right passphrase, lines typed", r.ok(), &r.text);
    s.check("admin_shell_text", &SECRETS);
}
