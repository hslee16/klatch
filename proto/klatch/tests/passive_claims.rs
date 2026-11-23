//! Passive claims (#1401): the opt-in PostToolUse step claims the repo-relative path of a file a member edits or
//! writes, renews it on every further edit, releases it at Stop and SessionEnd, and on a refusal prints one line and
//! never blocks. Fixture hook inputs run through the real binary in throwaway homes; nothing touches a real settings
//! file or store.
mod common;

use common::*;

/// A project directory with a `.git` entry, so paths are taken relative to it.
fn project(h: &Home) -> String {
    let p = h.0.join("proj");
    std::fs::create_dir_all(p.join(".git")).unwrap();
    p.to_str().unwrap().to_string()
}

/// One PostToolUse input for `who`; `flag` is whether the hook command carries the consent, `extra` is more JSON
/// fields of the input (such as `"agent_id":"sub"`).
fn tool(
    h: &Home,
    who: &str,
    proj: &str,
    body: &str,
    (flag, extra): (bool, &str),
) -> std::process::Output {
    let payload = format!(
        r#"{{"session_id":"s-{who}","hook_event_name":"PostToolUse","cwd":"{proj}",{extra}{body}}}"#
    );
    let mut args = vec!["hook", "--event", "PostToolUse"];
    if flag {
        args.push("--passive-claims");
    }
    let o = hook_with_payload(h, Some(who), &payload, &args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o
}

fn edit_with(
    h: &Home,
    who: &str,
    proj: &str,
    file: &str,
    flag: (bool, &str),
) -> std::process::Output {
    tool(
        h,
        who,
        proj,
        &format!(
            r#""tool_name":"Edit","tool_input":{{"file_path":"{file}","old_string":"a","new_string":"b"}}"#
        ),
        flag,
    )
}

/// An edit of `proj/<file>` under the consent.
fn edit(h: &Home, who: &str, proj: &str, file: &str) -> std::process::Output {
    edit_with(h, who, proj, &format!("{proj}/{file}"), (true, ""))
}

fn stdout(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stop(h: &Home, who: &str, proj: &str) {
    let payload = format!(r#"{{"session_id":"s-{who}","hook_event_name":"Stop","cwd":"{proj}"}}"#);
    let o = hook_with_payload(
        h,
        Some(who),
        &payload,
        &["hook", "--event", "Stop", "--passive-claims"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

fn end(h: &Home, who: &str, proj: &str, flag: bool) {
    let payload = format!(
        r#"{{"session_id":"s-{who}","hook_event_name":"SessionEnd","cwd":"{proj}","reason":"other"}}"#
    );
    let mut args = vec!["hook", "--event", "SessionEnd"];
    if flag {
        args.push("--passive-claims");
    }
    let o = hook_with_payload(h, Some(who), &payload, &args);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// Whether `holder` holds `path` in the lobby, as another member reads it.
fn holds(h: &Home, holder: &str, path: &str) -> bool {
    h.ok("observer", &["claims", "lobby"])
        .lines()
        .any(|l| l.contains(path) && l.contains(holder))
}

fn world() -> (Home, String) {
    let h = Home::new();
    let p = project(&h);
    for who in ["alice", "bob", "observer"] {
        h.ok(who, &["join", "lobby"]);
    }
    (h, p)
}

#[test]
fn an_edit_claims_the_repo_relative_path_for_the_member_and_prints_nothing() {
    let (h, p) = world();
    let o = edit(&h, "alice", &p, "src/a.rs");
    assert_eq!(stdout(&o), "", "zero bytes when nothing is refused");
    assert!(
        holds(&h, "alice", "src/a.rs"),
        "{}",
        h.ok("bob", &["claims", "lobby"])
    );
    let w = tool(
        &h,
        "alice",
        &p,
        &format!(
            r#""tool_name":"Write","tool_input":{{"file_path":"{p}/src/b.rs","content":"x"}}"#
        ),
        (true, ""),
    );
    assert_eq!(stdout(&w), "");
    assert!(holds(&h, "alice", "src/b.rs"));
    // the claim says why it exists, as data
    let listed = h.ok("bob", &["claims", "lobby"]);
    assert!(listed.contains("passive"), "{listed}");
}

#[test]
fn a_second_edit_of_the_same_path_renews_the_one_claim() {
    let (h, p) = world();
    edit(&h, "alice", &p, "src/a.rs");
    let o = edit(&h, "alice", &p, "src/a.rs");
    assert_eq!(stdout(&o), "");
    let listed = h.ok("bob", &["claims", "lobby"]);
    assert_eq!(listed.matches("src/a.rs").count(), 1, "{listed}");
}

#[test]
fn a_refusal_prints_one_line_with_the_holder_the_age_the_ask_and_the_data_notice_and_never_blocks()
{
    let (h, p) = world();
    edit(&h, "alice", &p, "src/a.rs");
    let o = edit(&h, "bob", &p, "src/a.rs");
    assert!(o.status.success(), "a refusal never fails the tool call");
    let v: serde_json::Value = serde_json::from_str(stdout(&o).trim()).expect("one JSON object");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert_eq!(ctx.lines().count(), 1, "one line: {ctx}");
    assert!(ctx.contains("alice") && ctx.contains("src/a.rs"), "{ctx}");
    assert!(
        ctx.contains("klatch ask lobby"),
        "the exact ask to send: {ctx}"
    );
    assert!(ctx.contains("overlap:"), "{ctx}");
    assert!(ctx.contains("data"), "the data notice: {ctx}");
    assert!(ctx.contains("for "), "how long it has been held: {ctx}");
    // bob did not take it, and a path nobody holds is claimed and silent
    assert!(!holds(&h, "bob", "src/a.rs"));
    assert_eq!(stdout(&edit(&h, "bob", &p, "src/other.rs")), "");
    assert!(holds(&h, "bob", "src/other.rs"));
}

#[test]
fn the_never_claim_list_and_unusable_paths_are_ignored() {
    let (h, p) = world();
    for file in [
        "DESIGN.md",
        "CHANGELOG.md",
        "RESEARCH-NOTES.md",
        "docs/CHANGELOG.md",
    ] {
        assert_eq!(stdout(&edit(&h, "alice", &p, file)), "", "{file}");
    }
    for abs in ["/etc/hosts", "/tmp/elsewhere/x.rs"] {
        edit_with(&h, "alice", &p, abs, (true, ""));
    }
    edit_with(&h, "alice", &p, &format!("{p}/../outside.rs"), (true, ""));
    edit_with(&h, "alice", &p, &format!("{p}/src/a\\nb.rs"), (true, ""));
    edit_with(
        &h,
        "alice",
        &p,
        &format!("{p}/src/a\\u0007b.rs"),
        (true, ""),
    );
    let listed = h.ok("bob", &["claims", "lobby"]);
    assert!(!listed.contains("alice"), "nothing was claimed: {listed}");
    // a claim by someone else on a never-claim file does not refuse the edit either
    h.ok("bob", &["claim", "lobby", "CHANGELOG.md"]);
    let o = stdout(&edit(&h, "alice", &p, "CHANGELOG.md"));
    assert!(!o.contains("klatch ask"), "no refusal line: {o}");
}

#[test]
fn a_stop_and_a_session_end_release_the_passive_claims_and_never_the_explicit_ones() {
    let (h, p) = world();
    h.ok("alice", &["claim", "lobby", "src/explicit.rs"]);
    edit(&h, "alice", &p, "src/a.rs");
    edit(&h, "alice", &p, "src/b.rs");
    assert!(holds(&h, "alice", "src/a.rs"));
    stop(&h, "alice", &p);
    assert!(!holds(&h, "alice", "src/a.rs") && !holds(&h, "alice", "src/b.rs"));
    assert!(
        holds(&h, "alice", "src/explicit.rs"),
        "an explicit claim stays"
    );
    edit(&h, "alice", &p, "src/c.rs");
    assert!(holds(&h, "alice", "src/c.rs"));
    end(&h, "alice", &p, true);
    assert!(!holds(&h, "alice", "src/c.rs"));
}

#[test]
fn without_the_consent_nothing_is_claimed_and_nothing_is_released() {
    let (h, p) = world();
    let o = edit_with(&h, "alice", &p, &format!("{p}/src/a.rs"), (false, ""));
    assert_eq!(stdout(&o), "");
    assert!(!holds(&h, "alice", "src/a.rs"));
    // a Stop without the flag leaves a passive claim in place (a hook command without the flag does nothing)
    edit(&h, "alice", &p, "src/b.rs");
    let payload = format!(r#"{{"session_id":"s-alice","hook_event_name":"Stop","cwd":"{p}"}}"#);
    hook_with_payload(&h, Some("alice"), &payload, &["hook", "--event", "Stop"]);
    assert!(holds(&h, "alice", "src/b.rs"));
}

#[test]
fn a_tool_call_made_inside_a_subagent_does_not_claim() {
    let (h, p) = world();
    let o = edit_with(
        &h,
        "alice",
        &p,
        &format!("{p}/src/a.rs"),
        (true, r#""agent_id":"sub-1","agent_type":"x","#),
    );
    assert_eq!(stdout(&o), "");
    assert!(!holds(&h, "alice", "src/a.rs"));
}

#[test]
fn an_explicit_claim_that_covers_the_path_is_not_claimed_twice() {
    let (h, p) = world();
    h.ok("alice", &["claim", "lobby", "src"]);
    edit(&h, "alice", &p, "src/a.rs");
    let listed = h.ok("bob", &["claims", "lobby"]);
    assert!(!listed.contains("src/a.rs"), "{listed}");
    assert!(listed.contains("src"), "{listed}");
    // a read tool never claims
    let r = tool(
        &h,
        "alice",
        &p,
        &format!(r#""tool_name":"Read","tool_input":{{"file_path":"{p}/lib/z.rs"}}"#),
        (true, ""),
    );
    assert_eq!(stdout(&r), "");
    assert!(!holds(&h, "alice", "lib/z.rs"));
}

#[test]
fn a_member_in_no_room_claims_nothing_and_the_tool_call_still_succeeds() {
    let h = Home::new();
    let p = project(&h);
    let o = edit(&h, "loner", &p, "src/a.rs");
    assert_eq!(stdout(&o), "");
}

#[test]
fn a_store_that_cannot_be_read_never_fails_the_tool_call() {
    let (h, p) = world();
    std::fs::write(h.0.join("rooms.db"), b"this is not a database at all").unwrap();
    let o = edit(&h, "alice", &p, "src/a.rs");
    assert!(o.status.success());
}

/// The offline measure behind the ticket's metric: of N scripted edits, how many were already covered by a claim at
/// the moment of the edit, and how many edits of a path another member holds were met by a warning.
#[test]
fn coverage_of_scripted_edits_with_and_without_passive_claims() {
    // alice edits four files three times each (12 edits); bob then edits two of alice's files (4 edits)
    let script: Vec<&str> = ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"]
        .iter()
        .flat_map(|f| [*f, *f, *f])
        .collect();
    let bob_script = ["src/a.rs", "src/a.rs", "src/b.rs", "src/b.rs"];
    let run = |consent: bool| -> (usize, usize) {
        let (h, p) = world();
        let mut covered = 0;
        for file in &script {
            if holds(&h, "alice", file) {
                covered += 1;
            }
            edit_with(&h, "alice", &p, &format!("{p}/{file}"), (consent, ""));
        }
        let warned = bob_script
            .iter()
            .filter(|f| {
                !stdout(&edit_with(
                    &h,
                    "bob",
                    &p,
                    &format!("{p}/{f}"),
                    (consent, ""),
                ))
                .is_empty()
            })
            .count();
        (covered, warned)
    };
    let (without_covered, without_warned) = run(false);
    let (with_covered, with_warned) = run(true);
    assert_eq!((without_covered, without_warned), (0, 0));
    assert_eq!(
        (with_covered, with_warned),
        (8, 4),
        "the first edit of each file is claimed after the edit (PostToolUse), the next two are covered; every one of bob's edits of a held path meets the warning"
    );
    println!(
        "passive claims fixture: edits covered at edit time {with_covered}/{} with, {without_covered}/{} without; overlapping edits warned {with_warned}/{} with, {without_warned}/{} without",
        script.len(),
        script.len(),
        bob_script.len(),
        bob_script.len()
    );
}
