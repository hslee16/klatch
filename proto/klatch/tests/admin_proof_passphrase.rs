//! The admin passphrase life cycle and its recovery (#1233, #1271, #1281): the shell, setting and changing the passphrase,
//! the recovery code, restore, and what the prompts say. The refusals are in `admin_proof.rs`.
mod common;

use common::admin::*;
use common::pty::*;
use common::*;

#[test]
fn the_shell_asks_once_and_a_second_process_still_asks() {
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(
        admin_cmd(&h, &["shell"]),
        &[PASS, "admin lease-cap 100", "admin lease-cap 200", "exit"],
    );
    assert!(r.ok(), "{}", r.text);
    assert_eq!(
        r.text.matches("admin passphrase to approve").count(),
        1,
        "one passphrase for two commands:\n{}",
        r.text
    );
    assert_lacks(&r.text, &["error"]);
    // the unlock died with that process: the next klatch asks again
    let again = run_on_pty(admin_cmd(&h, &["lease-cap", "300"]), &[PASS]);
    assert!(again.ok(), "{}", again.text);
    assert_eq!(again.text.matches("admin passphrase to approve").count(), 1);
}

#[test]
fn the_shell_runs_only_the_five_families_and_refuses_a_wrong_passphrase_at_the_start() {
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(
        admin_cmd(&h, &["shell"]),
        &[PASS, "post r hello", "admin shell", "exit"],
    );
    assert_has(
        &r.text,
        &[
            "only admin, worker, anchor, run and judge",
            "already in the shell",
        ],
    );
    let bad = run_on_pty(
        admin_cmd(&h, &["shell"]),
        &[WRONG, "admin lease-cap 100", "exit"],
    );
    assert!(!bad.ok());
    assert!(
        !bad.text.contains("klatch> "),
        "no prompt without the proof"
    );
}

#[test]
fn init_passphrase_needs_two_matching_long_enough_entries() {
    let h = Home::new();
    let c = || h.launcher_command(&["admin", "init-passphrase"]);
    let short = run_on_pty(c(), &["short", "short", "short"]);
    assert!(!short.ok());
    assert_has(&short.text, &["at least"]);
    let differ = run_on_pty(c(), &[PASS, NEW, PASS, NEW, PASS, NEW]);
    assert!(!differ.ok());
    assert_has(&differ.text, &["differ"]);
    // nothing was set by either, so an admin act still says so
    let r = run_on_pty(admin_cmd(&h, &["pause"]), &[PASS]);
    assert_has(&r.text, &["init-passphrase"]);
    // asked again: short, mismatched, then a matching pair
    let ok = run_on_pty(c(), &["short", PASS, NEW, PASS, PASS]);
    assert!(ok.ok(), "{}", ok.text);
    let after = run_on_pty(admin_cmd(&h, &["lease-cap", "100"]), &[PASS]);
    assert!(after.ok(), "{}", after.text);
}

#[test]
fn the_first_passphrase_cannot_be_set_without_a_terminal() {
    let h = Home::new();
    let (ok, text) = detached(
        h.launcher_command(&["admin", "init-passphrase"]),
        Some(PASS),
    );
    assert!(!ok, "{text}");
    assert!(text.contains("terminal"), "{text}");
    // and so nothing is set
    let r = run_on_pty(admin_cmd(&h, &["pause"]), &[PASS]);
    assert_has(&r.text, &["init-passphrase"]);
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
    let old = run_on_pty(admin_cmd(&h, &["lease-cap", "100"]), &[PASS]);
    assert!(!old.ok(), "the old passphrase still works:\n{}", old.text);
    let new = run_on_pty(admin_cmd(&h, &["lease-cap", "100"]), &[NEW]);
    assert!(new.ok(), "{}", new.text);
}

#[test]
fn the_recovery_code_resets_a_forgotten_passphrase_and_a_wrong_code_does_not() {
    let h = Home::new();
    let shown = h.admin(&["recovery"], &["recovery"]);
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
    let old = run_on_pty(admin_cmd(&h, &["lease-cap", "100"]), &[PASS]);
    assert!(!old.ok());
    let new = run_on_pty(admin_cmd(&h, &["lease-cap", "100"]), &[NEW]);
    assert!(new.ok(), "{}", new.text);
    // without a terminal it is refused
    let (ok, text) = detached(c(), Some(&code));
    assert!(!ok, "{text}");
}

