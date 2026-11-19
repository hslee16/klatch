//! A helper that waits on an asker that is gone (#226, DESIGN section 25.14). An asker that crashes, forgets or
//! never posts `status: done` left its helpers waiting to their cap with no word why. klatch now says it in the
//! helper's own `wait` timeout: the asker of an ask the helper holds or answered has not been seen for as long as
//! the board lets a member stay listed (`MEMBER_ACTIVE_SECS`), or is no longer in the room. The line is advice and
//! data: the helper stops waiting for that asker and tells its user if it needs one. Presence is the signal (a
//! member that waits or runs any command refreshes it, a parked member holds until its return time), so no clock
//! of its own and no state: the same asker is named by every wait that times out.

use crate::domain::{
    Claim, MEMBER_ACTIVE_SECS, MemberInfo, Message, OpenAsk, answer_target, is_ask, secs_between,
};

/// An asker a helper serves: the ask, who posted it, and whether the helper already answered it (otherwise it
/// holds the ask's claim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    pub ask: i64,
    pub asker: String,
    pub answered: bool,
}

/// The asks `me` serves, from the room's recent messages and its claims: every ask it answered (`answer to #N:`),
/// and every open ask whose `ask:<N>` claim it holds and that has not expired. The ask's number must be a message
/// in `messages` (an ask older than the window is left out), posted by someone else.
pub fn served_by(
    messages: &[Message],
    (open, claims): (&[OpenAsk], &[Claim]),
    (me, now): (&str, i64),
) -> Vec<Served> {
    let answered = messages
        .iter()
        .filter(|m| m.sender == me)
        .filter_map(|m| answer_target(&m.body))
        .map(|n| (n, true));
    let held = claims
        .iter()
        .filter(|c| c.owner == me && c.expires > now)
        .filter_map(|c| ask_number(&c.path))
        .filter(|n| open.iter().any(|a| a.seq == *n))
        .map(|n| (n, false));
    let mut out: Vec<Served> = Vec::new();
    for (ask, answered) in answered.chain(held) {
        let known = out.iter().any(|s| s.ask == ask);
        if let Some(asker) = asker_of(messages, ask, me).filter(|_| !known) {
            out.push(Served {
                ask,
                asker,
                answered,
            });
        }
    }
    out
}

/// The number in an `ask:<N>` claim path.
fn ask_number(path: &str) -> Option<i64> {
    path.strip_prefix("ask:")?.parse().ok()
}

/// Who posted ask `n` among `messages`, if it is there and is not `me`.
fn asker_of(messages: &[Message], n: i64, me: &str) -> Option<String> {
    messages
        .iter()
        .find(|m| m.seq == n && is_ask(&m.body) && m.sender != me)
        .map(|m| m.sender.clone())
}

/// One line per served asker that is gone, at most three and then a count. Gone is: not in the room any more;
/// its process has exited (`exited` names the askers the liveness probe found gone, #790); not seen for
/// `MEMBER_ACTIVE_SECS` and not parked; or, for an ask the helper still holds, said `done` (an ask the helper
/// already answered treats `done` as the clean ending and is not named). Askers are named once however many asks of
/// theirs the helper serves.
pub fn gone_asker_notes(
    served: &[Served],
    members: &[MemberInfo],
    exited: &[String],
    now: i64,
) -> Vec<String> {
    let mut gone: Vec<(String, String)> = Vec::new();
    for s in served {
        if gone.iter().any(|(who, _)| *who == s.asker) {
            continue;
        }
        if let Some(why) = why_gone(s, members, exited, now) {
            gone.push((s.asker.clone(), why));
        }
    }
    let mut notes: Vec<String> = gone
        .iter()
        .take(3)
        .map(|(who, why)| {
            format!(
                "klatch wait: {who} {why}. Stop waiting for it: post your answer to the room anyway (a later reader \
                 sees it), release any ask claim you hold for it, and tell your user if the work cannot finish without it."
            )
        })
        .collect();
    if gone.len() > 3 {
        notes.push(format!(
            "klatch wait: {} more askers you serve are also gone; run `klatch who` and tell your user.",
            gone.len() - 3
        ));
    }
    notes
}

/// Only the askers whose process is known to have exited (#790): the check a waiting helper runs every few seconds,
/// where presence alone would take ten minutes. An asker gone for any other reason waits for the timeout's note.
pub fn exited_asker_notes(
    served: &[Served],
    members: &[MemberInfo],
    exited: &[String],
    now: i64,
) -> Vec<String> {
    let of_exited: Vec<Served> = served
        .iter()
        .filter(|s| exited.contains(&s.asker))
        .cloned()
        .collect();
    gone_asker_notes(&of_exited, members, exited, now)
}

