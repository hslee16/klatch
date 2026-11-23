//! Golden text of the admin passphrase failure limits and the passphrase and recovery flows (#1480): the lockout and
//! its wait, the per-terminal count, the recovery-code attempt limit, setting and changing the passphrase, and the
//! typed confirmations. A throwaway home, throwaway passphrases and fake recovery codes typed through a pty; the
//! real recovery code of the throwaway store is read only to type it back and is never kept (`forbid`). The wait is
//! shown as `<N>` because it counts down in real time. Regenerate on purpose only: `KLATCH_WRITE_GOLDEN=1`.
mod common;

use common::admin::*;
use common::golden::*;
use common::pty::*;
use common::*;

const FAKE: &str = "KLATCH-AAAA-AAAA";

fn sheet(h: &Home) -> Sheet {
    Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary())
}

fn init(h: &Home) -> std::process::Command {
    h.launcher_command(&["admin", "init-passphrase"])
}

#[test]
fn three_wrong_passphrases_lock_a_terminal_and_a_new_terminal_has_its_own_count() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = sheet(&h);
    let r = repeated_on_one_pty(admin_cmd(&h, &["pause"]), 4, &[WRONG, WRONG, WRONG, PASS]);
    s.block(
        "four runs on one terminal: three wrong, then the right one",
        r.ok(),
        &r.text,
    );
    let r = run_on_pty(admin_cmd(&h, &["resume"]), &[PASS]);
    s.block(
        "a terminal of its own is not locked by that",
        r.ok(),
        &r.text,
    );
    s.check("limit_per_terminal_lockout", &[PASS, WRONG, NEW]);
}

#[test]
fn the_recovery_code_has_its_own_attempt_limit_and_waits_after_three_wrong_codes() {
    let h = Home::new();
    let backup = h.0.join("backup.db");
    assert!(h.admin(&["backup", backup.to_str().unwrap()], &[]).ok());
    let code = recovery_code_of(&h);
    for f in ["rooms.db", "rooms.db-wal", "rooms.db-shm"] {
        let _ = std::fs::remove_file(h.0.join(f));
    }
    let args = owned(&["restore", backup.to_str().unwrap()]);
    let restore = |typed: &[&str]| run_on_pty(klatch_in(&h, &args), typed);
    let mut s = sheet(&h);
    for n in 1..=3 {
        let r = restore(&[FAKE, "restore"]);
        s.block(
            &format!("restore, wrong recovery code, attempt {n}"),
            r.ok(),
            &r.text,
        );
    }
    let r = restore(&[&code, "restore"]);
    s.block(
        "restore, the fourth attempt is refused before it is asked, even with the right code",
        r.ok(),
        &r.text,
    );
    let r = restore(&[&code, "nope"]);
    assert!(!r.ok());
    s.check("limit_recovery_attempts", &[PASS, WRONG, NEW, &code]);
}

#[test]
fn setting_and_changing_the_passphrase_says_what_is_wrong_with_an_entry() {
    let h = Home::new();
    let mut s = sheet(&h);
    let r = run_on_pty(init(&h), &["short", "short", "short"]);
    s.block("first passphrase: too short, three times", r.ok(), &r.text);
    let r = run_on_pty(init(&h), &[PASS, NEW, PASS, NEW, PASS, NEW]);
    s.block(
        "first passphrase: the two entries differ, three times",
        r.ok(),
        &r.text,
    );
    let r = run_on_pty(admin_cmd(&h, &["pause"]), &[PASS]);
    s.block("an admin command while none is set", r.ok(), &r.text);
    let r = run_on_pty(init(&h), &["short", PASS, NEW, PASS, PASS]);
    s.block(
        "first passphrase: short, then differing, then a matching pair",
        r.ok(),
        &r.text,
    );
    let (ok, text) = detached(init(&Home::new()), Some(PASS));
    s.block("first passphrase with no terminal", ok, &text);
    let r = run_on_pty(init(&h), &[WRONG, NEW, NEW]);
    s.block("change: a wrong old passphrase", r.ok(), &r.text);
    let r = run_on_pty(init(&h), &[PASS, NEW, NEW]);
    s.block(
        "change: the right old passphrase and a new pair",
        r.ok(),
        &r.text,
    );
    s.check("flow_passphrase_set_and_change", &[PASS, WRONG, NEW]);
}

#[test]
fn recovery_codes_reset_recover_and_show_only_after_a_typed_confirmation() {
    let h = Home::new();
    h.set_passphrase();
    let code = recovery_code_of(&h);
    let mut s = sheet(&h);
    let r = run_on_pty(admin_cmd(&h, &["recovery"]), &[PASS, "nope"]);
    s.block("recovery: the confirmation is not typed", r.ok(), &r.text);
    let reset = || h.launcher_command(&["admin", "reset-passphrase"]);
    let r = run_on_pty(reset(), &[FAKE, NEW, NEW]);
    s.block("reset-passphrase: a wrong recovery code", r.ok(), &r.text);
    let (ok, text) = detached(reset(), Some(FAKE));
    s.block("reset-passphrase: no terminal", ok, &text);
    let r = run_on_pty(admin_cmd(&h, &["close", "lobby"]), &[PASS, "nope"]);
    s.block(
        "close: the confirmation says what is confirmed",
        r.ok(),
        &r.text,
    );
    // the key is gone from the keystore: the store cannot be read, so the recovery code approves
    std::fs::remove_dir_all(h.0.join("keys")).unwrap();
    std::fs::create_dir(h.0.join("keys")).unwrap();
    let r = run_on_pty(admin_cmd(&h, &["recover", FAKE]), &["recover"]);
    s.block("recover: a wrong code", r.ok(), &r.text);
    let args = owned(&["recover", &code]);
    let r = run_on_pty(klatch_in(&h, &args), &["nope"]);
    s.block(
        "recover: the right code, the confirmation not typed",
        r.ok(),
        &r.text.replace(&code, "<code>"),
    );
    let r = run_on_pty(klatch_in(&h, &args), &["recover"]);
    s.block(
        "recover: the right code and the typed confirmation",
        r.ok(),
        &r.text.replace(&code, "<code>"),
    );
    s.check("flow_recovery_confirmations", &[PASS, WRONG, NEW, &code]);
}
