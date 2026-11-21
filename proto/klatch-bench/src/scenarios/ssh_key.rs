use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "two_ssh_forced_command_keys_are_two_members_and_neither_can_claim_to_be_the_other",
        origin: "#107, DESIGN section 20 rule 11",
        layer: "fault",
        run: two_keys,
    }]
}

/// The human maps two key labels at the test terminal; `serve --stdio --ssh-key` as each is that member whatever
/// `KLATCH_NAME` says, a frame naming the other member or forging an identity field is refused, an unmapped label does
/// not start, and the two acts are in the audit. No ssh or sshd is involved: the bytes are written to standard input.
fn two_keys(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("worker", &["join", "lobby"])?;
    check(
        w.admin(&["ssh-key", "key-a", "member-a"], "").ok,
        "mapping key-a failed",
    )?;
    check(
        w.admin(&["ssh-key", "key-b", "member-b"], "").ok,
        "mapping key-b failed",
    )?;
    w.advance(5);
    let a = w.serve_ssh(
        "key-a",
        "member-b",
        &[World::post_request(1, "lobby", "from a")],
    )?;
    has(
        &a.join("\n"),
        "\"member\":\"member-a\"",
        "key-a is member-a",
    )?;
    has(
        &a.join("\n"),
        "\"attribution\":\"verified\"",
        "the key is verified",
    )?;
    all_ok(&a[1..], 1)?;
    let b = w.serve_ssh(
        "key-b",
        "member-a",
        &[World::post_request(1, "lobby", "from b")],
    )?;
    has(
        &b.join("\n"),
        "\"member\":\"member-b\"",
        "key-b is member-b",
    )?;
    // a frame that names the other member is refused and runs nothing
    let forged = serde_json::json!({"v": 1, "type": "request", "id": "f", "method": "post",
        "params": {"room": "lobby", "body": "forged", "member": "member-b"}})
    .to_string();
    let r = w.serve_ssh("key-a", "member-b", &[forged])?.join("\n");
    has(&r, "member-fixed", "a frame naming another member")?;
    // an unmapped label does not start
    let e = w
        .serve_ssh("key-c", "member-a", &[])
        .err()
        .ok_or("an unmapped label started a session")?;
    has(&e, "ssh-key-unmapped", "the unmapped label")?;
    let log = w.ok("worker", &["read", "lobby"])?;
    has(&log, "from a", "a's post")?;
    has(&log, "from b", "b's post")?;
    lacks(&log, "forged", "the forged post")?;
    let audit = w.admin(&["audit"], "");
    check(audit.ok, "audit failed")?;
    has(
        &audit.stdout,
        "key-a/member-a=true",
        "the mapping is audited",
    )?;
    Ok(vec![])
}
