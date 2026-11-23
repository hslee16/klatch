use super::*;

#[test]
fn a_deadline_in_the_ask_text_is_read_as_utc() {
    // 2026-11-20 is 20777 days after the epoch
    let day = 20_777 * 86_400;
    let cases = [
        ("seeking: a review (by 2026-11-20)", Some(day + 86_399)),
        (
            "seeking: a review (by 2026-11-20 14:30)",
            Some(day + 14 * 3600 + 30 * 60),
        ),
        (
            "seeking: a (by 2026-11-19) b (by 2026-11-20)",
            Some(day + 86_399),
        ),
        ("seeking: a review by 2026-11-20", None),
        ("seeking: a review (by tomorrow)", None),
        ("seeking: a review (by 2026-13-40)", None),
        ("seeking: a review (by 2026-11-20 25:00)", None),
        ("seeking: nothing", None),
    ];
    for (text, want) in cases {
        assert_eq!(ask_deadline(text), want, "{text}");
    }
}

#[test]
fn civil_days_counts_real_calendar_days_only() {
    let cases = [
        ("1970-01-01", Some(0)),
        ("2000-02-29", Some(11_016)),
        ("2026-11-20", Some(20_777)),
        ("1900-02-29", None),
        ("2026-02-29", None),
        ("2026-04-31", None),
        ("2026-00-10", None),
        ("2026-12-32", None),
        ("2026-11-20-1", None),
        ("2026-11", None),
        ("2026-11-x", None),
    ];
    for (date, want) in cases {
        assert_eq!(civil_days(date), want, "{date}");
    }
}

#[test]
fn a_deadline_moves_the_stale_threshold_and_shows_on_the_summary() {
    let day = 20_777 * 86_400;
    let a = open_asks(
        &[msg(
            3,
            "asker",
            "seeking: x (by 2026-11-20)",
            day - 3 * 86_400,
        )],
        &[],
    )
    .remove(0);
    assert_eq!(
        a.stale_after(60),
        3 * 86_400 + 86_399,
        "the deadline replaces the room default"
    );
    assert!(
        a.summary(day - 86_400).contains("due in 1d"),
        "{}",
        a.summary(day - 86_400)
    );
    assert!(
        a.summary(day + 86_400 + 3600).contains("overdue 1h"),
        "{}",
        a.summary(day + 86_400 + 3600)
    );
    let plain = open_asks(&[msg(4, "asker", "seeking: y", 0)], &[]).remove(0);
    assert_eq!(plain.stale_after(60), 60);
    let past = open_asks(
        &[msg(
            5,
            "asker",
            "seeking: z (by 2026-11-20)",
            day + 86_400 * 2,
        )],
        &[],
    )
    .remove(0);
    assert_eq!(
        past.stale_after(60),
        60,
        "a deadline before the ask was posted never lowers the default"
    );
}
