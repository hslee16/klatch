//! Work commands: claims, scopes and the board.

use super::*;

/// Claims, scopes and the board: the commands about who is working on what.
pub(super) fn run_work(ctx: &Ctx, cmd: WorkCmd) -> Result<()> {
    let t = ctx.t;
    match cmd {
        WorkCmd::Claim {
            room,
            path,
            more,
            ttl,
            reason,
            shared,
            anchor,
        } => claim(
            ctx,
            &ClaimArgs {
                room,
                path: claim_target(path, &more)?,
                reason,
                ttl,
                shared,
                anchor,
            },
        )?,
        WorkCmd::Scope { scope } => scope_cmd(t, scope)?,
        WorkCmd::Role => println!("{}", t.my_role()?.as_deref().unwrap_or("none")),
        WorkCmd::Release { room, path, note } => release(t, &room, &path, note.as_deref())?,
        WorkCmd::Cross { room, seq, to } => {
            let id = t.request_crossing(&room, seq, &to)?;
            println!("crossing {id} queued: {room}#{seq} to {to}, waiting for a verdict");
        }
        WorkCmd::Crossings { limit } => {
            let list = t.my_crossings(limit)?;
            if list.is_empty() {
                println!("no crossings asked for by {}", ctx.me);
            }
            for c in list {
                println!(
                    "{}\t{}#{} to {}\t{}\t{}",
                    c.id,
                    flat(&c.room),
                    c.seq,
                    flat(&c.to_room),
                    c.state,
                    c.why
                );
            }
        }
        WorkCmd::Board { room } => for_rooms(t, room, |room| {
            print!("{}", render_board(&t.board(room)?, ctx.now));
            Ok(())
        })?,
        WorkCmd::Card { room, member } => card(ctx, &room, member.as_deref())?,
        WorkCmd::Asks { room } => for_rooms(t, room, |room| asks(ctx, room))?,
        WorkCmd::Stuck { room } => for_rooms(t, room, |room| {
            for e in t.stuck(room)? {
                println!("{}", e.line());
            }
            Ok(())
        })?,
        WorkCmd::CouldAnswer { room, ask, member } => {
            let c = t.could_answer(&room, member.as_deref().unwrap_or(ctx.me), ask)?;
            if c.could() {
                println!("yes");
            } else {
                println!("no");
                for b in &c.blockers {
                    println!("{}: {}", b.rule, flat(&b.reason));
                }
            }
            // what the ask points at (#1318); no verdict depends on it
            for line in klatch::domain::ref_lines(&c.refs, false) {
                println!("{line}");
            }
        }
        WorkCmd::Tasks { room } => {
            for task in t.board(&room)?.tasks {
                println!("{}", task.line(ctx.now, true));
            }
        }
        WorkCmd::Claims { room } => for_rooms(t, room, |room| claims(ctx, room))?,
        WorkCmd::Refusals { limit } => my_refusals(ctx, limit)?,
    }
    Ok(())
}

/// What `klatch claim` was asked.
pub(super) struct ClaimArgs {
    pub(super) room: String,
    pub(super) path: String,
    pub(super) reason: String,
    pub(super) ttl: Option<i64>,
    pub(super) shared: bool,
    pub(super) anchor: Option<String>,
}

/// Nouns that are never a target on their own: `claim lobby ticket #602` reaches us as just `ticket` because the
/// shell drops the comment, and would conflict every worker on the word (#610).
pub(super) const BARE_NOUNS: [&str; 6] = klatch::domain::WORK_ITEM_NOUNS;

