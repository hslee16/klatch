//! `klatch act` and `klatch review` (#506, #509): how a member that holds a role calls what the role allows, as
//! itself. They are not `klatch admin`, which stays the human's (a real terminal, no agent above it): these run as the
//! caller's own member name and the gateway refuses anyone without the role, so the command grants nothing. Only the
//! acts below exist, so no floor act (cap, label order, scrub list) has a form here.

use super::*;
use clap::Subcommand;
use klatch::domain::Verdict;

#[cfg(test)]
mod command_tests;

#[derive(Subcommand)]
pub enum ActCmd {
    /// Make a room read-only (admin agent only)
    Close { room: String },
    /// Open a closed room again (admin agent only)
    Reopen { room: String },
    /// Take a member out of a room and refuse it there until `act allow` (admin agent only)
    Remove {
        member: String,
        #[arg(long)]
        room: String,
    },
    /// Lift a removal (admin agent only)
    Allow {
        member: String,
        #[arg(long)]
        room: String,
    },
    /// Set a room's label (admin agent only)
    Label { room: String, label: String },
    /// Set a member's clearance, at most the agent cap (admin agent only)
    Clear { member: String, label: String },
    /// Open a run: its rooms, `run.<ID>.<name>`, may be used from now on (admin agent only)
    RunCreate { run: String },
    /// End a run and delete its rooms and messages, with no archive: a role names no file for the server to write
    /// (admin agent only)
    RunEnd { run: String },
    /// Report a work item done (an anchor such as `gh:pr:OWNER/REPO/N`) and release the claims anchored to it (admin
    /// agent only)
    AnchorDone { anchor: String },
    /// Remove the members not seen for the fixed retention of 24 hours; every member may, as at session start
    Prune,
    /// Show the latest administrative acts (admin agent or librarian)
    Audit {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show the latest refused calls (admin agent or librarian)
    Refusals {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum ReviewCmd {
    /// Show the oldest waiting item you may read, with its text (reviewer only)
    Next,
    /// Record a final verdict on a waiting item (reviewer only)
    Verdict {
        id: i64,
        #[command(subcommand)]
        verdict: VerdictCmd,
    },
}

#[derive(Subcommand)]
pub enum VerdictCmd {
    /// Let the message through as it is
    Allow,
    /// Let it through with this text in its place
    Redact { text: Vec<String> },
    /// Refuse it, with a reason
    Deny { reason: Vec<String> },
}

pub(crate) fn act(t: &dyn Transport, cmd: ActCmd) -> Result<()> {
    match cmd {
        ActCmd::RunCreate { run } => t.admin_run_create(&run)?,
        ActCmd::RunEnd { run } => {
            let gone = t.admin_run_end(&run)?;
            println!(
                "{} room(s), {} message(s) removed",
                gone.rooms, gone.messages
            );
            return Ok(());
        }
        ActCmd::AnchorDone { anchor } => {
            println!("released {} claim(s)", t.anchor_done(&anchor)?);
            return Ok(());
        }
        ActCmd::Prune => {
            let removed = t.prune_members(klatch::domain::MEMBER_RETENTION_SECS)?;
            println!("removed {removed} member(s)");
            return Ok(());
        }
        ActCmd::Close { room } => t.admin_close_room(&room, true)?,
        ActCmd::Reopen { room } => t.admin_close_room(&room, false)?,
        ActCmd::Remove { member, room } => t.admin_remove_member(&room, &member)?,
        ActCmd::Allow { member, room } => t.admin_allow_member(&room, &member)?,
        ActCmd::Label { room, label } => t.admin_set_room_label(&room, &label)?,
        ActCmd::Clear { member, label } => t.admin_set_clearance(&member, &label)?,
        ActCmd::Audit { limit } => return audit(t, limit),
        ActCmd::Refusals { limit } => return refusals(t, limit),
    }
    println!("done");
    Ok(())
}

fn audit(t: &dyn Transport, limit: usize) -> Result<()> {
    for line in audit_listing(t, limit)? {
        println!("{line}");
    }
    Ok(())
}

/// The lines of the audit listing, newest first, the operational `forwarded-ignored` notes left out and, when there
/// were any, one last line that says how many (#1650, as the human's `klatch admin audit` does, #1490).
fn audit_listing(t: &dyn Transport, limit: usize) -> Result<Vec<String>> {
    let now = now();
    let page = t.admin_audit_page(limit, None)?;
    let mut lines: Vec<String> = page
        .rows
        .iter()
        .map(|a| {
            format!(
                "{}s ago\t{}\t{}\t{}",
                klatch::domain::secs_between(now, a.ts),
                a.action,
                flat(&a.target),
                flat(&a.detail)
            )
        })
        .collect();
    if page.hidden > 0 {
        let action = klatch::store::FORWARDED_IGNORED_ACTION;
        lines.push(format!(
            "{} {action} rows hidden (operational notes; the human's `klatch admin audit --action {action}` shows them)",
            page.hidden
        ));
    }
    Ok(lines)
}

fn refusals(t: &dyn Transport, limit: usize) -> Result<()> {
    let now = now();
    for r in t.admin_refusals(limit)? {
        println!(
            "{}s ago\t{}\t{}\t{}\t{}",
            klatch::domain::secs_between(now, r.ts),
            flat(&r.member),
            flat(&r.room),
            r.operation,
            r.rule
        );
    }
    Ok(())
}

pub(crate) fn review(t: &dyn Transport, cmd: ReviewCmd) -> Result<()> {
    match cmd {
        ReviewCmd::Next => match t.review_next()? {
            None => println!("no review waiting"),
            Some(i) => {
                println!(
                    "review {}\troom {}\tseq {}\tto {}\t{}",
                    i.id,
                    flat(&i.room),
                    i.seq,
                    flat(&i.to_label),
                    flat(&i.reason)
                );
                println!("{}", i.text);
            }
        },
        ReviewCmd::Verdict { id, verdict } => {
            let v = match verdict {
                VerdictCmd::Allow => Verdict::Allow,
                VerdictCmd::Redact { text } => Verdict::Redact(text.join(" ")),
                VerdictCmd::Deny { reason } => Verdict::Deny(reason.join(" ")),
            };
            t.review_verdict(id, &v)?;
            println!("verdict recorded");
        }
    }
    Ok(())
}
