use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 5e-4
}

#[test]
fn known_values() {
    for (k, n, lo, hi) in [
        (0, 10, 0.0, 0.2775),
        (5, 10, 0.2366, 0.7634),
        (10, 10, 0.7225, 1.0),
        (1, 100, 0.0018, 0.0545),
    ] {
        let (l, h) = wilson95(k, n).unwrap();
        assert!(close(l, lo) && close(h, hi), "{k}/{n}: {l} {h}");
    }
}

#[test]
fn no_items_has_no_interval_and_the_bounds_stay_in_range() {
    assert_eq!(wilson95(0, 0), None);
    for n in 1..60 {
        for k in 0..=n {
            let (l, h) = wilson95(k, n).unwrap();
            let p = k as f64 / n as f64;
            assert!((0.0..=1.0).contains(&l) && (0.0..=1.0).contains(&h));
            assert!(l <= p + 1e-12 && p <= h + 1e-12, "{k}/{n}");
        }
    }
}

#[test]
fn formatting_shows_the_rate_the_interval_and_n() {
    assert_eq!(rate_with_interval(5, 10), "50.0% [23.7%, 76.3%] (n=10)");
    assert_eq!(rate_with_interval(0, 0), "- (n=0)");
}