/// The one target of a claim. Extra words and a bare noun are refused (never truncated, never joined: a joined
/// "ticket 602" would still not match another member's `ticket:602`) with the form that works.
pub(super) fn claim_target(path: String, more: &[String]) -> Result<String> {
    let noun = path.to_lowercase();
    if !more.is_empty() || BARE_NOUNS.contains(&noun.as_str()) {
        let words = std::iter::once(path.as_str())
            .chain(more.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        bail!(
            "a claim names one target, and `{words}` is more than one word or only a noun. Write it as one word, like `ticket:602`, or quote a path that has spaces: `\"my dir/file.md\"`. (A `#` starts a shell comment, so `ticket #602` is just `ticket`.)"
        );
    }
    Ok(path)
}

pub(super) fn claim(ctx: &Ctx, a: &ClaimArgs) -> Result<()> {
    let (room, path) = (a.room.as_str(), a.path.as_str());
    // the default lease of an `ask:` claim is the room's setting (#242)
    let ttl = ctx.t.claim_ttl(room, path, a.ttl)?;
    // what the last holder said it changed, read before this claim so it is the earlier holder's
    let earlier = ctx.t.last_release(room, path).ok().flatten();
    let mut spec = ClaimSpec::new(path, ttl, &a.reason).shared(a.shared);
    if let Some(anchor) = a.anchor.as_deref() {
        spec = spec.anchored(anchor);
    }
    ctx.t.claim(room, &spec)?;
    println!("claimed {path} for {ttl}s (advisory)");
    if let Some(n) = earlier.filter(|n| n.owner != ctx.me) {
        println!(
            "last released by {} {}s ago: {} (re-read it before you edit)",
            flat(&n.owner),
            klatch::domain::secs_between(ctx.now, n.ts),
            flat(&n.note)
        );
    }
    Ok(())
}

pub(super) fn scope_cmd(t: &dyn Transport, scope: Option<String>) -> Result<()> {
    match scope {
        Some(s) => {
            t.set_scope(&s)?;
            println!("scope set to {s}");
        }
        None => match t.scope()? {
            Some(s) => println!("{s}"),
            None => println!("no scope (set one with `klatch scope <name>`)"),
        },
    }
    Ok(())
}

/// `klatch card ROOM [MEMBER]`: one member's card (#251). A member that is not on the board has no card, said in
/// words and as a failure so a script can tell.
pub(super) fn card(ctx: &Ctx, room: &str, member: Option<&str>) -> Result<()> {
    let name = member.unwrap_or(ctx.me);
    match ctx.t.member_card(room, name)? {
        Some(card) => {
            print!("{}", klatch::domain::render_card(&card, room, ctx.now));
            Ok(())
        }
        None => bail!(
            "{} has no card in {}: not on its board (not in the room, or not seen for over 10 minutes)",
            flat(name),
            flat(room)
        ),
    }
}

pub(super) fn asks(ctx: &Ctx, room: &str) -> Result<()> {
    let open = ctx.t.open_asks(room)?;
    for a in &open {
        let summary = a.summary(ctx.now);
        let gap = if summary.is_empty() { "" } else { " " };
        let state = if a.claimed { "claimed" } else { "unclaimed" };
        println!(
            "#{} {} {state} {}s{gap}{summary}: {}",
            a.seq,
            a.sender,
            klatch::domain::secs_between(ctx.now, a.ts),
            flat(&a.text)
        );
        // what the ask points at (#1318): inert text, one line each
        for line in klatch::domain::ref_lines(&a.refs, false) {
            println!("{line}");
        }
    }
    // a cycle through asks (#273): anyone can see it, and it names who yields
    let claims = ctx.t.claims(room).unwrap_or_default();
    for c in klatch::domain::ask_cycles(&open, &claims, ctx.now) {
        println!("ask cycle: {}; {} yields", c.describe(), c.yielder());
    }
    Ok(())
}

pub(super) fn claims(ctx: &Ctx, room: &str) -> Result<()> {
    for c in ctx.t.claims(room)? {
        let scope = if c.scope.is_empty() {
            String::new()
        } else {
            format!("\tscope: {}", c.scope)
        };
        println!(
            "{}\t{}\t{}s left\t{}{scope}",
            c.path,
            c.owner,
            klatch::domain::secs_between(c.expires, ctx.now),
            flat(&c.reason)
        );
    }
    Ok(())
}
