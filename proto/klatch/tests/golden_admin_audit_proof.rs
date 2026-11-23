//! Golden audit rows of the proof itself and of the launcher commands (#1480): a wrong passphrase, the lockout, a
//! refusal for no terminal, the passphrase set, changed and reset, the recovery attempts, and the rows of `worker`,
//! `run` and `anchor` acts. Names, outcome words and counts only; the terminal label is scrubbed to `<TTY>`, the
//! wait to `<N>`, the age to `<AGE>`. Throwaway home and passphrases. Regenerate on purpose only: `KLATCH_WRITE_GOLDEN=1`.
mod common;

use common::admin::*;
use common::golden::*;
use common::pty::*;
use common::*;

const SECRETS: [&str; 3] = [PASS, WRONG, NEW];

/// The audit of the home, oldest first, read at a terminal with the passphrase `pass`.
fn audit(h: &Home, pass: &str) -> String {
    let r = run_on_pty(admin_cmd(h, &["audit", "--limit", "500"]), &[pass]);
    assert!(r.ok(), "{}", r.text);
    let mut rows: Vec<&str> = r
        .text
        .lines()
        .filter(|l| !l.starts_with("admin passphrase to approve"))
        .collect();
    rows.reverse();
    rows.join("\n")
}

#[test]
fn a_wrong_passphrase_a_lockout_and_a_refusal_leave_rows_with_counts_and_no_secret() {
    let h = Home::new();
    h.set_passphrase();
    let mut s = Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary());
    let _ = repeated_on_one_pty(admin_cmd(&h, &["pause"]), 4, &[WRONG, WRONG, WRONG, PASS]);
    let (_, _) = detached(admin_cmd(&h, &["resume"]), None);
    let _ = run_on_pty(init_cmd(&h), &[PASS, NEW, NEW]);
    s.note("the audit, oldest first", &audit(&h, NEW));
    s.check("audit_rows_of_the_proof", &SECRETS);
}

fn init_cmd(h: &Home) -> std::process::Command {
    h.launcher_command(&["admin", "init-passphrase"])
}

#[test]
fn the_reset_and_the_recovery_attempts_leave_rows_of_counts_only() {
    let h = Home::new();
    h.set_passphrase();
    let code = recovery_code_of(&h);
    let reset = h.launcher_command(&["admin", "reset-passphrase"]);
    let r = run_on_pty(reset, &[&code, NEW, NEW]);
    assert!(r.ok(), "{}", r.text);
    let backup = h.0.join("backup.db");
    assert!(run_on_pty(admin_cmd(&h, &["backup", backup.to_str().unwrap()]), &[NEW]).ok());
    for f in ["rooms.db", "rooms.db-wal", "rooms.db-shm"] {
        let _ = std::fs::remove_file(h.0.join(f));
    }
    let args = owned(&["restore", backup.to_str().unwrap()]);
    let r = run_on_pty(klatch_in(&h, &args), &["KLATCH-AAAA-AAAA", "restore"]);
    assert!(!r.ok());
    let r = run_on_pty(klatch_in(&h, &args), &[&code, "restore"]);
    assert!(r.ok(), "{}", r.text);
    let mut s = Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary());
    s.note("the audit, oldest first", &audit(&h, NEW));
    s.check(
        "audit_rows_of_reset_and_recovery",
        &[PASS, WRONG, NEW, &code],
    );
}

#[test]
fn the_launcher_commands_leave_rows_of_names_and_counts() {
    let h = Home::new();
    h.set_passphrase();
    assert!(h.run("m", &["join", "r"]).status.success());
    let mut s = Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary());
    let bundle = h.0.join("w.bundle");
    let issue = [
        "worker",
        "issue",
        "w-1",
        "--clearance",
        "internal",
        "--out",
        bundle.to_str().unwrap(),
    ];
    let cmds: Vec<Vec<&str>> = vec![
        issue.to_vec(),
        vec!["worker", "list"],
        vec!["worker", "revoke", "w-1"],
        vec!["run", "create", "r1"],
        vec!["run", "list"],
        vec!["run", "end", "r1"],
        vec!["anchor", "done", "a"],
        vec!["anchor", "reap", "--dir", "/nonexistent-klatch-dir"],
        vec!["judge", "run", "--once"],
    ];
    for args in cmds {
        let r = h.launcher_typing(&args, &[]);
        let shown = format!("$ klatch {}", args.join(" ")).replace(h.0.to_str().unwrap(), "<HOME>");
        // the issued bundle is a credential: its terminal text is not kept for `worker issue`
        let text = if args[1] == "issue" {
            String::new()
        } else {
            r.text.clone()
        };
        s.block(&shown, r.ok(), &text);
    }
    s.note("the audit, oldest first", &audit(&h, PASS));
    s.check("audit_rows_of_the_launcher_commands", &SECRETS);
}
