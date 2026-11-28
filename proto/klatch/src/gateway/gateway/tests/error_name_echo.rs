//! Error text never repeats a caller's name raw (#1768, after #1763): a name that failed validation is echoed capped
//! and escaped, so a 100 KB name, a control, bidi or tag character or an ANSI sequence cannot steer a terminal or
//! flood a reader through an error.

use super::*;
use crate::gateway::ClaimSpec;
use crate::gateway::CredentialSpec;

/// A name that is long and carries every kind of character an error must not pass on.
fn hostile() -> String {
    // the junk comes first as well as last: a cap alone must not hide a missing escape
    format!(
        "\u{1b}[31m\u{202e}\u{e0041}\r\n{}\u{1b}]0;pwned\u{7}\u{2066}\u{e0001}\u{200b}\t\u{0}x\\u{{41}}",
        "n".repeat(100 * 1024)
    )
}

/// The sites checked so far and what each got wrong; every site is tried before the test fails, so one run shows them
/// all.
#[derive(Default)]
struct Sites(Vec<String>);

impl Sites {
    /// The error of `r` must be bounded pure ASCII without a control character.
    fn check(&mut self, r: Result<impl std::fmt::Debug>, site: &str) {
        self.check_within(r, site, 600);
    }

    fn check_within(&mut self, r: Result<impl std::fmt::Debug>, site: &str, max: usize) {
        let Err(e) = r else {
            self.0.push(format!("{site}: no error"));
            return;
        };
        let text = e.to_string();
        let shown: String = text
            .chars()
            .take(60)
            .collect::<String>()
            .escape_debug()
            .to_string();
        if !text.is_ascii() {
            self.0.push(format!(
                "{site}: not pure ASCII ({} bytes): {shown}",
                text.len()
            ));
        } else if text.chars().any(|c| c.is_control()) {
            self.0.push(format!("{site}: a control character: {shown}"));
        } else if text.len() >= max {
            self.0
                .push(format!("{site}: {} bytes: {shown}", text.len()));
        }
    }

    fn finish(self) {
        assert!(self.0.is_empty(), "\n{}", self.0.join("\n"));
    }
}

fn hostile_text() -> String {
    hostile()
}

#[test]
fn every_member_name_echo_is_bounded_and_ascii() {
    let g = open();
    let h = hostile_text();
    let (a, r) = (admin(), "r");
    let cases: Vec<(&str, Result<()>)> = vec![
        ("admin_remove_member", g.admin_remove_member(&a, r, &h)),
        ("admin_allow_member", g.admin_allow_member(&a, r, &h)),
        (
            "grant_room_clearance",
            g.grant_room_clearance(&a, r, &h, "internal"),
        ),
        (
            "admin_set_room_grantor",
            g.admin_set_room_grantor(&a, r, &h, true),
        ),
        (
            "admin_set_room_clearance",
            g.admin_set_room_clearance(&a, r, &h, None),
        ),
        (
            "admin_set_clearance",
            g.admin_set_clearance(&a, &h, "internal"),
        ),
        ("admin_set_reviewer", g.admin_set_reviewer(&a, &h, true)),
        (
            "admin_set_admin_agent",
            g.admin_set_admin_agent(&a, &h, true),
        ),
        ("admin_set_ssh_key", g.admin_set_ssh_key(&a, "k", &h, true)),
        ("admin_revoke_credential", g.admin_revoke_credential(&a, &h)),
        (
            "admin_grant_room_capability",
            g.admin_grant_room_capability(&a, &h, true),
        ),
        ("invite member", g.invite(&alice(), &h, r, r)),
        ("invite room", g.invite(&alice(), "bob", &h, r)),
        ("invite via", g.invite(&alice(), "bob", r, &h)),
    ];
    let mut sites = Sites::default();
    for (site, res) in cases {
        sites.check(res, site);
    }
    sites.check(g.could_answer(&a, r, &h, 1), "could_answer");
    sites.finish();
}