#[test]
fn recover_works_when_the_store_key_is_lost() {
    let h = Home::new();
    let code = recovery_code_of(&h);
    // the key is gone from the keystore; the store (and the verifier in it) cannot be read
    std::fs::remove_dir_all(h.0.join("keys")).unwrap();
    std::fs::create_dir(h.0.join("keys")).unwrap();
    // a detached process cannot do it
    let (ok, text) = detached(klatch_in(&h, &["recover".to_string(), code.clone()]), None);
    assert!(!ok, "{text}");
    assert!(text.contains("terminal"), "{text}");
    // a wrong code, or no typed confirmation, changes nothing
    let bad = run_on_pty(
        admin_cmd(&h, &["recover", "KLATCH-AAAA-AAAA"]),
        &["recover"],
    );
    assert!(!bad.ok(), "{}", bad.text);
    let unconfirmed = run_on_pty(
        klatch_in(&h, &["recover".to_string(), code.clone()]),
        &["nope"],
    );
    assert!(!unconfirmed.ok(), "{}", unconfirmed.text);
    assert_has(&unconfirmed.text, &["not confirmed"]);
    // the right code and the typed confirmation put the key back
    let ok = run_on_pty(klatch_in(&h, &["recover".to_string(), code]), &["recover"]);
    assert!(ok.ok(), "{}", ok.text);
    assert_has(&ok.text, &["the key is back"]);
    // and the store is readable again, with its passphrase
    let after = h.admin(&["lease-cap", "100"], &[]);
    assert!(after.ok(), "{}", after.text);
}

#[test]
fn restore_works_when_the_store_is_lost_and_asks_for_the_recovery_code() {
    let h = Home::new();
    let backup = h.0.join("backup.db");
    let made = h.admin(&["backup", backup.to_str().unwrap()], &[]);
    assert!(made.ok(), "{}", made.text);
    let code = recovery_code_of(&h);
    for f in ["rooms.db", "rooms.db-wal", "rooms.db-shm"] {
        let _ = std::fs::remove_file(h.0.join(f));
    }
    let path = backup.to_str().unwrap().to_string();
    let restore =
        |typed: &[&str]| run_on_pty(klatch_in(&h, &["restore".to_string(), path.clone()]), typed);
    let (ok, text) = detached(klatch_in(&h, &["restore".to_string(), path.clone()]), None);
    assert!(!ok, "{text}");
    let bad = restore(&["KLATCH-AAAA-AAAA", "restore"]);
    assert!(!bad.ok(), "{}", bad.text);
    let good = restore(&[&code, "restore"]);
    assert!(good.ok(), "{}", good.text);
    // the backup carried the verifier: the passphrase works again
    let after = h.admin(&["lease-cap", "100"], &[]);
    assert!(after.ok(), "{}", after.text);
}

#[test]
fn restore_without_a_store_waits_after_three_wrong_codes_and_audits_once_the_store_is_back() {
    let h = Home::new();
    let backup = h.0.join("backup.db");
    let made = h.admin(&["backup", backup.to_str().unwrap()], &[]);
    assert!(made.ok(), "{}", made.text);
    let code = recovery_code_of(&h);
    for f in ["rooms.db", "rooms.db-wal", "rooms.db-shm"] {
        let _ = std::fs::remove_file(h.0.join(f));
    }
    let path = backup.to_str().unwrap().to_string();
    let restore =
        |typed: &[&str]| run_on_pty(klatch_in(&h, &["restore".to_string(), path.clone()]), typed);
    for _ in 0..3 {
        let bad = restore(&["KLATCH-AAAA-AAAA", "restore"]);
        assert!(!bad.ok(), "{}", bad.text);
    }
    // the fourth is refused before it is asked, even with the right code
    let waits = restore(&[&code, "restore"]);
    assert!(!waits.ok(), "{}", waits.text);
    assert_has(&waits.text, &["try again in"]);
    // the attempts sit in a file beside the store; clearing the wait is the owner's act on their own files
    let file = h.0.join("rooms.db.recovery-fails");
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 3);
    std::fs::remove_file(&file).unwrap();
    let good = restore(&[&code, "restore"]);
    assert!(good.ok(), "{}", good.text);
    // a success with nothing charged before it still leaves one attempt, recorded in the audit once the store opens
    assert!(!file.exists(), "the attempts file outlived the restore");
    let audit = h.admin(&["audit"], &[]);
    assert_has(&audit.text, &["recovery-attempts"]);
}

