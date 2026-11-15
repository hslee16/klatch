use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "the_admin_agent_role_is_granted_by_the_human_and_replaces_another_role",
        origin: "#50, R30",
        layer: "fault",
        run: admin_role,
    }]
}

fn admin_role(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "town"])?;
    check(
        w.admin(&["admin-agent", "ada"], "").ok,
        "granting the role failed",
    )?;
    // one role per member: making it a librarian replaces the admin agent role, and it may only look
    check(
        w.admin(&["librarian", "ada"], "").ok,
        "making it a librarian failed",
    )?;
    w.advance(600);
    has(
        &w.refused("ada", &["post", "town", "hello"])?,
        "librarian",
        "post",
    )?;
    check(
        w.admin(&["admin-agent", "ada", "--off"], "").ok,
        "removing the role failed",
    )?;
    Ok(vec![])
}
