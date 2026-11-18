//! End-to-end tests (#301): a member declares its kind with `--member-kind` or KLATCH_KIND, `who` and the board show
//! it, a bad or session kind is refused, and a member that declared nothing shows none.
mod common;

use common::*;
use std::process::{Command, Output};

fn as_kind(h: &Home, who: &str, kind: Option<(&str, &str)>, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_klatch"));
    c.env_remove("KLATCH_IDENTITY_FILE")
        .env_remove("KLATCH_KIND")
        .env("KLATCH_HOME", &h.0)
        .env("KLATCH_KEY_DIR", h.0.join("keys"))
        .env("KLATCH_NAME", who);
    match kind {
        Some(("flag", k)) => {
            c.args(["--member-kind", k]);
        }
        Some((_, k)) => {
            c.env("KLATCH_KIND", k);
        }
        None => {}
    }
    c.args(args).output().unwrap()
}

fn who_line(h: &Home, name: &str) -> String {
    h.ok("viewer", &["who", "lobby"])
        .lines()
        .find(|l| l.starts_with(name))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_kind_declared_by_flag_or_environment_shows_in_who_and_on_the_board() {
    let h = Home::new();
    assert!(
        as_kind(&h, "grok", Some(("flag", "bot")), &["post", "lobby", "hi"])
            .status
            .success()
    );
    assert!(
        as_kind(&h, "weather", Some(("env", "service")), &["join", "lobby"])
            .status
            .success()
    );
    h.ok("plain", &["post", "lobby", "hello"]);
    assert!(
        who_line(&h, "grok").contains("kind: bot"),
        "{}",
        who_line(&h, "grok")
    );
    assert!(who_line(&h, "weather").contains("kind: service"));
    assert!(!who_line(&h, "plain").contains("kind"));
    let board = h.ok("viewer", &["board", "lobby"]);
    assert!(board.contains("grok last seen"), "{board}");
    assert!(board.contains("kind: bot"), "{board}");
}

#[test]
fn a_declared_kind_is_remembered_and_off_withdraws_it() {
    let h = Home::new();
    as_kind(&h, "grok", Some(("flag", "human")), &["join", "lobby"]);
    h.ok("grok", &["post", "lobby", "no flag this time"]);
    assert!(who_line(&h, "grok").contains("kind: human"));
    let off = as_kind(&h, "grok", Some(("flag", "off")), &["post", "lobby", "x"]);
    assert!(off.status.success());
    assert!(!who_line(&h, "grok").contains("kind"));
}

#[test]
fn a_kind_outside_the_vocabulary_and_a_declared_session_are_refused_and_nothing_is_stored() {
    let h = Home::new();
    for bad in ["robot", "session", "Bot", ""] {
        let r = as_kind(&h, "grok", Some(("flag", bad)), &["post", "lobby", "hi"]);
        assert!(!r.status.success(), "{bad:?} must be refused");
        let err = String::from_utf8_lossy(&r.stderr).to_lowercase();
        assert!(err.contains("kind"), "{bad:?}: {err}");
    }
    h.ok("grok", &["post", "lobby", "hi"]);
    assert!(!who_line(&h, "grok").contains("kind"));
}
