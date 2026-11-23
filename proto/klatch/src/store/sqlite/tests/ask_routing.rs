//! Ask routing in the store (#1407): one record per ask and member, one event per ask and outcome, once each, and what
//! pruning does with them. Names and numbers only.

use super::*;
use crate::domain::RouteKind;

fn to(names: &[&str]) -> Vec<(String, RouteKind)> {
    names
        .iter()
        .map(|n| (n.to_string(), RouteKind::Capable))
        .collect()
}

#[test]
fn an_ask_is_routed_once_and_a_second_call_changes_nothing() {
    let s = store();
    assert!(s.route_ask("r", 4, &to(&["bob", "carol"]), 10).unwrap());
    assert!(!s.route_ask("r", 4, &to(&["dave"]), 11).unwrap());
    let rows = s.ask_routes("r").unwrap();
    assert_eq!(
        rows.iter().map(|r| r.member.as_str()).collect::<Vec<_>>(),
        ["bob", "carol"]
    );
    assert!(rows.iter().all(|r| r.seq == 4 && r.ts == 10));
    assert_eq!(s.route_outcomes("r", 4).unwrap(), ["routed"]);
    assert!(s.ask_routes("other").unwrap().is_empty());
}

#[test]
fn an_outcome_is_noted_once_and_added_routes_keep_the_earlier_kind() {
    let s = store();
    s.route_ask("r", 4, &to(&["bob"]), 10).unwrap();
    assert!(s.note_route_outcome("r", 4, "escalated", 1, 20).unwrap());
    assert!(!s.note_route_outcome("r", 4, "escalated", 1, 21).unwrap());
    s.add_ask_routes(
        "r",
        4,
        &[
            ("bob".into(), RouteKind::Coordinator),
            ("coord".into(), RouteKind::Coordinator),
        ],
        20,
    )
    .unwrap();
    let rows = s.ask_routes("r").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().find(|r| r.member == "bob").unwrap().kind,
        RouteKind::Capable
    );
    assert_eq!(
        rows.iter().find(|r| r.member == "coord").unwrap().kind,
        RouteKind::Coordinator
    );
}

#[test]
fn a_late_addressing_is_told_to_each_surface_once() {
    let s = store();
    s.route_ask("r", 4, &to(&["bob"]), 10).unwrap();
    s.add_ask_routes("r", 4, &[("coord".into(), RouteKind::Coordinator)], 20)
        .unwrap();
    assert!(
        s.untold_routes("r", "bob", 1).unwrap().is_empty(),
        "capable rows are not late"
    );
    assert_eq!(s.untold_routes("r", "coord", 1).unwrap(), [4]);
    s.mark_told("r", "coord", &[4], 1).unwrap();
    assert!(s.untold_routes("r", "coord", 1).unwrap().is_empty());
    assert_eq!(
        s.untold_routes("r", "coord", 2).unwrap(),
        [4],
        "the other surface has not"
    );
    s.mark_told("r", "coord", &[4], 2).unwrap();
    assert!(s.untold_routes("r", "coord", 2).unwrap().is_empty());
}

#[test]
fn the_tables_hold_no_text_and_old_rows_age_out() {
    let s = store();
    for table in ["ask_routes", "ask_route_events"] {
        let cols: String = s
            .conn
            .query_row(
                &format!("SELECT group_concat(name) FROM pragma_table_info('{table}')"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!cols.contains("body") && !cols.contains("text"), "{cols}");
    }
    s.route_ask("r", 1, &to(&["bob"]), 10).unwrap();
    s.route_ask("r", 2, &to(&["bob"]), 10 + 31 * 86_400)
        .unwrap();
    assert_eq!(
        s.route_events("r").unwrap().len(),
        1,
        "the 31 day old event is gone"
    );
    assert_eq!(s.ask_routes("r").unwrap().len(), 1);
}

#[test]
fn a_departed_member_takes_its_routes_with_it() {
    let s = store();
    s.join("r", "bob", 1).unwrap();
    s.route_ask("r", 1, &to(&["bob"]), 10).unwrap();
    s.prune_members(100, 100_000).unwrap();
    assert!(s.ask_routes("r").unwrap().is_empty());
}
