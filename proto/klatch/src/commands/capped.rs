//! What `read` and `wait` print for a room, bounded (#1390): whole messages from the oldest unread up to a message count
//! and a byte count, a count of the rest with the command to continue, and only what was shown marked read.

use super::*;

pub(super) fn read(
    t: &dyn Transport,
    (me, now): (&str, i64),
    room: &str,
    peek: bool,
) -> Result<()> {
    use klatch::domain::budget::{READ_MAX_BYTES, READ_MAX_MESSAGES};
    // Always looked at first: what is marked read is only what was shown, and a bounded call shows a part (#1390).
    let messages = t.read(room, false)?;
    if messages.is_empty() {
        eprintln!("{EMPTY_READ_NOTE}");
    } else {
        eprintln!("{DATA_RULE}");
    }
    let shown = print_capped(
        room,
        &messages,
        (false, (READ_MAX_MESSAGES, READ_MAX_BYTES)),
    );
    let omitted = &messages[shown..];
    if !omitted.is_empty() {
        let for_you = klatch::domain::mentioning(omitted, me);
        let you = if for_you > 0 {
            format!(", {for_you} of them mention you")
        } else {
            String::new()
        };
        let next = if peek {
            // a peek moves nothing, so the same command would show the same messages again
            format!("klatch history {room} --since {}", messages[shown - 1].seq)
        } else {
            format!("klatch read {room}")
        };
        println!(
            "[{} more messages not shown{you}; run `{next}` for the next ones]",
            omitted.len()
        );
    }
    // a peek leaves the messages unread; otherwise only what was shown is marked read, after it was printed
    if !peek && let Some(last) = messages[..shown].last() {
        t.read_through(room, last.seq)?;
    }
    // a peek leaves the messages unread, so the hook digest still shows them and carries the help itself
    if !peek {
        for line in dm_help_lines(t, me, room, &messages[..shown]) {
            println!("{line}");
        }
        for line in asks_note_lines(t, (me, now), room, shown > 0, true) {
            println!("{line}");
        }
    }
    Ok(())
}

/// One message as `read` and `history` print it: marked, one line, handoff folded in.
pub(super) fn message_row(m: &klatch::domain::Message) -> String {
    row_in(None, m)
}

/// `message_row`, with the room named first when `room` is given (what `wait` prints, as it may watch several).
fn row_in(room: Option<&str>, m: &klatch::domain::Message) -> String {
    let room = room.map_or_else(String::new, |r| format!("{r} "));
    format!(
        "[{room}{} {} {}] {}{}{}{}",
        m.seq,
        m.ts,
        m.sender_label(),
        flat(&m.body),
        m.scope_mark(),
        m.refs_mark(),
        handoff_suffix(m)
    )
}

/// Print the first messages of `messages` that fit the cap `(messages, bytes)`, each with its reference lines, and
/// return how many were printed (#1390). A message is never split: it is shown whole or left for the next call. The
/// first is always shown, and when it alone is larger than the byte cap it is cut with a marker that says how many bytes
/// were cut and how to read all of it, so a very long message cannot block the ones behind it. `in_room` names the
/// room in each row (`wait`).
pub(super) fn print_capped(
    room: &str,
    messages: &[klatch::domain::Message],
    (in_room, (max_messages, max_bytes)): (bool, (usize, usize)),
) -> usize {
    let items: Vec<String> = messages
        .iter()
        .map(|m| {
            let mut item = row_in(in_room.then_some(room), m);
            for line in klatch::domain::ref_lines(&m.refs, true) {
                item.push('\n');
                item.push_str(&line);
            }
            item
        })
        .collect();
    let sizes: Vec<usize> = items.iter().map(String::len).collect();
    let shown = klatch::domain::fit_prefix(&sizes, max_messages, max_bytes);
    for (i, item) in items[..shown].iter().enumerate() {
        if i == 0 && item.len() + 1 > max_bytes {
            let since = messages[0].seq - 1;
            let cut = klatch::domain::cut_to(item, max_bytes - 1, |n| {
                format!(
                    " [truncated, {n} more bytes; run `klatch history {room} --since {since}` for all of it]"
                )
            });
            println!("{cut}");
        } else {
            println!("{item}");
        }
    }
    shown
}

