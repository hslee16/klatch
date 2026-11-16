use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "creating_rooms_and_inviting_need_a_grant_once_the_human_requires_one",
        origin: "#188, DESIGN 32 item 68",
        layer: "fault",
        run: room_capability,
    }]
}

fn room_capability(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "lobby"])?;
    w.ok("ada", &["join", "lobby"])?;
    // open by default: nothing changes until the human decides
    w.ok("ada", &["join", "side-room"])?;
    check(
        w.admin(&["room-creation", "granted"], "").ok,
        "requiring a grant failed",
    )?;
    w.advance(5);
    let e = w.refused("ada", &["join", "another-room"])?;
    has(
        &e,
        "room-capability",
        "a member without a grant creating a room",
    )?;
    let e = w.refused("ada", &["invite", "worker", "side-room"])?;
    has(&e, "room-capability", "a member without a grant inviting")?;
    // a room that exists is still open to everyone
    w.ok("worker", &["join", "side-room"])?;
    check(
        w.admin(&["grant-rooms", "ada"], "").ok,
        "granting the capability failed",
    )?;
    w.advance(5);
    w.ok("ada", &["join", "another-room"])?;
    w.ok(
        "ada",
        &["invite", "worker", "another-room", "--via", "lobby"],
    )?;
    let audit = w.admin(&["audit"], "");
    check(audit.ok, "audit failed")?;
    has(&audit.stdout, "create-room", "the creation is audited")?;
    has(&audit.stdout, "created by ada", "the audit names who")?;
    has(&audit.stdout, "invited by ada", "the invite names who")?;
    Ok(vec![])
}
