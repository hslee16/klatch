//! What klatch tells an agent that has no name or an empty read (tickets #495 and #500). The causes found: a
//! member name is one cursor, so a second process using the same name (a subagent that resolved to its parent
//! under --discover) finds the room already read; and a missing name said only how to type one.
mod common;

use common::*;

fn stderr(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn a_missing_name_error_says_how_to_get_one_from_the_session() {
    let h = Home::new();
    let o = h
        .launcher_command(&["join", "r"])
        .detached()
        .output()
        .unwrap();
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(
        err.contains("KLATCH_NAME") && err.contains("--discover"),
        "the error must name both ways to be someone: {err}"
    );
}

#[test]
fn an_empty_read_says_why_it_is_empty_and_keeps_stdout_clean() {
    let h = Home::new();
    h.ok("a", &["join", "r"]);
    h.ok("b", &["join", "r"]);
    h.ok("a", &["post", "r", "hello"]);
    assert!(h.ok("b", &["read", "r"]).contains("hello"));
    let o = h.run("b", &["read", "r"]);
    assert!(o.status.success());
    assert!(o.stdout.is_empty(), "stdout must stay as scripts match it");
    let err = stderr(&o);
    assert!(
        err.contains("no unread") && err.contains("same member name"),
        "an empty read must explain itself: {err}"
    );
}

#[test]
fn two_names_each_see_the_other_but_one_name_shares_one_cursor() {
    let h = Home::new();
    h.ok("a", &["join", "r"]);
    h.ok("b", &["join", "r"]);
    h.ok("a", &["post", "r", "from a"]);
    h.ok("b", &["post", "r", "from b"]);
    assert!(h.ok("b", &["read", "r"]).contains("from a"));
    assert!(h.ok("a", &["read", "r"]).contains("from b"));
    h.ok("a", &["post", "r", "second"]);
    // a second process acting as "b" finds the room already read by the first
    assert!(h.ok("b", &["read", "r"]).contains("second"));
    assert!(!h.ok("b", &["read", "r"]).contains("second"));
}

#[test]
fn a_declared_name_gets_no_join_hint() {
    let h = Home::new();
    let o = h.run("a", &["join", "r"]);
    assert!(stderr(&o).is_empty(), "{}", stderr(&o));
}
