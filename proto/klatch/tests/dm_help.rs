//! `klatch help dm` teaches the direct message: what it does, how to wait, who can read it, example flows (#1185).
mod common;

use common::*;

fn help(h: &Home, args: &[&str]) -> String {
    h.ok("alice", args)
}

#[test]
fn help_dm_and_dm_help_are_the_same_text() {
    let h = Home::new();
    let a = help(&h, &["help", "dm"]);
    let b = help(&h, &["dm", "--help"]);
    assert_eq!(a, b);
}

#[test]
fn dm_help_names_wait_privacy_grants_and_the_room() {
    let h = Home::new();
    let out = help(&h, &["help", "dm"]);
    for must in [
        "klatch wait dm:",
        "klatch wait dm.",
        "dm.<a>.<b>",
        "The two members and the human who runs klatch can read it",
        "No grant is needed",
        "both members are joined",
        "digest",
    ] {
        assert!(out.contains(must), "help dm lacks {must:?}:\n{out}");
    }
}

#[test]
fn the_top_level_help_points_to_help_dm() {
    let h = Home::new();
    let out = help(&h, &["--help"]);
    assert!(out.contains("klatch help dm"), "got: {out}");
}

/// End the word being built, if there is one (an empty pair of quotes is one).
fn end_word(out: &mut Vec<String>, cur: &mut String, any: &mut bool) {
    if *any || !cur.is_empty() {
        out.push(std::mem::take(cur));
        *any = false;
    }
}

/// Split a help example into words: whitespace separates, double quotes group (an empty pair of quotes is a word).
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut quoted, mut any) = (String::new(), false, false);
    for c in line.chars() {
        let separator = c.is_whitespace() && !quoted;
        if c == '"' {
            quoted = !quoted;
            any = true;
        } else if !separator {
            cur.push(c);
        } else {
            end_word(&mut out, &mut cur, &mut any);
        }
    }
    end_word(&mut out, &mut cur, &mut any);
    out
}

/// The drift test: every example line (`member: klatch ...`) of the help runs against the real CLI, in order.
#[test]
fn every_example_in_the_help_runs() {
    let h = Home::new();
    let out = help(&h, &["help", "dm"]);
    let mut ran = 0;
    for line in out.lines() {
        let line = line.trim();
        let Some((who, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(cmd) = rest.trim_start().strip_prefix("klatch ") else {
            continue;
        };
        if who.contains(' ') || who == "Usage" {
            continue;
        }
        let mut args = words(cmd);
        if args[0] == "wait" {
            args.extend(["--timeout".to_string(), "0".to_string()]);
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let o = h.run(who, &refs);
        assert!(
            o.status.success(),
            "example {line:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        ran += 1;
    }
    assert!(ran >= 6, "the help should carry two flows, ran {ran}");
}
