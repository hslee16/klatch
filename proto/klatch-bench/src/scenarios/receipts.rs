use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_sender_sees_per_recipient_liveness_for_its_message_and_nobody_else_can_ask",
        origin: "#546, RESEARCH-NOTES 47.2, SURVEY 12",
        layer: "functional",
        run: receipt,
    }]
}

fn receipt(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["sender", "active", "silent", "parked"] {
        w.ok(who, &["join", "r"])?;
    }
    w.ok("parked", &["post", "r", "parked: building, back by 2h"])?;
    w.advance(10);
    w.ok("sender", &["post", "r", "please review the plan"])?;
    w.advance(30);
    w.ok("active", &["read", "r", "--peek"])?;
    // the parked member's seq 1 is the first message, so the sender's message is #2
    let out = w.ok("sender", &["receipt", "r", "2"])?;
    has(&out, "active: seen 0m after the post", "the active member")?;
    has(&out, "silent: not seen since the post", "the silent member")?;
    has(&out, "parked: parked, back in", "the parked member")?;
    check(
        !out.contains("please review the plan"),
        "the receipt printed message text",
    )?;
    has(
        &w.refused("active", &["receipt", "r", "2"])?,
        "only the sender",
        "a recipient could ask",
    )?;
    // shown and read (#675): the active member peeked (read cursor untouched, hook not run), so neither flag is set
    has(&out, "active: seen 0m after the post", "active line")?;
    has(&out, "shown: no, read: no", "untouched cursors")?;
    w.ok("silent", &["read", "r"])?;
    let out = w.ok("sender", &["receipt", "r", "2"])?;
    let silent = out
        .lines()
        .find(|l| l.contains("silent:"))
        .ok_or("no silent line")?;
    has(silent, "read: yes", "the silent member read it")?;
    Ok(vec![])
}
