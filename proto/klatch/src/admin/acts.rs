//! The admin acts that need only the gateway, grouped by what they act on. `execute` sends each command to its family.

use super::*;

/// "no longer" when a role is being taken away, else "now".
fn now_or_no_longer(off: bool) -> &'static str {
    if off { "no longer" } else { "now" }
}

pub(super) fn execute(t: &dyn Transport, cmd: AdminCmd) -> Result<()> {
    match cmd {
        AdminCmd::Pause
        | AdminCmd::Resume
        | AdminCmd::Close { .. }
        | AdminCmd::Reopen { .. }
        | AdminCmd::Canary { .. }
        | AdminCmd::Label { .. } => rooms(t, cmd),
        AdminCmd::Remove { .. }
        | AdminCmd::Allow { .. }
        | AdminCmd::Clear { .. }
        | AdminCmd::Librarian { .. }
        | AdminCmd::Reviewer { .. }
        | AdminCmd::AdminAgent { .. }
        | AdminCmd::GrantRooms { .. } => members(t, cmd),
        AdminCmd::RoomCreation { .. }
        | AdminCmd::RateLimit { .. }
        | AdminCmd::LeaseCap { .. }
        | AdminCmd::LeaseGrace { .. }
        | AdminCmd::Cap { .. }
        | AdminCmd::Chain { .. }
        | AdminCmd::ScrubAdd { .. }
        | AdminCmd::ScrubRemove { .. }
        | AdminCmd::ScrubList => policy(t, cmd),
        AdminCmd::Refusals { limit } => listing::refusals(t, limit),
        AdminCmd::Audit { limit } => listing::audit(t, limit),
        AdminCmd::ReviewQueue { limit } => listing::review_queue(t, limit),
        AdminCmd::Recovery
        | AdminCmd::Recover { .. }
        | AdminCmd::RotateKey
        | AdminCmd::Backup { .. }
        | AdminCmd::Restore { .. }
        | AdminCmd::Retention { .. }
        | AdminCmd::Archive { .. }
        | AdminCmd::RestoreArchive { .. }
        | AdminCmd::Archives
        | AdminCmd::Export { .. }
        | AdminCmd::SealLabel { .. }
        | AdminCmd::RunArchive { .. } => {
            unreachable!("handled in run, which needs the keystore and not only the gateway")
        }
    }
}

fn rooms(t: &dyn Transport, cmd: AdminCmd) -> Result<()> {
    match cmd {
        AdminCmd::Pause => {
            t.admin_pause(true)?;
            println!(
                "all rooms are paused; nothing is posted, claimed or delivered until `klatch admin resume`"
            );
        }
        AdminCmd::Resume => {
            t.admin_pause(false)?;
            println!("rooms resumed");
        }
        AdminCmd::Close { room } => {
            t.admin_close_room(&room, true)?;
            println!("room {room} is closed (read-only)");
        }
        AdminCmd::Reopen { room } => {
            t.admin_close_room(&room, false)?;
            println!("room {room} is open again");
        }
        AdminCmd::Label { room, label } => {
            t.admin_set_room_label(&room, &label)?;
            println!("room {room} is labelled {label}");
        }
        AdminCmd::Canary { room } => {
            t.admin_plant_canary(&room)?;
            println!("a canary is planted in {room}; a copy of it anywhere else pauses that room");
        }
        _ => unreachable!("not a room act"),
    }
    Ok(())
}

fn members(t: &dyn Transport, cmd: AdminCmd) -> Result<()> {
    match cmd {
        AdminCmd::Remove {
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
        AdminCmd::Allow {
            member,
            room,
            all_rooms,
        } => {
            let room = room_of(room, all_rooms);
            t.admin_allow_member(&room, &member)?;
            println!("{member} is allowed in {room} again");
        }
        AdminCmd::Clear { member, label } => {
            t.admin_set_clearance(&member, &label)?;
            println!("{member} is cleared to {label}");
        }
        AdminCmd::GrantRooms { member, off } => {
            t.admin_grant_room_capability(&member, !off)?;
            println!(
                "{member} {} create rooms and invite",
                if off { "may no longer" } else { "may now" }
            );
        }
        _ => return roles(t, cmd),
    }
    Ok(())
}

fn roles(t: &dyn Transport, cmd: AdminCmd) -> Result<()> {
    let (member, off, what) = match cmd {
        AdminCmd::Librarian { member, off } => {
            t.admin_set_librarian(&member, !off)?;
            (member, off, "a librarian")
        }
        AdminCmd::Reviewer { member, off } => {
            t.admin_set_reviewer(&member, !off)?;
            (member, off, "a reviewer")
        }
        AdminCmd::AdminAgent { member, off } => {
            t.admin_set_admin_agent(&member, !off)?;
            (member, off, "the admin agent")
        }
        _ => unreachable!("not a member act"),
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

/// SECS, or `--off` alone; anything else is a usage error.
fn secs_or_off(secs: Option<i64>, off: bool, name: &str) -> Result<Option<i64>> {
    match (off, secs) {
        (true, None) => Ok(None),
        (false, Some(s)) => Ok(Some(s)),
        _ => bail!("give SECS ({name} 600), or --off alone"),
    }
}

fn policy(t: &dyn Transport, cmd: AdminCmd) -> Result<()> {
    match cmd {
        AdminCmd::RoomCreation { policy } => {
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
        }
        AdminCmd::RateLimit { posts, secs, off } => {
            let limit = rate_limit_of(posts, secs, off)?;
            t.admin_set_rate_limit(limit)?;
            match limit {
                Some((p, s)) => {
                    println!("each member may post {p} times in {s} seconds in all rooms")
                }
                None => println!("no rate limit"),
            }
        }
        AdminCmd::LeaseCap { secs, off } => {
            let secs = secs_or_off(secs, off, "lease-cap")?;
            t.admin_set_lease_cap(secs)?;
            match secs {
                Some(s) => {
                    println!("a busy holder with no progress keeps its claims for {s} seconds")
                }
                None => println!("the lease cap is back to its default"),
            }
        }
        AdminCmd::LeaseGrace { secs, off } => {
            let secs = secs_or_off(secs, off, "lease-grace")?;
            t.admin_set_lease_grace(secs)?;
            match secs {
                Some(s) => println!("the claims of a holder found gone linger {s} seconds"),
                None => println!("the lease grace is back to its default"),
            }
        }
        AdminCmd::Cap { label } => {
            t.admin_set_cap(&label)?;
            println!("the agent cap is {label}");
        }
        AdminCmd::Chain { levels } => {
            let names: Vec<&str> = levels.split(',').map(str::trim).collect();
            t.admin_set_label_chain(&names)?;
            println!(
                "label order is {}; the agent cap is back to its default",
                names.join(" < ")
            );
        }
        _ => return scrub(t, cmd),
    }
    Ok(())
}

fn scrub(t: &dyn Transport, cmd: AdminCmd) -> Result<()> {
    match cmd {
        AdminCmd::ScrubAdd { name, term } => {
            t.admin_add_scrub_term(&name, &term.join(" "))?;
            println!("rule {name} now refuses its term in rooms below the agent cap");
        }
        AdminCmd::ScrubRemove { name } => {
            t.admin_remove_scrub_term(&name)?;
            println!("rule {name} removed");
        }
        AdminCmd::ScrubList => {
            for (name, term) in t.admin_scrub_terms()? {
                println!("{name}\t{}", flat(&term));
            }
        }
        _ => unreachable!("not a policy act"),
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
