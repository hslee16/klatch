use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_narrowed_view_shows_own_scope_addressed_and_broadcast_news_and_read_shows_all",
        origin: "#186, DESIGN 24.1 decision 5",
        layer: "fault",
        run: delivery_view,
    }]
}

fn delivery_view(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("listener", &["--member-scope", "team:a", "join", "lobby"])?;
    w.ok("mate", &["--member-scope", "team:a", "join", "lobby"])?;
    w.ok("other", &["--member-scope", "team:b", "join", "lobby"])?;
    has(
        &w.ok("listener", &["view"])?,
        "view: all",
        "the default view",
    )?;
    has(
        &w.ok("listener", &["view", "scope"])?,
        "view: scope",
        "narrowing",
    )?;
    w.refused("listener", &["view", "nonsense"])?;
    w.advance(5);
    w.ok("other", &["post", "lobby", "chatter from the other team"])?;
    w.advance(5);
    w.ok("mate", &["post", "lobby", "note from a teammate"])?;
    w.advance(5);
    w.ok("other", &["post", "lobby", "@listener this one is for you"])?;
    w.advance(5);
    w.ok("other", &["post", "lobby", "@all everyone please pull"])?;
    let seen = w.ok("listener", &["hook", "--event", "UserPromptSubmit"])?;
    has(&seen, "note from a teammate", "own scope")?;
    has(&seen, "this one is for you", "addressed")?;
    has(&seen, "everyone please pull", "broadcast")?;
    lacks(
        &seen,
        "chatter from the other team",
        "other scope, not addressed",
    )?;
    // a view never narrows an explicit read
    let read = w.ok("listener", &["read", "lobby", "--peek"])?;
    has(&read, "chatter from the other team", "read shows all")?;
    Ok(vec![])
}
