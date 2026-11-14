use super::*;

#[test]
fn a_post_over_the_size_cap_is_refused_with_the_reason_and_stores_nothing() {
    let g = Gateway::new(SpyStore::new(), FixedClock(1), AllowAll);
    let at_cap = "a".repeat(crate::domain::budget::MAX_POST_BYTES);
    g.post(&alice(), "r", &at_cap).unwrap();
    let err = g
        .post(&alice(), "r", &format!("{at_cap}a"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("too large") && err.contains("32768"), "{err}");
    assert_eq!(g.store.head("r").unwrap(), 1);
}

#[test]
fn an_identical_repeat_within_a_minute_is_refused_naming_the_original() {
    let (g, clock) = timed();
    let first = g.post(&alice(), "r", "closed #3").unwrap();
    clock.set(1_010);
    let err = g.post(&alice(), "r", "closed #3").unwrap_err().to_string();
    assert!(
        err.contains(&format!("same as your message #{}", first.seq)) && err.contains("10s ago"),
        "{err}"
    );
    g.post(&bob(), "r", "closed #3").unwrap();
    g.post(&alice(), "other", "closed #3").unwrap();
    g.post(&alice(), "r", "closed #4").unwrap();
    clock.set(1_061);
    g.post(&alice(), "r", "closed #3").unwrap();
}

#[test]
fn a_burst_of_posts_is_refused_with_how_long_to_wait_and_clears_with_time() {
    let (g, clock) = timed();
    for i in 0..crate::domain::budget::MAX_POSTS_PER_WINDOW {
        g.post(&alice(), "r", &format!("m{i}")).unwrap();
    }
    let err = g
        .post(&alice(), "r", "one too many")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("at most 100 posts in 60 seconds") && err.contains("wait"),
        "{err}"
    );
    g.post(&bob(), "r", "someone else is not limited").unwrap();
    clock.set(1_061);
    g.post(&alice(), "r", "after the window").unwrap();
}
