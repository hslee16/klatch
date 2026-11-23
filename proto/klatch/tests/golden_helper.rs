//! The golden helper itself (#1480): the diff, the scrub of volatile text and the refusal of a secret. The goldens
//! of the security code are in `golden_*.rs`.
mod common;

use common::golden::{Scrub, check, diff, forbid};

#[test]
fn the_diff_marks_changed_added_and_removed_lines() {
    assert_eq!(diff("a\nb\nc\n", "a\nb\nc\n"), "");
    assert_eq!(diff("a\nb\n", "a\nx\n"), "-   2 b\n+   2 x\n");
    assert_eq!(diff("a\n", "a\nb\n"), "+   2 b\n");
    assert_eq!(diff("a\nb\n", "b\n"), "-   1 a\n");
    assert_eq!(diff("a\n", "a"), "(the final newline differs)\n");
}

#[test]
fn volatile_tokens_are_replaced_and_the_rest_is_kept() {
    let s = Scrub::new();
    assert_eq!(
        s.apply("failures=2 terminal=ttys003/s4121\n"),
        "failures=2 terminal=<TTY>\n"
    );
    assert_eq!(
        s.apply("child, pid 4121, below\n"),
        "child, pid <PID>, below\n"
    );
    assert_eq!(s.apply("a\r\nb"), "a\nb");
    let p = std::path::Path::new("/tmp/klatch-x-1");
    assert_eq!(
        Scrub::new()
            .path(p, "<HOME>")
            .apply("in /tmp/klatch-x-1/keys\n"),
        "in <HOME>/keys\n"
    );
}

#[test]
fn a_secret_in_a_text_is_refused() {
    assert!(std::panic::catch_unwind(|| forbid("x hunter2 y", &["hunter2"])).is_err());
    forbid("x y", &["hunter2"]);
}

#[test]
fn a_changed_golden_fails_with_a_readable_diff_and_a_missing_one_says_how_to_write_it() {
    if std::env::var("KLATCH_WRITE_GOLDEN").is_ok() {
        return;
    }
    let msg = |r: Result<(), Box<dyn std::any::Any + Send>>| {
        let e = r.unwrap_err();
        e.downcast_ref::<String>().cloned().unwrap_or_default()
    };
    let m = msg(std::panic::catch_unwind(|| {
        check("helper_sample", "line one\nline TWO\nline three\n", &[]);
    }));
    assert!(
        m.contains("-   2 line two") && m.contains("+   2 line TWO"),
        "{m}"
    );
    let m = msg(std::panic::catch_unwind(|| {
        check("helper_no_such_golden", "x\n", &[])
    }));
    assert!(m.contains("KLATCH_WRITE_GOLDEN=1"), "{m}");
}

#[test]
fn the_wait_of_a_lockout_is_scrubbed_in_text_and_in_audit_rows() {
    let s = Scrub::new();
    assert_eq!(
        s.apply("too many wrong passphrases; try again in 28 seconds\n"),
        "too many wrong passphrases; try again in <N> seconds\n"
    );
    assert_eq!(
        s.apply("proof-locked wait 28s terminal=ttys001/s9\n"),
        "proof-locked wait <N>s terminal=<TTY>\n"
    );
    assert_eq!(s.apply("wait for it\n"), "wait for it\n");
}

#[test]
fn the_age_of_a_listing_line_is_scrubbed() {
    let s = Scrub::new();
    assert_eq!(s.apply("12s ago\tpause\t*\t\n"), "<AGE>s ago\tpause\t*\t\n");
    assert_eq!(s.apply("12 s ago\n"), "12 s ago\n");
}

#[test]
fn the_times_of_a_worker_and_a_run_listing_are_scrubbed() {
    let s = Scrub::new();
    assert_eq!(
        s.apply("w-1\tlive\texpires in 3599s\tcap in 86399s\n"),
        "w-1\tlive\texpires in <N>\tcap in <N>\n"
    );
    assert_eq!(
        s.apply("r1\t0 room(s)\topened 1s ago\n"),
        "r1\t0 room(s)\topened <N>s ago\n"
    );
}
