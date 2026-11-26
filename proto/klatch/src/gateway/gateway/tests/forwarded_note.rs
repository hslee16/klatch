//! The server's note of ignored forwarded identity headers (#1490): one counts-only audit row, nothing a peer chose.

use super::*;

fn rows(g: &Gateway<SpyStore>) -> Vec<crate::domain::AuditRecord> {
    g.admin_audit(&admin(), 50).unwrap()
}

#[test]
fn a_counts_only_note_is_one_audit_row_under_a_fixed_action() {
    let g = open();
    g.note_forwarded_ignored("requests=3 x-forwarded-for=2 x-ssl-client-*=1")
        .unwrap();
    let kept: Vec<_> = rows(&g)
        .into_iter()
        .filter(|r| r.action == "forwarded-ignored")
        .collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].target, "");
    assert_eq!(
        kept[0].detail,
        "requests=3 x-forwarded-for=2 x-ssl-client-*=1"
    );
    assert_eq!(kept[0].ts, 100);
}

#[test]
fn text_that_is_not_names_and_counts_is_refused_and_nothing_is_kept() {
    let g = open();
    for bad in [
        "requests=1 x-forwarded-for=203.0.113.77",
        "requests=1 CN=evil",
        "requests=1\nsecret",
        "requests=1 user@example",
        &"a".repeat(600),
    ] {
        assert!(is_invalid(g.note_forwarded_ignored(bad)), "{bad:?}");
    }
    assert!(rows(&g).iter().all(|r| r.action != "forwarded-ignored"));
}
