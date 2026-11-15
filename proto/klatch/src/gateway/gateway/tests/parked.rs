use super::*;
use crate::domain::Parked;

#[test]
fn a_parked_post_parks_its_sender_and_any_other_post_ends_it() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(
        &alice(),
        "r",
        "parked: waiting on the nightly build, back by 3h",
    )
    .unwrap();
    let who = g.who(&bob(), "r").unwrap();
    let p = who[0].parked.as_ref().expect("alice is parked");
    assert_eq!(p.why, "waiting on the nightly build");
    assert_eq!(p.back_by, 1_000 + 3 * 3600);
    t.set(1_100);
    g.post(&alice(), "r", "back, the build is done").unwrap();
    assert!(g.who(&bob(), "r").unwrap()[0].parked.is_none());
}

#[test]
fn a_parked_member_stays_on_the_board_past_the_hide_window_until_the_return_time() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.post(&alice(), "r", "parked: reviewing, back by 2h")
        .unwrap();
    t.set(1_000 + 3600);
    let b = g.board(&bob(), "r").unwrap();
    let a = b
        .members
        .iter()
        .find(|m| m.name == "alice")
        .expect("listed while parked");
    assert!(a.parked.is_some());
    assert_eq!(b.stale_members, 0);
    t.set(1_000 + 2 * 3600 + 1);
    let b = g.board(&bob(), "r").unwrap();
    assert!(
        b.members.iter().all(|m| m.name != "alice"),
        "after the return time it is hidden like any quiet member"
    );
    assert_eq!(b.stale_members, 1);
}

#[test]
fn a_stop_with_background_work_parks_the_member_and_a_later_stop_with_none_ends_it() {
    let (g, t) = timed();
    g.join(&alice(), "r").unwrap();
    g.record_stop(&alice(), 2, 1).unwrap();
    let p: Parked = g.who(&bob(), "r").unwrap()[0].parked.clone().unwrap();
    assert_eq!(p.why, "2 background tasks, 1 cron");
    assert_eq!(p.back_by, 1_000 + crate::domain::PARKED_DEFAULT_SECS);
    t.set(1_500);
    g.record_stop(&alice(), 0, 0).unwrap();
    assert!(g.who(&bob(), "r").unwrap()[0].parked.is_none());
}

#[test]
fn a_hook_that_runs_is_a_sign_of_life_and_ends_the_park() {
    let (g, _t) = timed();
    g.join(&alice(), "r").unwrap();
    g.record_stop(&alice(), 1, 0).unwrap();
    assert!(g.who(&bob(), "r").unwrap()[0].parked.is_some());
    g.sign_of_life(&alice()).unwrap();
    assert!(g.who(&bob(), "r").unwrap()[0].parked.is_none());
}
