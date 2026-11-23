//! A golden test of the whole usage report (#1442): the text and the JSON for a seeded store under several option
//! sets must equal the files in `golden/`. It guards the refactoring of `stats.rs` and `usage_stats.rs`; set
//! `KLATCH_WRITE_GOLDEN=1` to rewrite the files after an intended change to the report.

use super::stats_fixture::{D1, D2, bench_with_claims, opts};
use crate::domain::StatsOptions;

fn cases() -> Vec<(&'static str, StatsOptions)> {
    vec![
        ("all", opts()),
        (
            "lobby",
            StatsOptions {
                room: Some("lobby".into()),
                ..opts()
            },
        ),
        (
            "late",
            StatsOptions {
                since: Some(D2 - 1),
                top: 2,
                ..opts()
            },
        ),
        (
            "vault_early",
            StatsOptions {
                room: Some("vault".into()),
                since: Some(D1),
                top: 1,
                ..opts()
            },
        ),
    ]
}

fn render(o: &StatsOptions) -> String {
    let r = bench_with_claims().usage_stats(o).unwrap();
    let json = serde_json::to_string_pretty(&r.to_json()).unwrap();
    format!("{}\n=== json ===\n{json}\n", r.to_text())
}

#[test]
fn the_report_for_a_seeded_store_matches_the_golden_files() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/store/sqlite/tests/golden");
    for (name, o) in cases() {
        let got = render(&o);
        let path = dir.join(format!("stats_{name}.txt"));
        if std::env::var_os("KLATCH_WRITE_GOLDEN").is_some() {
            std::fs::write(&path, &got).unwrap();
        }
        let want = std::fs::read_to_string(&path).unwrap();
        assert_eq!(got, want, "golden {name} differs");
    }
}
