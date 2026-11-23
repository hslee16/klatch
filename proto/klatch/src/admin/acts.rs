//! The admin acts that need only the gateway, grouped by what they act on. `execute` sends each command to its family.

use super::route::{Act, ListingAct, MemberAct, PolicyAct, RoleAct, RoomAct, ScrubAct};
use super::*;

/// "no longer" when a role is being taken away, else "now".
fn now_or_no_longer(off: bool) -> &'static str {
    if off { "no longer" } else { "now" }
}

pub(super) fn execute(t: &dyn Transport, act: Act) -> Result<()> {
    match act {
        Act::Room(a) => rooms(t, a),
        Act::Member(a) => members(t, a),
        Act::Role(a) => roles(t, a),
        Act::Policy(a) => policy(t, a),
        Act::Scrub(a) => scrub(t, a),
        Act::Listing(a) => listings(t, a),
    }
}

fn listings(t: &dyn Transport, act: ListingAct) -> Result<()> {
    match act {
        ListingAct::Refusals { limit } => listing::refusals(t, limit),
        ListingAct::Audit { limit } => listing::audit(t, limit),
        ListingAct::ReviewQueue { limit } => listing::review_queue(t, limit),
    }
}

fn rooms(t: &dyn Transport, act: RoomAct) -> Result<()> {
    match act {
        RoomAct::Pause => {
            t.admin_pause(true)?;
            println!(
                "all rooms are paused; nothing is posted, claimed or delivered until `klatch admin resume`"
            );
        }
        RoomAct::Resume => {
            t.admin_pause(false)?;
            println!("rooms resumed");
        }
        RoomAct::Close { room } => {
            t.admin_close_room(&room, true)?;
            println!("room {room} is closed (read-only)");
        }
        RoomAct::Reopen { room } => {
            t.admin_close_room(&room, false)?;
            println!("room {room} is open again");
        }
        RoomAct::Label { room, label } => {
            t.admin_set_room_label(&room, &label)?;
            println!("room {room} is labelled {label}");
        }
        RoomAct::Canary { room } => {
            t.admin_plant_canary(&room)?;
            println!("a canary is planted in {room}; a copy of it anywhere else pauses that room");
        }
        RoomAct::CanaryRetire { room } => {
            t.admin_retire_canary(&room)?;
            println!("the canary of {room} is retired");
        }
    }
    Ok(())
}

fn members(t: &dyn Transport, act: MemberAct) -> Result<()> {
    match act {
        MemberAct::Remove {
            member,
            room,
            all_rooms,
        } => {
            let room = room_of(room, all_rooms);
            t.admin_remove_member(&room, &member)?;
            println!(
                "{member} is removed from {room} and refused there until `klatch admin allow`"
            );
        }
        MemberAct::Allow {
            member,
            room,
            all_rooms,
        } => {
            let room = room_of(room, all_rooms);
            t.admin_allow_member(&room, &member)?;
            println!("{member} is allowed in {room} again");
        }
        MemberAct::Clear { member, label } => {
            t.admin_set_clearance(&member, &label)?;
            println!("{member} is cleared to {label}");
        }
        MemberAct::Grantor { room, member, off } => {
            t.admin_set_room_grantor(&room, &member, !off)?;
            println!(
                "{member} is {} the grantor of {room}",
                if off { "no longer" } else { "now" }
            );
        }
        MemberAct::RoomClear {
            room,
            member,
            label,
        } => {
            t.admin_set_room_clearance(&room, &member, label.as_deref())?;
            match label {
                Some(l) => println!("{member} is cleared to {l} in {room} only"),
                None => println!("{member}'s clearance in {room} is revoked"),
            }
        }
        MemberAct::SshKey { label, member, off } => {
            t.admin_set_ssh_key(&label, &member, !off)?;
            if off {
                println!(
                    "ssh key {label} is no longer {member}; end any session already open on it (the gateway checks the mapping when a session starts) and remove its line from authorized_keys"
                );
            } else {
                println!("ssh key {label} is now {member}");
            }
        }
        MemberAct::GrantRooms { member, off } => {
            t.admin_grant_room_capability(&member, !off)?;
            println!(
                "{member} {} create rooms and invite",
                if off { "may no longer" } else { "may now" }
            );
        }
    }
    Ok(())
}