fn why_gone(s: &Served, members: &[MemberInfo], exited: &[String], now: i64) -> Option<String> {
    let Some(m) = members.iter().find(|m| m.name == s.asker) else {
        return Some("is no longer in this room".into());
    };
    let done = m.status.as_deref().is_some_and(|t| t.starts_with("done"));
    if done {
        return (!s.answered).then(|| "said it is done".into());
    }
    if exited.contains(&s.asker) {
        return Some("has exited (its process is gone; it never said done)".into());
    }
    let parked = m.parked.as_ref().is_some_and(|p| p.holds(now));
    let quiet = secs_between(now, m.last_seen);
    (!parked && quiet >= MEMBER_ACTIVE_SECS).then(|| {
        format!(
            "has not been seen for {} minutes (it never said done)",
            quiet / 60
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Attribution, Parked};

    const NOW: i64 = 100_000;

    fn msg(seq: i64, sender: &str, body: &str) -> Message {
        Message {
            id: format!("id{seq}"),
            seq,
            sender: sender.into(),
            attribution: Attribution::Unattributed,
            label: "public".into(),
            body: body.into(),
            ts: NOW - 1_000,
            kind: None,
            payload: None,
        }
    }

    fn member(name: &str, quiet: i64) -> MemberInfo {
        MemberInfo {
            status_ts: 0,
            name: name.into(),
            joined: 0,
            last_seen: NOW - quiet,
            status: None,
            parent: None,
            parked: None,
            kind: None,
            place: None,
            mode: None,
        }
    }

    fn open(seq: i64) -> OpenAsk {
        OpenAsk {
            seq,
            ..OpenAsk::default()
        }
    }

    fn claim(path: &str, owner: &str, left: i64) -> Claim {
        Claim {
            path: path.into(),
            owner: owner.into(),
            reason: String::new(),
            expires: NOW + left,
            scope: String::new(),
        }
    }

    fn served(answered: bool) -> Vec<Served> {
        vec![Served {
            ask: 1,
            asker: "asker".into(),
            answered,
        }]
    }

    #[test]
    fn an_asker_seen_one_second_inside_the_window_is_not_gone() {
        let m = [member("asker", MEMBER_ACTIVE_SECS - 1)];
        assert!(gone_asker_notes(&served(true), &m, &[], NOW).is_empty());
    }

    #[test]
    fn an_asker_quiet_for_the_window_is_named_with_its_minutes_and_that_it_never_said_done() {
        let m = [member("asker", MEMBER_ACTIVE_SECS)];
        let notes = gone_asker_notes(&served(true), &m, &[], NOW);
        assert_eq!(notes.len(), 1, "{notes:?}");
        let n = &notes[0];
        assert!(n.contains("asker has not been seen for 10 minutes"), "{n}");
        assert!(n.contains("never said done"), "{n}");
        assert!(n.contains("Stop waiting for it"), "{n}");
        assert!(n.contains("tell your user"), "{n}");
    }

    #[test]
    fn an_asker_that_left_the_room_is_gone_at_once() {
        let notes = gone_asker_notes(&served(false), &[], &[], NOW);
        assert!(notes[0].contains("is no longer in this room"), "{notes:?}");
    }

    #[test]
    fn a_parked_asker_holds_until_its_return_time_then_is_gone() {
        let mut m = member("asker", 5_000);
        m.parked = Some(Parked {
            why: "a build".into(),
            back_by: NOW + 60,
        });
        assert!(gone_asker_notes(&served(true), &[m.clone()], &[], NOW).is_empty());
        assert_eq!(
            gone_asker_notes(&served(true), &[m], &[], NOW + 60).len(),
            1
        );
    }

    #[test]
    fn done_is_a_clean_ending_for_an_answered_ask_and_a_note_for_a_held_one() {
        let mut m = member("asker", 5_000);
        m.status = Some("done leaving".into());
        assert!(gone_asker_notes(&served(true), &[m.clone()], &[], NOW).is_empty());
        let held = gone_asker_notes(&served(false), &[m], &[], NOW);
        assert!(held[0].contains("said it is done"), "{held:?}");
    }

    #[test]
    fn an_asker_is_named_once_however_many_asks_of_it_are_served() {
        let two = vec![
            Served {
                ask: 1,
                asker: "asker".into(),
                answered: true,
            },
            Served {
                ask: 2,
                asker: "asker".into(),
                answered: false,
            },
        ];
        let notes = gone_asker_notes(&two, &[], &[], NOW);
        assert_eq!(notes.len(), 1, "{notes:?}");
    }

    #[test]
    fn many_gone_askers_name_three_and_count_the_rest() {
        let many: Vec<Served> = (1..=5)
            .map(|n| Served {
                ask: n,
                asker: format!("a{n}"),
                answered: true,
            })
            .collect();
        let notes = gone_asker_notes(&many, &[], &[], NOW);
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(notes[3].contains("2 more askers"), "{notes:?}");
    }

    #[test]
    fn a_note_has_no_em_dash_and_is_one_line() {
        let notes = gone_asker_notes(&served(true), &[], &[], NOW);
        assert!(
            notes
                .iter()
                .all(|n| !n.contains('\n') && !n.contains('\u{2014}'))
        );
    }

    #[test]
    fn a_clock_behind_the_last_seen_never_makes_an_asker_gone() {
        let m = [member("asker", -500)];
        assert!(gone_asker_notes(&served(true), &m, &[], NOW).is_empty());
    }

    #[test]
    fn served_is_every_answered_ask_and_every_held_open_ask_of_someone_else() {
        let messages = [
            msg(1, "asker", "seeking: one"),
            msg(2, "asker", "seeking: two"),
            msg(3, "me", "answer to #1: here"),
            msg(4, "me", "seeking: my own"),
            msg(5, "me", "answer to #4: mine"),
        ];
        let claims = [claim("ask:2", "me", 60)];
        let s = served_by(&messages, (&[open(2)], &claims), ("me", NOW));
        assert_eq!(
            s,
            vec![
                Served {
                    ask: 1,
                    asker: "asker".into(),
                    answered: true
                },
                Served {
                    ask: 2,
                    asker: "asker".into(),
                    answered: false
                },
            ]
        );
    }

    #[test]
    fn a_claim_that_expired_or_is_someone_elses_or_on_a_closed_ask_does_not_count() {
        let messages = [msg(2, "asker", "seeking: two")];
        let expired = [claim("ask:2", "me", 0)];
        assert!(served_by(&messages, (&[open(2)], &expired), ("me", NOW)).is_empty());
        let theirs = [claim("ask:2", "other", 60)];
        assert!(served_by(&messages, (&[open(2)], &theirs), ("me", NOW)).is_empty());
        let live = [claim("ask:2", "me", 60)];
        assert!(served_by(&messages, (&[], &live), ("me", NOW)).is_empty());
    }

    #[test]
    fn an_ask_outside_the_window_or_a_claim_on_a_file_is_left_out() {
        let messages = [msg(9, "me", "answer to #1: here")];
        assert!(served_by(&messages, (&[], &[]), ("me", NOW)).is_empty());
        let file = [claim("file:a.rs", "me", 60)];
        assert!(served_by(&messages, (&[open(1)], &file), ("me", NOW)).is_empty());
    }

    // #790: the liveness probe says an asker's process exited, in seconds, where presence takes ten minutes

    fn exited(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn an_asker_whose_process_exited_is_named_though_it_was_seen_a_second_ago() {
        let m = [member("asker", 1)];
        let notes = gone_asker_notes(&served(true), &m, &exited(&["asker"]), NOW);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("asker has exited (its process is gone; it never said done)"),
            "{notes:?}"
        );
        assert!(notes[0].contains("Stop waiting for it"), "{notes:?}");
    }

    #[test]
    fn an_asker_not_in_the_exited_list_is_judged_by_presence_as_before() {
        let m = [member("asker", 1)];
        assert!(gone_asker_notes(&served(true), &m, &exited(&["someone-else"]), NOW).is_empty());
        let quiet = [member("asker", MEMBER_ACTIVE_SECS)];
        let notes = gone_asker_notes(&served(true), &quiet, &[], NOW);
        assert!(
            notes[0].contains("has not been seen for 10 minutes"),
            "{notes:?}"
        );
    }

    #[test]
    fn done_after_an_answer_is_still_the_clean_ending_when_the_process_has_exited() {
        let mut m = member("asker", 1);
        m.status = Some("done thanks".into());
        assert!(gone_asker_notes(&served(true), &[m.clone()], &exited(&["asker"]), NOW).is_empty());
        let held = gone_asker_notes(&served(false), &[m], &exited(&["asker"]), NOW);
        assert!(
            held[0].contains("said it is done"),
            "done is the better reason: {held:?}"
        );
    }

    #[test]
    fn a_parked_asker_whose_process_exited_is_named_because_a_park_cannot_end_without_one() {
        let mut m = member("asker", 5_000);
        m.parked = Some(Parked {
            why: "a build".into(),
            back_by: NOW + 3_600,
        });
        assert!(gone_asker_notes(&served(true), &[m.clone()], &[], NOW).is_empty());
        let notes = gone_asker_notes(&served(true), &[m], &exited(&["asker"]), NOW);
        assert!(notes[0].contains("has exited"), "{notes:?}");
    }

    #[test]
    fn the_early_check_names_only_askers_whose_process_exited() {
        // quiet for twenty minutes but no word about its process: that waits for the timeout, not the early check
        let quiet = [member("asker", 1_200)];
        assert!(exited_asker_notes(&served(true), &quiet, &[], NOW).is_empty());
        let notes = exited_asker_notes(&served(true), &quiet, &exited(&["asker"]), NOW);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("has exited"), "{notes:?}");
    }

    #[test]
    fn the_early_check_says_nothing_for_an_asker_that_said_done_after_an_answer() {
        let mut m = member("asker", 1);
        m.status = Some("done".into());
        assert!(exited_asker_notes(&served(true), &[m], &exited(&["asker"]), NOW).is_empty());
    }
}
