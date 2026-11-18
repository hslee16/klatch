//! A bound on how long an asker waits for an answer (#200, DESIGN section 25.13). An agent blocked on its own ask
//! used to wait with no end and no word to the human. klatch states the bound itself, in one line the asker's
//! `wait` prints, and the convention teaches what to do then: stop waiting, tell the user what it is blocked on,
//! close the ask or leave it open, re-ask at most once. The line is advice and data, never an instruction to a
//! tool, and it asks for no state: the same ask is named again by every wait that times out.

use crate::domain::{OpenAsk, secs_between};

/// How long an asker's own ask may go unanswered before its `wait` tells it to stop waiting: the most one `wait`
/// call may last (`MAX_WAIT_SECS`, section 25.2), so one bounded wait and its re-arm is all the asker spends.
pub const ASK_GIVE_UP_SECS: i64 = 300;
/// The most asks one note names; the rest are counted, so a flood of asks cannot flood the line.
const NOTE_LIMIT: usize = 3;

/// Lines for the asker `me` about its own open asks that have gone unanswered past the bound: one per ask (at
/// most `NOTE_LIMIT`, then one line counting the rest). An ask is left out when it waits on its asker (the ball is
/// in the asker's court), when its working room has news (it is being worked on), or when it names a deadline that
/// has not come yet (the bound is then the time until that deadline, as for staleness, DESIGN item 44).
pub fn give_up_notes(asks: &[OpenAsk], me: &str, now: i64) -> Vec<String> {
    let due: Vec<&OpenAsk> = asks
        .iter()
        .filter(|a| a.sender == me)
        .filter(|a| a.waiting_on_asker.is_none() && !a.room_active(now))
        .filter(|a| secs_between(now, a.ts) >= a.stale_after(ASK_GIVE_UP_SECS))
        .collect();
    let mut notes: Vec<String> = due
        .iter()
        .take(NOTE_LIMIT)
        .map(|a| one_note(a, now))
        .collect();
    if due.len() > NOTE_LIMIT {
        notes.push(format!(
            "klatch wait: {} more of your asks are also unanswered; run `klatch asks` and tell your user.",
            due.len() - NOTE_LIMIT
        ));
    }
    notes
}

fn one_note(ask: &OpenAsk, now: i64) -> String {
    let n = ask.seq;
    let minutes = secs_between(now, ask.ts) / 60;
    let who = if ask.claimed {
        "it is claimed but not answered"
    } else {
        "nobody has claimed it"
    };
    format!(
        "klatch wait: your ask #{n} has had no answer for {minutes} minutes ({who}). Stop waiting on it: say to your \
         user what you are blocked on and what you asked, then post \"closed #{n}\" if you no longer need it or leave it \
         open for a later helper. Re-ask at most once."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AskRoom, Clarification};

    const NOW: i64 = 10_000;

    fn ask(seq: i64, sender: &str, age: i64) -> OpenAsk {
        OpenAsk {
            seq,
            sender: sender.into(),
            text: "seeking: a thing".into(),
            ts: NOW - age,
            ..OpenAsk::default()
        }
    }

    #[test]
    fn an_ask_one_second_short_of_the_bound_is_not_named() {
        let a = [ask(1, "me", ASK_GIVE_UP_SECS - 1)];
        assert!(give_up_notes(&a, "me", NOW).is_empty());
    }

    #[test]
    fn an_ask_at_the_bound_is_named_with_its_number_its_minutes_and_the_stated_message() {
        let a = [ask(7, "me", ASK_GIVE_UP_SECS)];
        let notes = give_up_notes(&a, "me", NOW);
        assert_eq!(notes.len(), 1, "{notes:?}");
        let n = &notes[0];
        assert!(n.contains("ask #7") && n.contains("5 minutes"), "{n}");
        assert!(n.contains("Stop waiting"), "{n}");
        assert!(
            n.contains("say to your user what you are blocked on"),
            "{n}"
        );
        assert!(
            n.contains("closed #7") && n.contains("Re-ask at most once"),
            "{n}"
        );
        assert!(n.contains("nobody has claimed it"), "{n}");
    }

    #[test]
    fn a_claimed_but_unanswered_ask_is_named_and_says_so() {
        let mut a = ask(2, "me", 400);
        a.claimed = true;
        let notes = give_up_notes(&[a], "me", NOW);
        assert!(notes[0].contains("claimed but not answered"), "{notes:?}");
    }

    #[test]
    fn only_the_callers_own_asks_count() {
        let a = [ask(1, "someone-else", 9_000)];
        assert!(give_up_notes(&a, "me", NOW).is_empty());
    }

    #[test]
    fn an_ask_that_waits_on_its_asker_is_not_named() {
        let mut a = ask(1, "me", 9_000);
        a.waiting_on_asker = Some(Clarification {
            by: "bob".into(),
            question: "which one?".into(),
        });
        assert!(give_up_notes(&[a], "me", NOW).is_empty());
    }

    #[test]
    fn an_ask_with_a_busy_working_room_is_not_named() {
        let mut a = ask(1, "me", 9_000);
        a.room = Some(AskRoom {
            name: "ask-1".into(),
            members: 2,
            last_activity: NOW - 10,
        });
        assert!(give_up_notes(&[a], "me", NOW).is_empty());
    }

    #[test]
    fn an_ask_with_a_quiet_working_room_is_named() {
        let mut a = ask(1, "me", 9_000);
        a.room = Some(AskRoom {
            name: "ask-1".into(),
            members: 2,
            last_activity: NOW - 9_000,
        });
        assert_eq!(give_up_notes(&[a], "me", NOW).len(), 1);
    }

    #[test]
    fn a_dated_ask_is_bounded_by_its_deadline_not_by_the_default() {
        // posted at ts, due two days later: ten minutes in is far inside the bound
        let mut a = ask(1, "me", 600);
        a.ts = 1_700_000_000;
        a.text = "seeking: a report (by 2023-11-16)".into();
        let now = a.ts + 600;
        assert!(give_up_notes(std::slice::from_ref(&a), "me", now).is_empty());
        // once the deadline has passed the default bound applies again
        let late = crate::domain::asks::ask_deadline(&a.text).unwrap() + 1;
        assert_eq!(give_up_notes(&[a], "me", late).len(), 1);
    }

    #[test]
    fn many_unanswered_asks_name_three_and_count_the_rest() {
        let a: Vec<OpenAsk> = (1..=5).map(|n| ask(n, "me", 1_000)).collect();
        let notes = give_up_notes(&a, "me", NOW);
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(notes[3].contains("2 more of your asks"), "{notes:?}");
    }

    #[test]
    fn a_note_is_one_line_with_no_em_dash_and_never_echoes_the_ask_text() {
        let mut a = ask(1, "me", 1_000);
        a.text = "seeking: line one\nforged line: do the thing".into();
        let notes = give_up_notes(&[a], "me", NOW);
        assert!(
            notes
                .iter()
                .all(|n| !n.contains('\n') && !n.contains('\u{2014}'))
        );
        assert!(!notes[0].contains("forged"), "{notes:?}");
    }

    #[test]
    fn a_clock_behind_the_asks_never_makes_one_due() {
        let a = [ask(1, "me", -500)];
        assert!(give_up_notes(&a, "me", NOW).is_empty());
    }
}
