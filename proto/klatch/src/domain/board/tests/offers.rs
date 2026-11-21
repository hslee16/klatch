use super::*;

#[test]
fn an_offer_starts_with_the_prefix() {
    assert!(is_offer("offering: slugify help"));
    assert!(!is_offer("Offering: x"));
    assert!(!is_offer(" offering: x"));
    assert!(!is_offer("seeking: x"));
}

#[test]
fn an_offer_is_current_with_its_sender_and_age() {
    let m = [msg(1, ("helper", "offering: text helpers"), 10)];
    let o = current_offers(&m, 20, 3600);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0].sender.as_str(), o[0].seq, o[0].ts), ("helper", 1, 10));
    assert_eq!(o[0].text, "offering: text helpers");
}

#[test]
fn a_new_offer_from_the_same_sender_replaces_the_old_one() {
    let m = [
        msg(1, ("helper", "offering: old"), 10),
        msg(2, ("helper", "offering: new"), 20),
    ];
    let o = current_offers(&m, 30, 3600);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].text, "offering: new");
}

#[test]
fn a_withdrawal_clears_the_senders_offer_and_only_theirs() {
    let m = [
        msg(1, ("a", "offering: x"), 1),
        msg(2, ("b", "offering: y"), 2),
        msg(3, ("a", "withdrawn: no longer"), 3),
    ];
    let o = current_offers(&m, 10, 3600);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].sender, "b");
}

#[test]
fn status_done_clears_the_offer_but_another_status_does_not() {
    let done = [
        msg(1, ("a", "offering: x"), 1),
        msg(2, ("a", "status: done leaving"), 2),
    ];
    assert!(current_offers(&done, 10, 3600).is_empty());
    let busy = [
        msg(1, ("a", "offering: x"), 1),
        msg(2, ("a", "status: busy"), 2),
    ];
    assert_eq!(current_offers(&busy, 10, 3600).len(), 1);
    let back = [
        msg(1, ("a", "status: done"), 1),
        msg(2, ("a", "offering: again"), 2),
    ];
    assert_eq!(
        current_offers(&back, 10, 3600).len(),
        1,
        "an offer after done is current"
    );
}

#[test]
fn an_offer_expires_after_the_lifetime() {
    let m = [msg(1, ("a", "offering: x"), 100)];
    assert_eq!(current_offers(&m, 100 + 3600, 3600).len(), 1);
    assert!(current_offers(&m, 100 + 3601, 3600).is_empty());
}

#[test]
fn offers_come_oldest_first_and_other_messages_are_ignored() {
    let m = [
        msg(1, ("a", "hello"), 1),
        msg(2, ("b", "offering: b thing"), 2),
        msg(3, ("c", "seeking: z"), 3),
        msg(4, ("a", "offering: a thing"), 4),
    ];
    let who: Vec<_> = current_offers(&m, 10, 3600)
        .iter()
        .map(|o| o.sender.clone())
        .collect();
    assert_eq!(who, ["b", "a"]);
}

#[test]
fn statuses_are_the_latest_per_member() {
    let m = [
        msg(1, ("a", "status: busy"), 1),
        msg(2, ("b", "status: done"), 2),
        msg(3, ("a", "status: done leaving"), 3),
        msg(4, ("b", "chatter"), 4),
    ];
    let s = statuses(&m);
    assert_eq!(s.len(), 2);
    assert_eq!(
        (s[0].member.as_str(), s[0].status.as_str()),
        ("a", "done leaving")
    );
    assert_eq!((s[1].member.as_str(), s[1].status.as_str()), ("b", "done"));
}

#[test]
fn the_board_shows_asks_offers_and_members_with_ages_and_status() {
    let out = render_board(&sample(), 100);
    assert!(out.starts_with("[board lobby]"), "got: {out}");
    assert!(
        out.contains("#1 api-1 unclaimed 10s: seeking: slugify"),
        "got: {out}"
    );
    assert!(out.contains("#2 docs-1 claimed 20s: seeking: the api"));
    assert!(out.contains("lib-1 offers (50s ago): offering: text helpers"));
    assert!(out.contains("lib-1 last seen 2s ago"));
    assert!(out.contains("api-1 last seen 60s ago, status: done leaving"));
}
