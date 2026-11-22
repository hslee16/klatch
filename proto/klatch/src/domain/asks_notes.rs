//! The three stop-waiting notes of the asks delivery work, derived from the room state each time they are asked for
//! (#789, DESIGN item 50 and 89): a helper whose asker is gone (#226, #790), an asker whose ask has gone past its
//! bound (#200), and the members of a cycle through asks (#273). The hook digest and the trailing lines of `wait` and
//! `read` call this, so an agent that never lets a wait time out still hears them.
//!
//! The rule is derived, with no table of what was reported: the same lines come back at every hook while the
//! condition holds, and none once it clears (the ask answered or closed, the claim released, the asker back). Each
//! line is a fixed template over validated member names, an ask number and a count, never message text. The count is
//! bounded: at most `NOTES_PER_KIND` lines of each kind and one line counting the rest.

use crate::domain::{
    Claim, MemberInfo, OpenAsk, Served, cycle_notes, give_up_notes, gone_asker_notes,
};

/// How many recent messages are read to find the asks a member answered (#226).
pub const SERVED_WINDOW: usize = 200;

/// The most lines of one kind; more are counted in one further line.
pub const NOTES_PER_KIND: usize = 3;

/// What the notes are derived from, for the member `me` at `now`.
pub struct AsksView<'a> {
    pub asks: &'a [OpenAsk],
    pub claims: &'a [Claim],
    pub served: &'a [Served],
    pub members: &'a [MemberInfo],
    /// The askers the liveness probe found gone (#790).
    pub exited: &'a [String],
    pub me: &'a str,
    pub now: i64,
    /// Whether to include the ask cycles: `wait` says them itself before it blocks, so its trailing lines leave them out.
    pub with_cycles: bool,
}

/// The notes for `view.me`: askers it serves that are gone, its own asks past the bound, then the ask cycles it is in.
/// Every line starts `klatch: `. Empty when nothing holds.
pub fn derived_asks_notes(view: &AsksView) -> Vec<String> {
    let gone = gone_asker_notes(view.served, view.members, view.exited, view.now);
    let due = give_up_notes(view.asks, view.me, view.now);
    let mut cycles = if view.with_cycles {
        cycle_notes(view.asks, view.claims, view.me, view.now)
    } else {
        Vec::new()
    };
    if cycles.len() > NOTES_PER_KIND {
        let more = cycles.len() - NOTES_PER_KIND;
        cycles.truncate(NOTES_PER_KIND);
        cycles.push(format!(
            "klatch: {more} more ask cycles you are in; run `klatch stuck` and tell your user."
        ));
    }
    gone.into_iter()
        .chain(due)
        .map(|n| match n.strip_prefix("klatch wait: ") {
            Some(rest) => format!("klatch: {rest}"),
            None => n,
        })
        .chain(cycles)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000;

    fn ask(seq: i64, sender: &str, age: i64, claimed: bool) -> OpenAsk {
        OpenAsk {
            seq,
            sender: sender.into(),
            text: "seeking: SECRET text".into(),
            ts: NOW - age,
            claimed,
            ..OpenAsk::default()
        }
    }

    fn claim(seq: i64, owner: &str) -> Claim {
        Claim {
            path: format!("ask:{seq}"),
            owner: owner.into(),
            reason: String::new(),
            expires: NOW + 60,
            scope: String::new(),
        }
    }

    fn view<'a>(asks: &'a [OpenAsk], claims: &'a [Claim], me: &'a str) -> AsksView<'a> {
        AsksView {
            asks,
            claims,
            served: &[],
            members: &[],
            exited: &[],
            me,
            now: NOW,
            with_cycles: true,
        }
    }

    #[test]
    fn nothing_holds_so_no_note() {
        assert!(derived_asks_notes(&view(&[], &[], "alice")).is_empty());
    }

    #[test]
    fn an_old_own_ask_is_a_note_that_repeats_and_never_carries_the_text() {
        let asks = [ask(7, "alice", 400, false)];
        let v = view(&asks, &[], "alice");
        let first = derived_asks_notes(&v);
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(first[0].starts_with("klatch: your ask #7 has had no answer"));
        assert!(!first[0].contains("SECRET"));
        assert_eq!(derived_asks_notes(&v), first, "same lines every time");
        let young = [ask(7, "alice", 60, false)];
        assert!(derived_asks_notes(&view(&young, &[], "alice")).is_empty());
    }

    #[test]
    fn a_cycle_names_its_members_and_clears_when_a_claim_is_released() {
        let asks = [ask(1, "bob", 5, true), ask(2, "alice", 5, true)];
        let held = [claim(1, "alice"), claim(2, "bob")];
        let notes = derived_asks_notes(&view(&asks, &held, "alice"));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("klatch: ask cycle:"), "{notes:?}");
        assert!(derived_asks_notes(&view(&asks, &held[..1], "alice")).is_empty());
    }

    #[test]
    fn many_asks_stay_bounded() {
        let asks: Vec<OpenAsk> = (1..=40).map(|n| ask(n, "alice", 400, false)).collect();
        let notes = derived_asks_notes(&view(&asks, &[], "alice"));
        assert_eq!(notes.len(), NOTES_PER_KIND + 1, "{notes:?}");
        assert!(notes[NOTES_PER_KIND].contains("37 more of your asks"));
    }
}
