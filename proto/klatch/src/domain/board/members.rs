//! Members on the board and in `who`: status, place, liveness, and the notes an asker or poster is shown about who
//! can be reached.

use super::{BOARD_LIST_LIMIT, Board, Now, more, section};
use crate::domain::hook::clip;
use crate::domain::{MemberInfo, MemberKind, MemberPlace, Message, Parked, flat};

/// A member not seen for this long is counted on the board, not listed.
pub const MEMBER_ACTIVE_SECS: i64 = 600;
/// Members not seen for this long are removed when a session starts (their messages stay).
pub const MEMBER_RETENTION_SECS: i64 = 86_400;
/// A message starting with this text sets its sender's status.
pub const STATUS_PREFIX: &str = "status:";
/// The latest status a member announced (`status: <text>`), with the text after the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatus {
    pub member: String,
    pub status: String,
}

/// A room member as the board shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardMember {
    pub name: String,
    pub last_seen: i64,
    /// The member's latest announced status text, if it ever announced one.
    pub status: Option<String>,
    /// When the status was set; 0 when unknown (#260).
    pub status_ts: i64,
    /// The member whose session started this one, if known.
    pub parent: Option<String>,
    /// A declared long wait that still holds.
    pub parked: Option<Parked>,
    /// What sort of member this is (item 103), if known.
    pub kind: Option<MemberKind>,
    /// Where the member was observed (#181), for display only.
    pub place: Option<MemberPlace>,
    /// Admitted in flag mode of a room-require (#878), so its harness lacks a capability the room asked for.
    pub advisory: bool,
}

/// The last component of a directory path, at most 40 characters, or `None` when there is none (the root, empty).
/// This is all of a directory that `who` and the board show.
pub fn short_dir_name(dir: &str) -> Option<String> {
    let name = dir.trim_end_matches('/').rsplit('/').next()?;
    (!name.is_empty()).then(|| name.chars().take(40).collect())
}

/// The fixed marker `who` and the board show when a member's declared scope names another repository than the one it
/// runs in (#894).
pub const SCOPE_MISMATCH_MARK: &str = "scope!=repo";

/// ` repo: ..., dir: ...` pieces of a member's place, tab separated, empty when it has none.
pub(super) fn place_parts(place: Option<&MemberPlace>) -> Vec<String> {
    let Some(p) = place else { return Vec::new() };
    let mut parts = Vec::new();
    if let Some(r) = &p.repository {
        parts.push(format!("repo: {}", flat(r)));
    }
    if let Some(d) = &p.directory {
        parts.push(format!("dir: {}", flat(d)));
    }
    if p.scope_differs {
        parts.push(SCOPE_MISMATCH_MARK.to_string());
    }
    parts
}

/// `klatch who` as text: each member with last seen and status. Unless `all`, members not seen for
/// `MEMBER_ACTIVE_SECS` are counted in one line instead of listed, as on the board.
pub fn render_who(members: &[MemberInfo], now: i64, all: bool) -> String {
    let now = Now(now);
    let mut out = String::new();
    let mut gone = 0;
    for m in members {
        if !all && is_stale(m, now) {
            gone += 1;
            continue;
        }
        out.push_str(&who_line(m, now));
        out.push('\n');
    }
    match gone {
        0 => {}
        1 => out.push_str("1 other not seen for over 10 minutes (who --all lists them)\n"),
        n => out.push_str(&format!(
            "{n} others not seen for over 10 minutes (who --all lists them)\n"
        )),
    }
    out
}

/// Not seen for over `MEMBER_ACTIVE_SECS` and not holding a park.
fn is_stale(m: &MemberInfo, now: Now) -> bool {
    now.0 - m.last_seen > MEMBER_ACTIVE_SECS && !holds_park(m.parked.as_ref(), now)
}

/// One member's `who` line, without the trailing newline.
fn who_line(m: &MemberInfo, now: Now) -> String {
    let mut out = format!(
        "{}\tlast seen {}s ago (liveness, not progress)",
        flat(&m.name),
        now.since(m.last_seen)
    );
    if let Some(parent) = &m.parent {
        out.push_str(&format!("\tchild of {}", flat(parent)));
    }
    if let Some(kind) = m.kind {
        out.push_str(&format!("\tkind: {}", kind.word()));
    }
    for part in place_parts(m.place.as_ref()) {
        out.push_str(&format!("\t{part}"));
    }
    if let Some(status) = &m.status {
        out.push_str(&format!(
            "\tstatus: {}",
            status_with_age(&flat(status), m.status_ts, now.0)
        ));
    }
    if let Some(p) = m.parked.as_ref().filter(|p| p.holds(now.0)) {
        out.push_str(&format!("\t{}", flat(&p.phrase(now.0))));
    }
    if m.advisory {
        out.push_str("\tadvisory: harness lacks a capability this room asked for");
    }
    out.push_str(&format!(
        "\tmode: {} (wake: {})",
        m.mode.as_deref().map_or("unknown".into(), flat),
        crate::domain::wake_path(m.mode.as_deref()).word()
    ));
    out
}

