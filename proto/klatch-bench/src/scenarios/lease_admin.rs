use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "the_human_sets_and_lifts_the_lease_cap_and_grace_and_a_bad_value_is_refused",
        origin: "#537, #304",
        layer: "fault",
        run: lease_admin,
    }]
}

fn lease_admin(w: &World) -> Res {
    w.set_clock(T0);
    for (act, good, too_big) in [
        ("lease-cap", "86400", "604801"),
        ("lease-grace", "60", "86401"),
    ] {
        check(w.admin(&[act, good], "").ok, "setting failed")?;
        check(!w.admin(&[act, "0"], "").ok, "zero was accepted")?;
        check(
            !w.admin(&[act, too_big], "").ok,
            "a value past the bound was accepted",
        )?;
        check(
            !w.admin(&[act], "").ok,
            "no value and no --off was accepted",
        )?;
        check(w.admin(&[act, "--off"], "").ok, "lifting failed")?;
    }
    Ok(vec![])
}
