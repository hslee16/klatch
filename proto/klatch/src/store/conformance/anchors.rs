use super::*;
pub fn releasing_an_anchor_releases_its_live_claims_in_every_room_and_only_those<S: Store>(
    make: fn() -> S,
) {
    let s = make();
    take(
        &s,
        ("r1", "w-1"),
        "src/a.rs",
        Lease::new(100, 600).anchored("wo:7"),
    );
    take(
        &s,
        ("r2", "w-2"),
        "src/b.rs",
        Lease::new(100, 600).anchored("wo:7"),
    );
    take(
        &s,
        ("r1", "w-3"),
        "src/c.rs",
        Lease::new(100, 600).anchored("wo:8"),
    );
    take(
        &s,
        ("r1", "w-4"),
        "src/d.rs",
        Lease::new(100, 600).anchored(""),
    );
    let mut released = s.release_anchored("wo:7", 200).unwrap();
    released.sort();
    assert_eq!(
        released,
        vec![
            ("r1".to_string(), "src/a.rs".to_string(), "w-1".to_string()),
            ("r2".to_string(), "src/b.rs".to_string(), "w-2".to_string()),
        ]
    );
    let held: Vec<String> = s
        .claims("r1", 200)
        .unwrap()
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(held, vec!["src/c.rs".to_string(), "src/d.rs".to_string()]);
    assert!(s.claims("r2", 200).unwrap().is_empty());
    assert!(
        s.release_anchored("wo:7", 200).unwrap().is_empty(),
        "a second release found something"
    );
}
pub fn an_expired_claim_is_cleared_but_not_reported_as_released<S: Store>(make: fn() -> S) {
    let s = make();
    take(
        &s,
        ("r", "w-1"),
        "src/a.rs",
        Lease::new(100, 600).anchored("wo:7"),
    ); // expires at 700
    assert!(s.release_anchored("wo:7", 800).unwrap().is_empty());
    assert!(s.claims("r", 800).unwrap().is_empty());
}
pub fn a_released_path_can_be_claimed_by_someone_else_at_once<S: Store>(make: fn() -> S) {
    let s = make();
    take(
        &s,
        ("r", "w-1"),
        "src/a.rs",
        Lease::new(100, 600).anchored("wo:7"),
    );
    assert!(
        s.claim(&ClaimRequest::new(("r", "w-2"), "src/a.rs", 600, ""), 150)
            .is_err()
    );
    s.release_anchored("wo:7", 160).unwrap();
    s.claim(&ClaimRequest::new(("r", "w-2"), "src/a.rs", 600, ""), 170)
        .unwrap();
}
pub fn the_anchors_in_use_are_the_distinct_anchors_of_live_claims<S: Store>(make: fn() -> S) {
    let s = make();
    take(&s, ("r", "w-1"), "a", Lease::new(100, 600).anchored("wo:2"));
    take(&s, ("r", "w-2"), "b", Lease::new(100, 600).anchored("wo:1"));
    take(&s, ("r", "w-3"), "c", Lease::new(100, 600).anchored("wo:1"));
    take(&s, ("r", "w-4"), "d", Lease::new(100, 600).anchored(""));
    take(
        &s,
        ("r", "w-5"),
        "e",
        Lease::new(10, 600).anchored("wo:gone"),
    ); // expires at 610
    assert_eq!(
        s.anchors_in_use(650).unwrap(),
        vec!["wo:1".to_string(), "wo:2".to_string()]
    );
}