/// How long ago, in the largest whole unit: `59s`, `12m`, `3h`, `2d`.
pub fn age_phrase(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// A status with the time since it was set, `done (12m ago)`; the text alone when the time is unknown (0).
pub fn status_with_age(status: &str, set_at: i64, now: i64) -> String {
    if set_at > 0 {
        format!("{status} ({} ago)", age_phrase(now - set_at))
    } else {
        status.to_string()
    }
}

/// A parked member whose return time has not come stays listed however long since it was seen.
fn holds_park(parked: Option<&Parked>, now: Now) -> bool {
    parked.is_some_and(|p| p.holds(now.0))
}

/// The `members:` section.
pub(super) fn render(board: &Board, now: Now, out: &mut String) {
    section(out, "members:", None);
    for m in board.members.iter().take(BOARD_LIST_LIMIT) {
        let status = m
            .status
            .as_deref()
            .map(|s| {
                format!(
                    ", status: {}",
                    clip(&status_with_age(s, m.status_ts, now.0))
                )
            })
            .unwrap_or_default();
        let parked = m
            .parked
            .as_ref()
            .filter(|p| p.holds(now.0))
            .map(|p| format!(", {}", clip(&p.phrase(now.0))))
            .unwrap_or_default();
        let parent = m
            .parent
            .as_deref()
            .map(|p| format!(", child of {}", flat(p)))
            .unwrap_or_default();
        let kind = m
            .kind
            .map(|k| format!(", kind: {}", k.word()))
            .unwrap_or_default();
        let place: String = place_parts(m.place.as_ref())
            .iter()
            .map(|p| format!(", {p}"))
            .collect();
        let advisory = if m.advisory {
            ", advisory (harness lacks a capability this room asked for)"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {} last seen {}s ago{parent}{kind}{place}{status}{parked}{advisory}\n",
            flat(&m.name),
            now.since(m.last_seen)
        ));
    }
    more(out, board.members.len(), "members");
    match board.stale_members {
        0 => {}
        1 => out.push_str("  1 other not seen for over 10 minutes\n"),
        n => out.push_str(&format!("  {n} others not seen for over 10 minutes\n")),
    }
}

/// The text after `status:` in a status message, trimmed.
pub(super) fn status_text(body: &str) -> Option<&str> {
    body.strip_prefix(STATUS_PREFIX).map(str::trim)
}

/// Each member's latest announced status, in order of the member's first status message.
pub fn statuses(messages: &[Message]) -> Vec<MemberStatus> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut out: Vec<MemberStatus> = Vec::new();
    for m in ordered {
        let Some(text) = status_text(&m.body) else {
            continue;
        };
        match out.iter_mut().find(|s| s.member == m.sender) {
            Some(slot) => slot.status = text.to_string(),
            None => out.push(MemberStatus {
                member: m.sender.clone(),
                status: text.to_string(),
            }),
        }
    }
    out
}

/// What an asker should be told right after posting an ask: whether anyone could be reading it (#224). Option A of
/// the ticket made automatic, because the asker that did not check waited minutes for a helper that had left. A
/// member counts as reachable when it is not the asker, has not said it is done, and was seen within
/// `MEMBER_ACTIVE_SECS` or holds a declared wait. `None` when someone is reachable (the common case prints nothing).
pub fn ask_reach_note(members: &[MemberInfo], me: &str, now: i64) -> Option<String> {
    let reachable = members.iter().filter(|m| m.name != me).any(|m| {
        let done = m.status.as_deref().is_some_and(|s| s.starts_with("done"));
        !done
            && (now - m.last_seen <= MEMBER_ACTIVE_SECS || holds_park(m.parked.as_ref(), Now(now)))
    });
    (!reachable).then(|| {
        "klatch: no other member of this room has been seen in the last 10 minutes (or all said done). Your ask is \
         saved and a helper that joins later sees it at session start, but do not wait on it: run `klatch board` to \
         see who is around, or tell your human what is missing."
            .to_string()
    })
}

/// What a poster should be told when a member that is present gets no socket nudge because its last reported
/// permission mode bypasses prompts (#677): the nudge would be held and dropped, so it reaches that member through its
/// hooks and the wake watcher. Names only members seen within `MEMBER_ACTIVE_SECS`, never the poster, and never a
/// member whose mode is merely unknown (that would be noise for every member whose hooks have not run).
pub fn nudge_skipped_note(members: &[MemberInfo], me: &str, now: i64) -> Option<String> {
    let skipped: Vec<String> = members
        .iter()
        .filter(|m| m.name != me && now - m.last_seen <= MEMBER_ACTIVE_SECS)
        .filter(|m| m.mode.as_deref() == Some("bypassPermissions"))
        .map(|m| flat(&m.name))
        .collect();
    (!skipped.is_empty()).then(|| {
        format!(
            "klatch: no socket nudge for {} (permission mode bypassPermissions: Claude Code holds and drops it); it gets \
             this from its hooks and the wake watcher (klatch init --wake), so it may take until its next prompt or \
             watcher tick.",
            skipped.join(", ")
        )
    })
}
