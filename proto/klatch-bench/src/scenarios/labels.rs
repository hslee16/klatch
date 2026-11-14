use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "labels_keep_a_private_fact_out_of_a_public_room_and_every_refusal_is_audited",
        origin: "#40 to #45, R7 to R9, R27",
        layer: "fault",
        run: labels_and_clearance,
    }]
}

fn labels_and_clearance(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["priv", "pub"] {
        w.ok(who, &["join", "town"])?;
    }
    check(
        w.admin(&["label", "vault", "private"], "").ok,
        "label failed",
    )?;
    check(
        w.admin(&["clear", "priv", "private"], "").ok,
        "clear failed",
    )?;
    // a member below the room is refused in it, with the levels named
    has(
        &w.refused("pub", &["post", "vault", "hello"])?,
        "labelled private, above your clearance public",
        "a public member is refused in a private room",
    )?;
    w.ok("priv", &["post", "vault", "the machine is called quokka"])?;
    // a private fact cannot enter a public room, and the refusal does not repeat it
    let refused = w.refused(
        "priv",
        &[
            "post",
            "--label",
            "private",
            "town",
            "quokka is the machine",
        ],
    )?;
    has(
        &refused,
        "cannot go into room town",
        "a private message into a public room",
    )?;
    lacks(&refused, "quokka", "the refusal repeated the text")?;
    check(
        w.ok("pub", &["read", "town", "--peek"])?.trim().is_empty(),
        "the refused message reached the public room",
    )?;
    // no agent can be cleared past the cap, so secret stays out of reach
    check(
        !w.admin(&["clear", "priv", "secret"], "").ok,
        "an agent was cleared to secret",
    )?;
    // the audit lists the refusals by rule and never the text
    let log = w.admin(&["refusals"], "");
    check(log.ok, "refusals failed")?;
    let rules: Vec<&str> = log
        .stdout
        .lines()
        .filter_map(|l| l.split('\t').nth(4))
        .collect();
    check(rules == ["label", "clearance"], format!("rules: {rules:?}"))?;
    lacks(&log.stdout, "quokka", "the audit repeated the text")?;
    // lowering a clearance takes effect on the next call
    check(
        w.admin(&["clear", "priv", "internal"], "").ok,
        "clear failed",
    )?;
    has(
        &w.refused("priv", &["post", "vault", "one more"])?,
        "above your clearance internal",
        "a lowered clearance applies at once",
    )?;
    Ok(vec![])
}
