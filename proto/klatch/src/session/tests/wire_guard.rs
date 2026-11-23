use super::*;

struct MovingClock(std::rc::Rc<std::cell::Cell<i64>>);
impl crate::gateway::Clock for MovingClock {
    fn now(&self) -> i64 {
        self.0.get()
    }
}

#[test]
fn a_session_for_a_verified_member_whose_credential_has_ended_is_refused_on_every_request() {
    // the process proved itself while the credential was live and kept running (#161): the session checks again
    let now = std::rc::Rc::new(std::cell::Cell::new(1_000));
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        MovingClock(now.clone()),
        AllowAll,
    );
    let spec = crate::gateway::CredentialSpec {
        member: "w-1",
        clearance: "internal",
        ttl: 10,
        max_life: None,
        parent: None,
    };
    g.admin_issue_credential(&alice(), &spec).unwrap();
    let who = Principal::verified("w-1");
    let live = run(
        &g,
        &who,
        &[req("a", "post", json!({"room":"r","body":"x"}))],
    );
    assert!(response(&live, "a").0, "a live credential was refused");
    now.set(1_010);
    let f = run(
        &g,
        &who,
        &[
            req("a", "post", json!({"room":"r","body":"y"})),
            req("b", "who", json!({"room":"r"})),
            req("c", "claims", json!({"room":"r"})),
        ],
    );
    for id in ["a", "b", "c"] {
        let (ok, _, err) = response(&f, id);
        assert!(!ok, "{id} was answered after the credential expired");
        assert_eq!(err.unwrap().0, "refused");
    }
}

#[test]
fn read_carries_advisory_true_only_for_a_flagged_sender_and_omits_the_key_otherwise() {
    use crate::harness::{ClaudeCode, Codex, HookTrust, Support};
    let g = gateway();
    let human = Principal::unattributed("admin");
    g.admin_set_room_require(&human, "r", Some("flag:pre-action-enforcement"))
        .unwrap();
    g.join_with_harness(
        &alice(),
        "r",
        &ClaudeCode::new(Support::Unknown(String::new())),
    )
    .unwrap();
    g.join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Unknown))
        .unwrap();
    g.post(&bob(), "r", "flagged hello").unwrap();
    g.post(&alice(), "r", "clean hello").unwrap();
    let carol = Principal::unattributed("carol");
    let f = run(
        &g,
        &carol,
        &[req("r", "read", json!({"room":"r","peek":true}))],
    );
    let (ok, result, _) = response(&f, "r");
    assert!(ok);
    let ms = &result.unwrap()["messages"];
    assert_eq!(ms[0]["sender"], "bob");
    assert_eq!(ms[0]["advisory"], true);
    assert_eq!(ms[1]["sender"], "alice");
    assert!(
        ms[1].get("advisory").is_none(),
        "an unflagged message looks as it always did"
    );
}

fn flagged_wire_room() -> Gateway<SqliteStore> {
    use crate::harness::{ClaudeCode, Codex, HookTrust, Support};
    let g = gateway();
    let human = Principal::unattributed("admin");
    g.admin_set_room_require(&human, "r", Some("flag:pre-action-enforcement"))
        .unwrap();
    g.join_with_harness(
        &alice(),
        "r",
        &ClaudeCode::new(Support::Unknown(String::new())),
    )
    .unwrap();
    g.join_with_harness(&bob(), "r", &Codex::paper(HookTrust::Unknown))
        .unwrap();
    g.post(&bob(), "r", "flagged hello").unwrap();
    g.post(&alice(), "r", "clean hello").unwrap();
    g
}

#[test]
fn history_carries_advisory_true_only_for_a_flagged_sender() {
    let g = flagged_wire_room();
    let carol = Principal::unattributed("carol");
    let f = run(&g, &carol, &[req("h", "history", json!({"room":"r"}))]);
    let (ok, result, _) = response(&f, "h");
    assert!(ok);
    let ms = &result.unwrap()["messages"];
    assert_eq!(ms[0]["sender"], "bob");
    assert_eq!(ms[0]["advisory"], true);
    assert!(ms[1].get("advisory").is_none());
}

#[test]
fn the_live_stream_frame_carries_advisory_only_for_a_flagged_sender() {
    let g = flagged_wire_room();
    let f = run(
        &g,
        &Principal::unattributed("carol"),
        &[req("s", "subscribe", json!({"room":"r","from_seq":0}))],
    );
    let flags: Vec<(String, bool)> = f
        .iter()
        .filter_map(|x| match x {
            ServerFrame::Message {
                sender, advisory, ..
            } => Some((sender.clone(), *advisory)),
            _ => None,
        })
        .collect();
    assert_eq!(
        flags,
        vec![("bob".to_string(), true), ("alice".to_string(), false)]
    );
    // on the wire the key exists only when true
    let lines: Vec<String> = f
        .iter()
        .filter(|x| matches!(x, ServerFrame::Message { .. }))
        .map(|x| serde_json::to_string(x).unwrap())
        .collect();
    assert!(lines[0].contains("\"advisory\":true"), "{}", lines[0]);
    assert!(!lines[1].contains("advisory"), "{}", lines[1]);
}
