use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        scrub_and_dm(),
        Scenario {
            name: "labels_keep_a_private_fact_out_of_a_public_room_and_every_refusal_is_audited",
            origin: "#40 to #45, R7 to R9, R27",
            layer: "fault",
            run: labels_and_clearance,
        },
    ]
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

pub(super) fn scrub_and_dm() -> Scenario {
    Scenario {
        name: "the_scrubber_refuses_private_terms_and_direct_messages_are_capped_rooms",
        origin: "#43, #44, R9",
        layer: "fault",
        run: scrub_and_direct_messages,
    }
}

fn scrub_and_direct_messages(w: &World) -> Res {
    w.set_clock(T0);
    check(
        w.admin(&["scrub-add", "hosts", "quokka.internal"], "").ok,
        "scrub-add failed",
    )?;
    let refused = w.refused("alice", &["post", "town", "ssh to QUOKKA.internal"])?;
    has(
        &refused,
        "private-term rule hosts",
        "a listed term is refused by rule",
    )?;
    lacks(
        &refused.to_lowercase(),
        "quokka",
        "the refusal repeated the term",
    )?;
    w.ok("alice", &["post", "town", "a quokka is a marsupial"])?;
    let list = w.admin(&["scrub-list"], "");
    has(
        &list.stdout,
        "hosts\tquokka.internal",
        "the list shows the rule",
    )?;
    check(
        w.admin(&["scrub-remove", "hosts"], "").ok,
        "scrub-remove failed",
    )?;
    w.ok("alice", &["post", "town", "quokka.internal is fine now"])?;
    // a direct message is a two-member room capped at the lower clearance
    check(
        w.admin(&["clear", "alice", "private"], "").ok,
        "clear failed",
    )?;
    check(
        w.admin(&["clear", "bob", "internal"], "").ok,
        "clear failed",
    )?;
    let sent = w.ok("alice", &["dm", "bob", "just between us"])?;
    has(
        &sent,
        "room dm.alice.bob",
        "a direct message names its room",
    )?;
    has(
        &w.ok("bob", &["read", "dm.alice.bob"])?,
        "just between us",
        "the recipient reads it",
    )?;
    has(
        &w.refused("alice", &["dm", "bob", "--label", "private", "too high"])?,
        "cannot go into room dm.alice.bob",
        "a message above the lower clearance is refused",
    )?;
    has(
        &w.refused("carol", &["read", "dm.alice.bob"])?,
        "direct message between two other members",
        "a third member is refused",
    )?;
    Ok(vec![])
}