fn roles(t: &dyn Transport, act: RoleAct) -> Result<()> {
    let (member, off, what) = match act {
        RoleAct::Librarian { member, off } => {
            t.admin_set_librarian(&member, !off)?;
            (member, off, "a librarian")
        }
        RoleAct::Reviewer { member, off } => {
            t.admin_set_reviewer(&member, !off)?;
            (member, off, "a reviewer")
        }
        RoleAct::Facilitator { member, off } => {
            t.admin_set_facilitator(&member, !off)?;
            (member, off, "a facilitator")
        }
        RoleAct::Integrator { member, off } => {
            t.admin_set_integrator(&member, !off)?;
            (member, off, "an integrator")
        }
        RoleAct::AdminAgent { member, off } => {
            t.admin_set_admin_agent(&member, !off)?;
            (member, off, "the admin agent")
        }
    };
    println!("{member} is {} {what}", now_or_no_longer(off));
    Ok(())
}

/// POSTS and SECS together, or `--off` alone; anything else is a usage error.
fn rate_limit_of(
    posts: Option<usize>,
    secs: Option<i64>,
    off: bool,
) -> Result<Option<(usize, i64)>> {
    match (off, posts, secs) {
        (true, None, None) => Ok(None),
        (false, Some(p), Some(s)) => Ok(Some((p, s))),
        _ => bail!("give POSTS and SECS (rate-limit 30 60), or --off alone"),
    }
}

/// One line per limit in force for `rate-limit --list`: `global` or `member NAME`, a tab, then the limit; one line
/// saying so when there is none.
fn rate_limit_lines(limits: &[klatch::gateway::RateLimitEntry]) -> Vec<String> {
    if limits.is_empty() {
        return vec!["no rate limit".to_string()];
    }
    limits
        .iter()
        .map(|(who, (p, s))| {
            let who = who
                .as_ref()
                .map_or("global".to_string(), |m| format!("member {m}"));
            format!("{who}\t{p} posts in {s} seconds")
        })
        .collect()
}

/// SECS, or `--off` alone; anything else is a usage error.
fn secs_or_off(secs: Option<i64>, off: bool, name: &str) -> Result<Option<i64>> {
    match (off, secs) {
        (true, None) => Ok(None),
        (false, Some(s)) => Ok(Some(s)),
        _ => bail!("give SECS ({name} 600), or --off alone"),
    }
}

fn room_stale(t: &dyn Transport, room: &str, (secs, off): (Option<i64>, bool)) -> Result<()> {
    let secs = secs_or_off(secs, off, "room-stale ROOM")?;
    t.admin_set_room_stale(room, secs)?;
    let said =
        secs.map(|s| format!("an ask with no claim is reported to its asker after {s} seconds"));
    println!("{}", room_said(room, said, "the default stale threshold"));
    Ok(())
}

/// What a room setting change prints: what it now says, or that the room is back to the default.
fn room_said(room: &str, now: Option<String>, default: &str) -> String {
    match now {
        Some(text) => format!("{}: {text}", flat(room)),
        None => format!("{} is back to {default}", flat(room)),
    }
}

fn room_ask_lease(t: &dyn Transport, room: &str, (secs, off): (Option<i64>, bool)) -> Result<()> {
    let secs = secs_or_off(secs, off, "room-ask-lease ROOM")?;
    t.admin_set_room_ask_lease(room, secs)?;
    let said =
        secs.map(|s| format!("an ask claim lasts {s} seconds unless the claimer asks for less"));
    println!("{}", room_said(room, said, "the default ask lease"));
    Ok(())
}

fn room_require(t: &dyn Transport, room: &str, (spec, off): (Option<String>, bool)) -> Result<()> {
    let spec = match (off, spec) {
        (true, None) => None,
        (false, Some(s)) => Some(s),
        _ => bail!("give MODE:CAPABILITIES (room-require ROOM refuse:wake), or --off alone"),
    };
    t.admin_set_room_require(room, spec.as_deref())?;
    let said = spec.map(|s| format!("joining needs {s}"));
    println!("{}", room_said(room, said, "no harness requirement"));
    Ok(())
}

fn room_creation(t: &dyn Transport, policy: RoomPolicy) -> Result<()> {
    let granted = matches!(policy, RoomPolicy::Granted);
    t.admin_set_room_creation_granted(granted)?;
    println!(
        "creating rooms and inviting is now {}",
        if granted {
            "for members with a grant"
        } else {
            "open to every member"
        }
    );
    Ok(())
}

fn member_rate_limit(t: &dyn Transport, member: &str, limit: Option<(usize, i64)>) -> Result<()> {
    t.admin_set_member_rate_limit(member, limit)?;
    match limit {
        Some((p, s)) => println!(
            "{member} may post {p} times in {s} seconds in all rooms, whatever the global limit is"
        ),
        None => println!("{member} has no limit of its own and follows the global one"),
    }
    Ok(())
}