#[test]
fn the_other_name_echoes_are_bounded_and_ascii() {
    let g = open();
    let h = hostile_text();
    let a = admin();
    let mut sites = Sites::default();
    sites.check(g.admin_add_scrub_term(&a, &h, "x"), "scrub add");
    sites.check(g.admin_remove_scrub_term(&a, &h), "scrub remove");
    sites.check(g.admin_set_ssh_key(&a, &h, "bob", true), "ssh key label");
    sites.check(g.admin_run_create(&a, &h), "run create");
    sites.check(g.admin_run_end(&a, &h, None), "run end");
    sites.check(g.anchor_done(&a, &h), "anchor done");
    sites.check(
        g.claim(&alice(), "r", &ClaimSpec::new("p", 60, "why").anchored(&h)),
        "claim anchor",
    );
    sites.check(g.admin_set_cap(&a, &h), "unknown label (cap)");
    sites.check(g.check_sealable(&h), "unknown label (seal)");
    let spec = CredentialSpec {
        member: &h,
        clearance: "internal",
        ttl: 60,
        max_life: None,
        parent: None,
    };
    sites.check(g.admin_issue_credential(&a, &spec), "worker name");
    sites.finish();
}

#[test]
fn a_plain_bad_name_keeps_its_text() {
    let g = open();
    let a = admin();
    let t = |r: Result<()>| r.unwrap_err().to_string();
    assert_eq!(
        t(g.admin_remove_member(&a, "r", "bad name")),
        "bad member name: bad name"
    );
    assert_eq!(
        t(g.admin_add_scrub_term(&a, "bad name", "x")),
        "bad rule name: bad name"
    );
    assert_eq!(
        t(g.invite(&alice(), "bob", "bad room", "r")),
        "bad room name: bad room"
    );
    assert_eq!(
        t(g.admin_run_create(&a, "bad id")),
        "a run id is 1 to 32 characters of letters, digits, dash and underscore: \"bad id\""
    );
    assert_eq!(
        t(g.admin_set_ssh_key(&a, "bad label", "bob", true)),
        "bad ssh key label: \"bad label\"; use 1 to 40 letters, digits, '.', '_' or '-'"
    );
}

#[test]
fn a_label_chain_error_is_bounded_and_ascii() {
    let h = hostile_text();
    let e = crate::domain::LabelChain::new(&["a", h.as_str()]).unwrap_err();
    let mut sites = Sites::default();
    sites.check(Err::<(), _>(Error::Invalid(e)), "label chain");
    sites.finish();
    assert_eq!(
        crate::domain::LabelChain::new(&["a", "b c"]).unwrap_err(),
        "bad label name: b c"
    );
}

#[test]
fn a_claim_conflict_names_a_hostile_path_and_members_capped_and_ascii() {
    let h = hostile_text();
    let conflict = Error::Conflict {
        path: h.clone(),
        owner: h.clone(),
        secs_left: 5,
        held_secs: 4000,
        deadlock: true,
        cycle: vec![h.clone(), "bob".into()],
        via: h.clone(),
    };
    let mut sites = Sites::default();
    sites.check_within(Err::<(), _>(conflict), "claim conflict", 1200);
    sites.finish();
    let plain = Error::Conflict {
        path: "src/a".into(),
        owner: "bob".into(),
        secs_left: 5,
        held_secs: 0,
        deadlock: false,
        cycle: vec![],
        via: String::new(),
    };
    assert_eq!(
        plain.to_string(),
        "refused: src/a is claimed by bob for another 5s"
    );
}

#[test]
fn the_admin_refusals_and_the_not_removed_error_keep_their_text() {
    let g = open();
    let a = admin();
    g.join(&alice(), "r").unwrap();
    g.admin_close_room(&a, "r", true).unwrap();
    assert_eq!(
        g.post(&alice(), "r", "x").unwrap_err().to_string(),
        "refused (admin-close): room r is closed by the admin; it is read-only"
    );
    g.admin_close_room(&a, "r", false).unwrap();
    g.admin_remove_member(&a, "r", "alice").unwrap();
    assert_eq!(
        g.post(&alice(), "r", "x").unwrap_err().to_string(),
        "refused (admin-block): you were removed from room r by the admin"
    );
    assert_eq!(
        g.admin_allow_member(&a, "r", "bob")
            .unwrap_err()
            .to_string(),
        "bob is not removed from r"
    );
}
