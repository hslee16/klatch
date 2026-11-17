use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "an_open_pr_shows_held_then_without_an_owner_after_its_lease_lapses_and_ends_with_a_merged_note",
        origin: "#707, DESIGN s32 item 84",
        layer: "functional",
        run: pr_lifecycle_by_clock,
    }]
}

fn pr_lifecycle_by_clock(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("bob", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "pr: #7 add the thing"])?;
    w.ok("alice", &["claim", "lobby", "pr:7", "--shared"])?;
    let held = w.ok("bob", &["board", "lobby"])?;
    has(&held, "prs:", "the board has a prs section")?;
    has(&held, "#7 (held by alice)", "the claim holder is shown")?;
    w.advance(3_600);
    let lapsed = w.ok("bob", &["board", "lobby"])?;
    has(
        &lapsed,
        "#7 (open without an owner for",
        "a lapsed lease leaves the PR without an owner",
    )?;
    w.ok("bob", &["claim", "lobby", "pr:7", "--shared"])?;
    has(
        &w.ok("alice", &["board", "lobby"])?,
        "#7 (held by bob)",
        "taking the claim moves the PR",
    )?;
    w.ok("bob", &["release", "lobby", "pr:7", "--note", "merged"])?;
    lacks(
        &w.ok("alice", &["board", "lobby"])?,
        "prs:",
        "a merged note ends the PR",
    )?;
    Ok(vec![])
}