/// `policy rate-limit`: the listing, one member's own limit, or the global one.
fn rate_limit(
    t: &dyn Transport,
    (posts, secs, off): (Option<usize>, Option<i64>, bool),
    list: bool,
    member: Option<String>,
) -> Result<()> {
    if list {
        for line in rate_limit_lines(&t.admin_rate_limits()?) {
            println!("{line}");
        }
        return Ok(());
    }
    let limit = rate_limit_of(posts, secs, off)?;
    if let Some(member) = member {
        return member_rate_limit(t, &member, limit);
    }
    t.admin_set_rate_limit(limit)?;
    match limit {
        Some((p, s)) => {
            println!("each member may post {p} times in {s} seconds in all rooms")
        }
        None => println!("no rate limit"),
    }
    Ok(())
}

fn policy(t: &dyn Transport, act: PolicyAct) -> Result<()> {
    match act {
        PolicyAct::RoomCreation { policy } => room_creation(t, policy)?,
        PolicyAct::RateLimit {
            posts,
            secs,
            off,
            list,
            member,
        } => rate_limit(t, (posts, secs, off), list, member)?,
        PolicyAct::LeaseCap { secs, off } => {
            let secs = secs_or_off(secs, off, "lease-cap")?;
            t.admin_set_lease_cap(secs)?;
            match secs {
                Some(s) => {
                    println!("a busy holder with no progress keeps its claims for {s} seconds")
                }
                None => println!("the lease cap is back to its default"),
            }
        }
        PolicyAct::LeaseGrace { secs, off } => {
            let secs = secs_or_off(secs, off, "lease-grace")?;
            t.admin_set_lease_grace(secs)?;
            match secs {
                Some(s) => println!("the claims of a holder found gone linger {s} seconds"),
                None => println!("the lease grace is back to its default"),
            }
        }
        PolicyAct::RoomStale { room, secs, off } => room_stale(t, &room, (secs, off))?,
        PolicyAct::RoomAskLease { room, secs, off } => room_ask_lease(t, &room, (secs, off))?,
        PolicyAct::RoomRequire { room, spec, off } => room_require(t, &room, (spec, off))?,
        PolicyAct::Cap { label } => {
            t.admin_set_cap(&label)?;
            println!("the agent cap is {label}");
        }
        PolicyAct::Chain { levels } => {
            let names: Vec<&str> = levels.split(',').map(str::trim).collect();
            t.admin_set_label_chain(&names)?;
            println!(
                "label order is {}; the agent cap is back to its default",
                names.join(" < ")
            );
        }
    }
    Ok(())
}

fn scrub(t: &dyn Transport, act: ScrubAct) -> Result<()> {
    match act {
        ScrubAct::Add { name, term } => {
            t.admin_add_scrub_term(&name, &term.join(" "))?;
            println!("rule {name} now refuses its term in rooms below the agent cap");
        }
        ScrubAct::Remove { name } => {
            t.admin_remove_scrub_term(&name)?;
            println!("rule {name} removed");
        }
        ScrubAct::List => {
            for (name, term) in t.admin_scrub_terms()? {
                println!("{name}\t{}", flat(&term));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lease_setting_is_seconds_or_off_alone() {
        assert_eq!(
            secs_or_off(Some(600), false, "lease-cap").unwrap(),
            Some(600)
        );
        assert_eq!(secs_or_off(None, true, "lease-cap").unwrap(), None);
        assert!(secs_or_off(Some(600), true, "lease-cap").is_err());
        assert!(secs_or_off(None, false, "lease-cap").is_err());
    }

    #[test]
    fn the_rate_limit_listing_is_one_line_each_or_one_line_saying_none() {
        assert_eq!(rate_limit_lines(&[]), vec!["no rate limit"]);
        let lines = rate_limit_lines(&[(None, (30, 60)), (Some("alice".into()), (2, 10))]);
        assert_eq!(
            lines,
            vec![
                "global\t30 posts in 60 seconds",
                "member alice\t2 posts in 10 seconds"
            ]
        );
    }

    #[test]
    fn a_rate_limit_is_both_numbers_or_off_alone() {
        assert_eq!(
            rate_limit_of(Some(30), Some(60), false).unwrap(),
            Some((30, 60))
        );
        assert_eq!(rate_limit_of(None, None, true).unwrap(), None);
        assert!(rate_limit_of(Some(30), None, false).is_err());
        assert!(rate_limit_of(Some(30), Some(60), true).is_err());
        assert!(rate_limit_of(None, None, false).is_err());
    }
}
