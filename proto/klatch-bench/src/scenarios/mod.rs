//! The scenarios, grouped by layer. Every one names its origin: a ticket (a bug that must not come back), a real-agent
//! run it replays, or a DESIGN section. They drive the real binary through `World` and assert on its output.

use crate::{FakeHarness, Scenario, World};
use std::time::Instant;

mod admin;
mod admin_role;
mod anchors;
mod asks;
mod bigstore;
mod claims;
mod delivery_view;
mod factory;
mod faults;
mod hook_keeps_newest;
mod hooks;
mod identity;
mod judge;
mod keys;
mod labels;
mod librarian;
mod liveness;
mod rate_limit;
mod receipts;
mod retention;
mod reviewer;
mod roles_path;
mod room_capability;
mod scale;
mod sessions;

/// The time every clock scenario starts at (unix seconds).
pub(super) const T0: i64 = 1_800_000_000;

pub(super) type Metrics = Vec<(String, f64)>;
pub(super) type Res = Result<Metrics, String>;

pub(super) fn check(cond: bool, what: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(what.into()) }
}

pub(super) fn has(text: &str, needle: &str, what: &str) -> Result<(), String> {
    check(
        text.contains(needle),
        format!("{what}: expected {needle:?} in:\n{text}"),
    )
}

pub(super) fn lacks(text: &str, needle: &str, what: &str) -> Result<(), String> {
    check(
        !text.contains(needle),
        format!("{what}: did not expect {needle:?} in:\n{text}"),
    )
}

pub(super) fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

pub(super) fn all_ok(responses: &[String], expected: usize) -> Result<(), String> {
    let oks = responses
        .iter()
        .filter(|l| l.contains("\"ok\":true"))
        .count();
    check(
        oks == expected,
        format!("expected {expected} ok responses, got {oks}"),
    )
}

pub(super) fn no_panic(text: &str, what: &str) -> Result<(), String> {
    check(
        !text.contains("panicked") && !text.contains("RUST_BACKTRACE"),
        format!("{what} panicked:\n{text}"),
    )
}

/// Every scenario, by topic.
pub fn all() -> Vec<Scenario> {
    [
        asks::all(),
        bigstore::all(),
        claims::all(),
        sessions::all(),
        faults::all(),
        scale::all(),
        factory::all(),
        admin::all(),
        labels::all(),
        librarian::all(),
        liveness::all(),
        reviewer::all(),
        admin_role::all(),
        room_capability::all(),
        delivery_view::all(),
        rate_limit::all(),
        anchors::all(),
        roles_path::all(),
        hooks::all(),
        hook_keeps_newest::all(),
        receipts::all(),
        identity::all(),
        keys::all(),
        judge::all(),
        retention::all(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
