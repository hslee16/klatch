use super::*;

#[test]
fn post_then_read_round_trips_through_the_gateway() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    let p = g.post(&bob(), "r", "hello").unwrap();
    let got = g.read(&alice(), "r", true).unwrap();
    assert_eq!(
        (got[0].sender.as_str(), got[0].body.as_str(), got[0].seq),
        ("bob", "hello", p.seq)
    );
    assert!(g.read(&alice(), "r", true).unwrap().is_empty());
}

#[test]
fn the_injected_clock_is_the_only_source_of_time() {
    let g = open();
    g.claim(&alice(), "r", "src", 10, "").unwrap();
    assert_eq!(g.claims(&alice(), "r").unwrap()[0].expires, 110);
    assert_eq!(g.now(), 100);
}

#[test]
fn tail_includes_everyones_messages_and_head_reports_the_top_seq() {
    let g = open();
    g.post(&alice(), "r", "one").unwrap();
    g.post(&bob(), "r", "two").unwrap();
    assert_eq!(g.tail(&alice(), "r", 0, 10).unwrap().len(), 2);
    assert_eq!(g.head(&alice(), "r").unwrap(), 2);
}

#[test]
fn an_ack_moves_the_read_cursor_so_the_hook_path_does_not_redeliver() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    for i in 0..3 {
        g.post(&bob(), "r", &format!("m{i}")).unwrap();
    }
    g.ack(&alice(), "r", 2).unwrap();
    let unread = g.read(&alice(), "r", false).unwrap();
    assert_eq!(unread.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![3]);
}

#[test]
fn a_verified_principal_posts_a_verified_message_and_an_unattributed_one_stays_unattributed() {
    let g = open();
    g.post(&Principal::verified("carol"), "r", "proven")
        .unwrap();
    g.post(&Principal::unattributed("eve"), "r", "unproven")
        .unwrap();
    let got = g.tail(&alice(), "r", 0, 10).unwrap();
    assert_eq!(got[0].attribution, klatch_core::Attribution::Verified);
    assert_eq!(got[1].attribution, klatch_core::Attribution::Unattributed);
}

#[test]
fn messages_are_posted_under_the_default_label() {
    let g = open();
    g.post(&alice(), "r", "x").unwrap();
    assert_eq!(
        g.tail(&alice(), "r", 0, 10).unwrap()[0].label,
        DEFAULT_LABEL
    );
    assert_eq!(DEFAULT_LABEL, "public");
}

#[test]
fn a_message_a_digest_showed_can_still_be_read_once() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "ask").unwrap();
    assert_eq!(g.digest(&alice()).unwrap().rooms[0].messages.len(), 1);
    assert!(g.digest(&alice()).unwrap().rooms[0].messages.is_empty());
    assert_eq!(g.read(&alice(), "r", true).unwrap().len(), 1);
    assert!(g.read(&alice(), "r", true).unwrap().is_empty());
}

#[test]
fn a_message_a_wait_has_seen_is_still_unread_and_the_wait_does_not_wake_again() {
    let g = open();
    g.join(&alice(), "r").unwrap();
    g.post(&bob(), "r", "ask").unwrap();
    assert_eq!(g.wait_poll(&alice(), "r").unwrap().len(), 1);
    g.wait_seen(&alice(), "r", 1).unwrap();
    assert!(g.wait_poll(&alice(), "r").unwrap().is_empty());
    assert_eq!(g.read(&alice(), "r", false).unwrap().len(), 1);
}
