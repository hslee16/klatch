//! The read-only listings: the audit, the refusals and the review queue. Pointers and outcomes, never text.

use super::*;

/// One listing line: how long ago, then the other fields, tab separated.
fn aged_line(now: i64, ts: i64, fields: &[&str]) -> String {
    let mut line = format!("{}s ago", klatch::domain::secs_between(now, ts));
    for f in fields {
        line.push('\t');
        line.push_str(f);
    }
    line
}

/// The text of the filter's refusal, and the check clap runs on `--action`.
pub(super) fn valid_action(name: &str) -> std::result::Result<String, String> {
    if klatch::store::valid_audit_action(name) {
        Ok(name.to_string())
    } else {
        Err("an action is 1 to 64 of a-z, 0-9, -, _, : and .".to_string())
    }
}

/// The lines of the audit listing: the rows, newest first, then (default view only) one fixed line when operational
/// `forwarded-ignored` rows were left out (#1490). `only` is the exact action to show; it is checked again below the
/// transport, so a caller other than clap cannot send a bad name to a query.
fn audit_lines(
    t: &dyn Transport,
    now: i64,
    limit: usize,
    only: Option<&str>,
) -> Result<Vec<String>> {
    let page = t.admin_audit_page(limit, only)?;
    let mut lines: Vec<String> = page
        .rows
        .iter()
        .map(|a| aged_line(now, a.ts, &[&a.action, &flat(&a.target), &flat(&a.detail)]))
        .collect();
    if page.hidden > 0 {
        let action = klatch::store::FORWARDED_IGNORED_ACTION;
        lines.push(format!(
            "{} {action} rows hidden: --action {action} shows them",
            page.hidden
        ));
    }
    Ok(lines)
}

pub(super) fn audit(t: &dyn Transport, limit: usize, only: Option<&str>) -> Result<()> {
    for line in audit_lines(t, now(), limit, only)? {
        println!("{line}");
    }
    Ok(())
}

pub(super) fn refusals(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for r in t.admin_refusals(limit)? {
        println!(
            "{}",
            aged_line(
                now,
                r.ts,
                &[&flat(&r.member), &flat(&r.room), &r.operation, &r.rule]
            )
        );
    }
    Ok(())
}

/// The first of the two that is not empty, else `fallback`.
fn or_else<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

/// The verdict word, or `waiting` with the rules' flag when there is one (a waiting row's detail is that flag).
fn waiting_or_verdict(r: &klatch::store::QueuedReview) -> String {
    match (r.verdict.is_empty(), r.detail.is_empty()) {
        (true, false) => format!("waiting [{}]", flat(&r.detail)),
        (true, true) => "waiting".to_string(),
        _ => r.verdict.clone(),
    }
}

pub(super) fn review_queue(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for r in t.review_queue(limit)? {
        println!(
            "{}\t{}s ago\t{}#{}\t{}\t{}\t{}\t{}",
            r.id,
            klatch::domain::secs_between(now, r.ts),
            flat(&r.room),
            r.seq,
            or_else(&r.to_room, &r.to_label),
            waiting_or_verdict(&r),
            or_else(&r.applied, "-"),
            flat(&r.requester)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_line_is_the_age_then_tab_separated_fields() {
        assert_eq!(aged_line(100, 40, &["a", "b"]), "60s ago\ta\tb");
    }

    /// Five real admin acts, then `n` operational notes of ignored forwarded headers (the newest rows), as the human.
    fn after_acts_then_notes(n: usize) -> impl Transport {
        use klatch::gateway::areas::Admin;
        use klatch::session::Backend;
        let store = klatch::store::SqliteStore::open_in_memory().unwrap();
        let g = std::rc::Rc::new(klatch::gateway::Gateway::new(
            store,
            klatch::gateway::SystemClock,
            klatch::gateway::AllowAll,
        ));
        let t =
            klatch::gateway::Direct::new(g.clone(), klatch::domain::Principal::verified("human"));
        for i in 0..5 {
            t.admin_pause(i % 2 == 0).unwrap();
        }
        for _ in 0..n {
            g.note_forwarded_ignored("requests=1 x-real-ip=1").unwrap();
        }
        t
    }

    #[test]
    fn the_default_view_keeps_the_real_acts_in_sight_and_says_how_many_notes_it_hid() {
        let t = after_acts_then_notes(30);
        let lines = audit_lines(&t, 100, 20, None).unwrap();
        assert_eq!(lines.len(), 6, "{lines:#?}");
        assert!(
            lines[..5]
                .iter()
                .all(|l| l.contains("\tpause\t") || l.contains("\tresume\t")),
            "{lines:#?}"
        );
        assert_eq!(
            lines[5],
            "30 forwarded-ignored rows hidden: --action forwarded-ignored shows them"
        );
    }

    #[test]
    fn the_default_view_with_no_notes_has_no_hidden_line() {
        let t = after_acts_then_notes(0);
        let lines = audit_lines(&t, 100, 20, None).unwrap();
        assert_eq!(lines.len(), 5, "{lines:#?}");
        assert!(lines.iter().all(|l| !l.contains("hidden")));
    }

    #[test]
    fn an_action_filter_shows_only_those_rows_and_honours_the_limit() {
        let t = after_acts_then_notes(30);
        let all = audit_lines(&t, 100, 100, Some("forwarded-ignored")).unwrap();
        assert_eq!(all.len(), 30);
        assert!(all.iter().all(|l| l.contains("\tforwarded-ignored\t")));
        let some = audit_lines(&t, 100, 7, Some("forwarded-ignored")).unwrap();
        assert_eq!(some.len(), 7, "no hidden line with a filter");
        let acts = audit_lines(&t, 100, 20, Some("pause")).unwrap();
        assert_eq!(acts.len(), 3, "three of the five acts paused, two resumed");
        assert!(
            audit_lines(&t, 100, 20, Some("nothing-of-this-name"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_action_name_is_checked_against_the_action_charset() {
        for ok in ["pause", "forwarded-ignored", "rotate-label-key", "a_b:c.d"] {
            assert!(valid_action(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Pause", "a b", "a;b", "x\ny", "a'b", &"a".repeat(65)] {
            assert!(valid_action(bad).is_err(), "{bad:?}");
        }
        let t = after_acts_then_notes(0);
        assert!(audit_lines(&t, 100, 20, Some("Bad Name")).is_err());
    }

    #[test]
    fn an_empty_field_falls_back() {
        assert_eq!(or_else("", "waiting"), "waiting");
        assert_eq!(or_else("ok", "waiting"), "ok");
    }
}