/// What a wait found in one room, before anything is marked: the messages that woke it, and how far it looked.
pub(super) struct Looked {
    pub(super) found: Vec<klatch::domain::Message>,
    /// The newest seq it looked at, matching or not; what a peeking or filtered wait remembers when it shows all.
    pub(super) seen_to: Option<i64>,
}

/// Without a filter or `--peek` a wait consumes what it shows, like a blocking read. With either, it leaves
/// messages unread (a later read or the hook still shows them) and remembers what it has looked at so it does
/// not wake for the same message twice. Nothing is marked here: `print_woken` marks what it showed (#1390), so a
/// wait that shows a bounded part of a big room leaves the rest unread and unremembered.
pub(super) fn look(
    t: &dyn Transport,
    room: &str,
    filter: &klatch::domain::WaitFilter,
    peek: bool,
) -> Result<Looked> {
    if filter.is_open() && !peek {
        let found = t.read(room, false)?;
        let seen_to = found.last().map(|m| m.seq);
        return Ok(Looked { found, seen_to });
    }
    let seen = t.wait_poll(room)?;
    let seen_to = seen.iter().map(|m| m.seq).max();
    Ok(Looked {
        found: seen.into_iter().filter(|m| filter.accepts(m)).collect(),
        seen_to,
    })
}

/// Print the messages that woke a wait; true when there were any. Whole messages from the oldest, at most
/// `WAIT_MAX_MESSAGES` and `WAIT_MAX_BYTES` (#1390); the rest stay unread, are counted, and the next wait shows them.
pub(super) fn print_woken(t: &dyn Transport, a: &WaitArgs, waited: u64) -> Result<bool> {
    use klatch::domain::budget::{WAIT_MAX_BYTES, WAIT_MAX_MESSAGES};
    for (room, filter) in a.rooms {
        let Looked { found, seen_to } = look(t, room, filter, a.peek)?;
        let consuming = filter.is_open() && !a.peek;
        let shown = print_capped(room, &found, (true, (WAIT_MAX_MESSAGES, WAIT_MAX_BYTES)));
        let omitted = &found[shown..];
        if !omitted.is_empty() {
            let continue_with = if consuming {
                format!("klatch wait {room}")
            } else {
                "the same `klatch wait` again".to_string()
            };
            let for_you = klatch::domain::mentioning(omitted, a.caller.0);
            let you = if for_you > 0 {
                format!(", {for_you} of them mention you")
            } else {
                String::new()
            };
            let run = if continue_with.starts_with("klatch") {
                format!("run `{continue_with}`")
            } else {
                format!("run {continue_with}")
            };
            println!(
                "[{room}: {} more messages not shown{you}; {run} for the next ones]",
                omitted.len()
            );
        }
        // only what was shown is marked: a consuming wait moves the read cursor over it, a peeking or filtered one
        // remembers it (all it looked at when it showed every match)
        if let Some(last) = found[..shown].last().map(|m| m.seq) {
            if consuming {
                t.read_through(room, last)?;
            } else {
                let mark = if omitted.is_empty() {
                    seen_to.unwrap_or(last).max(last)
                } else {
                    last
                };
                t.wait_seen(room, mark)?;
            }
        } else if let Some(last) = seen_to {
            t.wait_seen(room, last)?;
        }
        // a wait that consumed what it returned hands the dm help over itself; a peek or a filter leaves the messages
        // unread, so the hook digest carries it (#1209)
        if filter.is_open() && !a.peek {
            for line in dm_help_lines(t, a.caller.0, room, &found[..shown]) {
                println!("{line}");
            }
            // the cycle was said before the wait began (#273); the rest is derived from the room now (#789)
            let now = a.caller.1 + waited as i64;
            for line in asks_note_lines(t, (a.caller.0, now), room, !found.is_empty(), false) {
                println!("{line}");
            }
        }
        if !found.is_empty() {
            eprintln!("{DATA_RULE}");
            return Ok(true);
        }
    }
    Ok(false)
}