#[test]
fn recovery_asks_for_a_typed_confirmation_before_it_shows_the_code() {
    let h = Home::new();
    h.set_passphrase();
    let no = run_on_pty(admin_cmd(&h, &["recovery"]), &[PASS, "nope"]);
    assert!(!no.ok(), "{}", no.text);
    assert_has(&no.text, &["not confirmed"]);
    assert_lacks(&no.text, &["KLATCH-"]);
}

#[test]
fn recover_without_a_store_never_plants_a_key() {
    // a valid code from another store, and a home where no store exists: nothing may be planted
    let a = Home::new();
    let code = recovery_code_of(&a);
    let b = Home::new();
    let r = run_on_pty(
        klatch_in(&b, &["recover".to_string(), code]),
        &["recover", "recover"],
    );
    assert!(
        !r.ok(),
        "a key was planted where no store exists:\n{}",
        r.text
    );
    let planted = std::fs::read_dir(b.0.join("keys"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(planted, 0, "a key file was written");
}

#[test]
fn a_plaintext_or_empty_store_never_makes_a_recovery_code_good() {
    let a = Home::new();
    let code = recovery_code_of(&a);
    for make in ["empty", "plaintext"] {
        let b = Home::new();
        let db = b.0.join("rooms.db");
        if make == "empty" {
            std::fs::write(&db, b"").unwrap();
        } else {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("create table t(x); insert into t values (1);")
                .unwrap();
        }
        let r = run_on_pty(
            klatch_in(&b, &["recover".to_string(), code.clone()]),
            &["recover", "recover"],
        );
        assert!(!r.ok(), "{make}: a key was planted:\n{}", r.text);
        let planted = std::fs::read_dir(b.0.join("keys"))
            .map(|d| d.count())
            .unwrap_or(0);
        assert_eq!(planted, 0, "{make}: a key file was written");
    }
}

#[test]
fn the_prompts_name_the_store_and_the_backup_file() {
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(admin_cmd(&h, &["lease-cap", "100"]), &[WRONG]);
    assert!(r.text.contains("rooms.db"), "the store path: {}", r.text);
    let backup = h.0.join("backup.db");
    assert!(h.admin(&["backup", backup.to_str().unwrap()], &[]).ok());
    for f in ["rooms.db", "rooms.db-wal", "rooms.db-shm"] {
        let _ = std::fs::remove_file(h.0.join(f));
    }
    let r = run_on_pty(
        klatch_in(
            &h,
            &["restore".to_string(), backup.to_str().unwrap().to_string()],
        ),
        &[WRONG, "restore"],
    );
    assert!(
        r.text.contains(backup.to_str().unwrap()),
        "the backup path must be in the prompt: {}",
        r.text
    );
}

#[test]
fn the_confirmation_says_what_is_being_confirmed() {
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(admin_cmd(&h, &["close", "lobby"]), &[PASS, "nope"]);
    assert_has(
        &r.text,
        &["to confirm", "type lobby to confirm `admin close lobby"],
    );
}

#[test]
fn the_passphrase_prompt_names_the_command_and_a_shell_names_every_command() {
    let h = Home::new();
    h.set_passphrase();
    let r = run_on_pty(admin_cmd(&h, &["lease-cap", "100"]), &[WRONG]);
    assert_has(&r.text, &["admin lease-cap 100"]);
    let w = h.launcher_command(&["worker", "revoke", "w-1"]);
    let r = run_on_pty(w, &[WRONG]);
    assert_has(&r.text, &["worker revoke w-1"]);
    let r = run_on_pty(admin_cmd(&h, &["shell"]), &[WRONG]);
    assert_has(&r.text, &["ALL admin and launcher commands"]);
    // a recovery code given as an argument is never echoed into the prompt
    let r = run_on_pty(
        admin_cmd(&h, &["recover", "KLATCH-SECRET-CODE"]),
        &[WRONG, WRONG],
    );
    assert_lacks(&r.text, &["SECRET-CODE"]);
}
