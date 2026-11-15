//! Lease rules driven by the liveness probe (#304, DESIGN section 29 item 4, section 32 item 47), on the controlled
//! clock with real fake sessions: a process named `claude`, a registry record with a turn state, a real process
//! table. A claim whose time to live has long passed holds while its holder is alive; a holder that is gone loses
//! its claims a grace period later, however long they were claimed for.

use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a_claim_past_its_ttl_holds_while_its_holder_is_alive",
            origin: "#304, #308",
            layer: "functional",
            run: held_while_alive,
        },
        Scenario {
            name: "a_dead_holders_long_claim_lapses_after_the_grace_period",
            origin: "#304, #308",
            layer: "fault",
            run: lapses_after_grace,
        },
    ]
}

/// Two sessions, both joined, with the holder idle at T0.
fn pair(h: &FakeHarness) -> Result<(crate::Session<'_>, crate::Session<'_>), String> {
    let holder = h.start("aaaa-0001", "holder")?;
    let asker = h.start("bbbb-0002", "asker")?;
    holder.hook("SessionStart")?;
    asker.hook("SessionStart")?;
    holder.at(T0)?;
    asker.at(T0)?;
    holder.set_turn("idle", T0)?;
    Ok((holder, asker))
}

fn held_while_alive(w: &World) -> Res {
    w.set_clock(T0);
    let h = FakeHarness::new(w)?;
    let (holder, asker) = pair(&h)?;
    let (ok, out) = holder.klatch(&["claim", "lobby", "src/a.rs", "--ttl", "60"])?;
    check(ok, format!("the holder claims: {out}"))?;
    asker.at(T0 + 3600)?;
    let (ok, out) = asker.klatch(&["claim", "lobby", "src/a.rs", "--ttl", "60"])?;
    check(
        !ok && out.contains("holder"),
        format!("an hour past the TTL the idle but live holder keeps the claim: {out}"),
    )?;
    // busy with a turn that began a minute ago: working
    holder.set_turn("busy", T0 + 3540)?;
    asker.at(T0 + 7200)?;
    holder.set_turn("busy", T0 + 7200 - 60)?;
    let (ok, out) = asker.klatch(&["claim", "lobby", "src/a.rs", "--ttl", "60"])?;
    check(
        !ok && out.contains("holder"),
        format!("two hours past the TTL the working holder keeps it: {out}"),
    )?;
    Ok(vec![])
}

fn lapses_after_grace(w: &World) -> Res {
    w.set_clock(T0);
    let h = FakeHarness::new(w)?;
    let (holder, asker) = pair(&h)?;
    let (ok, out) = holder.klatch(&["claim", "lobby", "src/a.rs", "--ttl", "3600"])?;
    check(ok, format!("the holder claims for an hour: {out}"))?;
    holder.die()?;
    asker.at(T0 + 10)?;
    let (ok, out) = asker.klatch(&["claim", "lobby", "src/a.rs"])?;
    check(
        !ok && out.contains("holder"),
        format!("inside the grace period the dead holder's claim lingers: {out}"),
    )?;
    asker.at(T0 + 10 + 301)?;
    let (ok, out) = asker.klatch(&["claim", "lobby", "src/a.rs"])?;
    check(
        ok,
        format!("after the grace period the claim is free, though it had 50 minutes left: {out}"),
    )?;
    Ok(vec![])
}
