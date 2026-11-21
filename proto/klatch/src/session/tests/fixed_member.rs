//! #107: a session whose member is fixed by its transport (an ssh forced command) refuses any request that tries to
//! say who is calling. Other sessions keep the old rule: such a field has no effect (PROTOCOL.md).

use super::*;

fn run_fixed(g: &Gateway<SqliteStore>, who: &Principal, lines: &[Value]) -> Vec<ServerFrame> {
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut out = Vec::new();
    let config = SessionConfig::default();
    Session::new(g, who, &config)
        .fixed_member()
        .serve(Cursor::new(input), &mut out)
        .unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap())
        .collect()
}

fn refused_rule(frames: &[ServerFrame], id: &str) -> Option<String> {
    frames.iter().find_map(|f| match f {
        ServerFrame::Refusal {
            request: Some(r),
            rule,
            ..
        } if r == id => Some(rule.clone()),
        _ => None,
    })
}

#[test]
fn a_request_naming_a_member_or_a_sender_is_refused_and_nothing_is_posted() {
    let g = gateway();
    let frames = run_fixed(
        &g,
        &alice(),
        &[
            req("m", "post", json!({"room":"r","body":"x","member":"bob"})),
            req("s", "post", json!({"room":"r","body":"x","sender":"bob"})),
            req("f", "post", json!({"room":"r","body":"x","from":"bob"})),
            req("a", "post", json!({"room":"r","body":"x","as":"bob"})),
            req("i", "post", json!({"room":"r","body":"x","Identity":"bob"})),
            req(
                "t",
                "post",
                json!({"room":"r","body":"x","attribution":"verified"}),
            ),
        ],
    );
    for id in ["m", "s", "f", "a", "i", "t"] {
        assert_eq!(
            refused_rule(&frames, id).as_deref(),
            Some("member-fixed"),
            "{id}"
        );
        let (ok, _, err) = response(&frames, id);
        assert!(!ok);
        assert_eq!(err.unwrap().0, "refused");
    }
    // nothing reached the store: the room was never made
    g.join(&bob(), "r").unwrap();
    assert!(g.read(&bob(), "r", true).unwrap().is_empty());
}

#[test]
fn a_plain_request_still_works_and_posts_as_the_fixed_member() {
    let g = gateway();
    let frames = run_fixed(
        &g,
        &alice(),
        &[req("p", "post", json!({"room":"r","body":"hello"}))],
    );
    assert!(response(&frames, "p").0);
    g.join(&bob(), "r").unwrap();
    let read = g.read(&bob(), "r", true).unwrap();
    assert_eq!(read[0].sender, "alice");
}

#[test]
fn a_member_param_that_is_the_target_of_the_method_is_not_a_claim_of_identity() {
    let g = gateway();
    g.join(&alice(), "r").unwrap();
    g.join(&alice(), "via").unwrap();
    // `invite` names the member it invites: a target, not a caller
    let frames = run_fixed(
        &g,
        &alice(),
        &[req(
            "inv",
            "invite",
            json!({"member":"bob","room":"r","via":"via"}),
        )],
    );
    assert_eq!(refused_rule(&frames, "inv"), None);
}

#[test]
fn without_the_fixed_member_a_member_field_still_has_no_effect() {
    let g = gateway();
    let frames = run(
        &g,
        &alice(),
        &[req(
            "p",
            "post",
            json!({"room":"r","body":"x","member":"bob"}),
        )],
    );
    assert_eq!(refused_rule(&frames, "p"), None);
}

#[test]
fn a_member_fixed_refusal_is_kept_in_the_refusals_without_the_claimed_name() {
    let g = gateway();
    let _ = run_fixed(
        &g,
        &alice(),
        &[req(
            "m",
            "post",
            json!({"room":"r","body":"x","member":"mallory-the-forger"}),
        )],
    );
    let kept = g.my_refusals(&alice(), 10).unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(
        (kept[0].member.as_str(), kept[0].rule.as_str()),
        ("alice", "member-fixed")
    );
    let shown = format!("{kept:?}");
    assert!(!shown.contains("mallory"), "{shown}");
}

#[test]
fn five_member_fixed_refusals_trip_the_breaker_for_that_member_only() {
    let g = gateway();
    let forged: Vec<Value> = (0..5)
        .map(|i| {
            req(
                &format!("f{i}"),
                "post",
                json!({"room":"r","body":"x","sender":"bob"}),
            )
        })
        .collect();
    let _ = run_fixed(&g, &alice(), &forged);
    let e = g.post(&alice(), "r", "innocent").unwrap_err();
    assert!(
        matches!(&e, crate::domain::Error::Refused { rule, .. } if rule == "breaker"),
        "{e:?}"
    );
    g.post(&bob(), "r", "others are unaffected").unwrap();
}
