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

pub(super) fn audit(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for a in t.admin_audit(limit)? {
        println!(
            "{}",
            aged_line(now, a.ts, &[&a.action, &flat(&a.target), &flat(&a.detail)])
        );
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
            or_else(&r.verdict, "waiting"),
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

    #[test]
    fn an_empty_field_falls_back() {
        assert_eq!(or_else("", "waiting"), "waiting");
        assert_eq!(or_else("ok", "waiting"), "ok");
    }
}
