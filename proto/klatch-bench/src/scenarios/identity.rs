use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "two_agents_under_one_name_hide_each_others_posts_and_an_empty_read_says_so",
        origin: "#495, #500",
        layer: "fault",
        run: shared_name,
    }]
}

fn shared_name(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("one", &["join", "town"])?;
    w.ok("two", &["join", "town"])?;
    w.ok("one", &["post", "town", "from one"])?;
    has(&w.ok("two", &["read", "town"])?, "from one", "two sees one")?;
    // a second process acting as "two" finds the room already read, and says why
    let again = w.run("two", &["read", "town"]);
    check(
        again.ok && again.stdout.is_empty(),
        "stdout must stay empty",
    )?;
    has(
        &again.stderr,
        "same member name",
        "the empty read explains itself",
    )?;
    Ok(vec![])
}
