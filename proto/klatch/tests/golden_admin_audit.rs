//! Golden audit rows of the admin acts (#1480): every act of `klatch admin` run once with the right passphrase in a
//! throwaway home, then the audit read back, oldest first. A row is the age (scrubbed), the act, the target and the
//! detail: act names, outcome words and counts only. The test refuses to keep a text that holds a typed secret, and
//! the output of an act that prints a secret (the recovery code, a rotated key, an export) is not kept at all.
//! Regenerate on purpose only: `KLATCH_WRITE_GOLDEN=1`.
mod common;

use common::admin::*;
use common::golden::*;
use common::*;

/// The acts that print a recovery code, a key, stored text or a date: their terminal text is not kept, only their rows.
const SECRET_OUTPUT: [&str; 5] = [
    "recovery",
    "rotate-key",
    "rotate-label-key",
    "export",
    "run-archive",
];

/// `(arguments, what is typed after the passphrase)`, in the order they are run.
fn acts(h: &Home) -> Vec<(Vec<String>, Vec<String>)> {
    let f = |n: &str| h.0.join(n).to_string_lossy().into_owned();
    let t = |a: &[&str], typed: &[&str]| (owned(a), owned(typed));
    vec![
        t(&["lease-cap", "100"], &[]),
        t(&["lease-grace", "60"], &[]),
        t(&["room-stale", "r", "120"], &[]),
        t(&["room-ask-lease", "r", "100"], &[]),
        t(&["room-require", "r", "--off"], &[]),
        t(&["rate-limit", "30", "60"], &[]),
        t(&["rate-limit", "--off"], &[]),
        t(&["room-creation", "granted"], &[]),
        t(&["grant-rooms", "m"], &[]),
        t(&["librarian", "m"], &[]),
        t(&["reviewer", "m"], &[]),
        t(&["facilitator", "m"], &[]),
        t(&["integrator", "m"], &[]),
        t(&["admin-agent", "m"], &[]),
        t(&["grantor", "r", "m"], &[]),
        t(&["label", "r", "private"], &[]),
        t(&["clear", "m", "internal"], &[]),
        t(&["room-clear", "r", "m", "internal"], &[]),
        t(&["cap", "internal"], &[]),
        t(&["canary", "r"], &[]),
        t(&["canary-retire", "r"], &[]),
        t(&["scrub-add", "n", "quokka"], &[]),
        t(&["scrub-list"], &[]),
        t(&["scrub-remove", "n"], &[]),
        t(&["seal-label", "private"], &[]),
        t(&["unlock-window", "120"], &[]),
        t(&["unlock-window", "--off"], &[]),
        t(&["retention", "r", "30"], &[]),
        t(&["pause"], &["pause"]),
        t(&["resume"], &[]),
        t(&["close", "r"], &["r"]),
        t(&["reopen", "r"], &[]),
        t(&["remove", "m", "--room", "r"], &["m"]),
        t(&["allow", "m", "--room", "r"], &[]),
        t(&["backup", &f("backup.db")], &[]),
        t(&["archive", "--all-rooms"], &[]),
        t(&["archives"], &[]),
        t(&["stats"], &[]),
        t(&["refusals"], &[]),
        t(&["review-queue"], &[]),
        t(&["export", "r"], &["r"]),
        t(&["recovery"], &["recovery"]),
        t(&["rotate-label-key", "private"], &["private"]),
        t(&["rotate-key"], &["rotate"]),
        t(&["chain", "a,b"], &[]),
    ]
}

#[test]
fn every_act_writes_a_row_of_names_outcomes_and_counts_and_never_a_secret() {
    let h = Home::new();
    h.set_passphrase();
    assert!(h.run("m", &["join", "r"]).status.success());
    let mut s = Sheet::new(Scrub::new().path(&h.0, "<HOME>").binary());
    for (args, typed) in acts(&h) {
        let typed: Vec<&str> = typed.iter().map(String::as_str).collect();
        let mut full: Vec<&str> = args.iter().map(String::as_str).collect();
        let r = h.admin(&full, &typed);
        full.insert(0, "admin");
        let label = format!(
            "$ klatch {}",
            full.join(" ").replace(h.0.to_str().unwrap(), "<HOME>")
        );
        if SECRET_OUTPUT.contains(&args[0].as_str()) {
            s.block(&format!("{label} (output not kept)"), r.ok(), "");
        } else {
            s.block(&label, r.ok(), &r.text);
        }
    }
    let audit = h.admin(&["audit", "--limit", "500"], &[]);
    assert!(audit.ok(), "{}", audit.text);
    let mut rows: Vec<&str> = audit.text.lines().collect();
    rows.reverse();
    s.note("the audit, oldest first", &rows.join("\n"));
    s.check("audit_rows_of_every_act", &[PASS, WRONG, NEW]);
}
