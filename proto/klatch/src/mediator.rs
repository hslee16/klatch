//! The reference rules-only mediator member (#271, DESIGN section 24, Q42): one pass reads the lobby's stuck-case
//! events through the caller's own `Transport` (the same guarded gateway calls a normal member's commands make, as that
//! member and no one else), asks `domain::plan_proposals` what to propose, then posts each proposal and, for a cycle
//! through asks, creates a room and invites the members into it.
//!
//! It only proposes. It holds no capability a normal member lacks beyond the two the human grants one by one, creating
//! a room and inviting (#188, `klatch admin grant-rooms`), and both go through the guard-checked, audited `join` and
//! `invite`. It has no admin act; it cancels, releases, grants and clears nothing and sets no label or clearance. Its
//! posts are fixed templates filled with numbers and member names (`domain/mediator.rs`), so its output is data like
//! any message and no wording of an ask reaches it. A refusal is reported and not retried in the same run, so a
//! refused mediator does not fill the refusal record.

use crate::domain::budget::HISTORY_MAX_MESSAGES;
use crate::domain::{MediatorView, Result, plan_proposals};
use crate::gateway::{HistoryRange, Transport};
use std::collections::HashSet;

/// What one pass did, one line each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A proposal was posted for the stuck case with this key.
    Proposed {
        key: String,
    },
    Invited {
        member: String,
        room: String,
    },
    /// An act the guard or the gateway refused, with its reason.
    Refused {
        what: String,
        reason: String,
    },
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Proposed { key } => write!(f, "proposed for {key}"),
            Event::Invited { member, room } => write!(f, "invited {member} to {room}"),
            Event::Refused { what, reason } => write!(f, "refused: {what}: {reason}"),
        }
    }
}

/// The mediator: the room it watches and what it already did (so a pass is idempotent within a run).
pub struct Mediator {
    pub me: String,
    pub lobby: String,
    done: HashSet<String>,
    /// Whether `done` has been seeded from the mediator's own earlier posts (once per run, on the first pass).
    seeded: bool,
}

impl Mediator {
    pub fn new(me: &str, lobby: &str) -> Mediator {
        Mediator {
            me: me.into(),
            lobby: lobby.into(),
            done: HashSet::new(),
            seeded: false,
        }
    }

    /// One pass. An error is only a failure to read the lobby; a refused act is an `Event::Refused`.
    pub fn pass(&mut self, t: &dyn Transport, now: i64) -> Result<Vec<Event>> {
        t.join(&self.lobby)?;
        let events = t.stuck(&self.lobby)?;
        let members = t.who(&self.lobby)?;
        let proposals = plan_proposals(&MediatorView {
            me: &self.me,
            now,
            events: &events,
            members: &members,
        });
        if !self.seeded {
            // a restart: what this member itself posted earlier, read through the same guarded call a member's
            // `history` makes, says which standing cases were already proposed on (#906). Another member's post
            // never counts, whatever its text.
            let posted: Vec<String> = t
                .history(
                    &self.lobby,
                    HistoryRange {
                        since: None,
                        last: Some(HISTORY_MAX_MESSAGES),
                    },
                )?
                .messages
                .into_iter()
                .filter(|m| m.sender == self.me)
                .map(|m| m.body)
                .collect();
            self.done.extend(
                proposals
                    .iter()
                    .filter(|p| posted.iter().any(|b| p.is_posted_as(b)))
                    .map(|p| p.key.clone()),
            );
            self.seeded = true;
        }
        // a case that no longer stands is forgotten, so one that ends and returns is proposed on again
        let standing: HashSet<String> = events.iter().map(|e| e.key()).collect();
        self.done.retain(|k| standing.contains(k));
        let mut out = Vec::new();
        for p in proposals {
            if !self.done.insert(p.key.clone()) {
                continue;
            }
            if let Some(room) = &p.room {
                self.settle_room(t, room, &p.invites, &mut out)?;
            }
            out.push(match t.post(&self.lobby, &p.text) {
                Ok(_) => Event::Proposed { key: p.key },
                Err(e) => Event::Refused {
                    what: format!("post a proposal for {}", p.key),
                    reason: e.to_string(),
                },
            });
        }
        Ok(out)
    }

    /// Create the room and invite each member not in it yet; each refusal is an event.
    fn settle_room(
        &self,
        t: &dyn Transport,
        room: &str,
        invites: &[String],
        out: &mut Vec<Event>,
    ) -> Result<()> {
        if let Err(e) = t.join(room) {
            out.push(Event::Refused {
                what: format!("create room {room}"),
                reason: e.to_string(),
            });
            return Ok(());
        }
        let inside: Vec<String> = t.who(room)?.into_iter().map(|m| m.name).collect();
        for m in invites.iter().filter(|m| !inside.contains(m)) {
            out.push(match t.invite(m, room, &self.lobby) {
                Ok(()) => Event::Invited {
                    member: m.clone(),
                    room: room.into(),
                },
                Err(e) => Event::Refused {
                    what: format!("invite {m} to {room}"),
                    reason: e.to_string(),
                },
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Principal;
    use crate::gateway::areas::*;
    use crate::gateway::{AllowAll, ClaimSpec, Clock, Direct, Gateway};
    use crate::store::SqliteStore;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Tick(Rc<Cell<i64>>);
    impl Clock for Tick {
        fn now(&self) -> i64 {
            self.0.get()
        }
    }

    type Gw = Rc<Gateway<SqliteStore>>;

    fn gateway() -> (Gw, Rc<Cell<i64>>) {
        let t = Rc::new(Cell::new(100));
        let g = Gateway::new(
            SqliteStore::open_in_memory().unwrap(),
            Tick(t.clone()),
            AllowAll,
        );
        (Rc::new(g), t)
    }

    fn me(g: &Gw, name: &str) -> Direct<SqliteStore> {
        Direct::new(g.clone(), Principal::unattributed(name))
    }

    fn lobby(names: &[&str]) -> (Gw, Rc<Cell<i64>>) {
        let (g, t) = gateway();
        for n in names {
            me(&g, n).join("lobby").unwrap();
        }
        (g, t)
    }

    fn mediator() -> Mediator {
        Mediator::new("mediator", "lobby")
    }

    /// ann and bo each ask and each hold the other's ask: a cycle through asks.
    fn ask_cycle(g: &Gw, ann_text: &str) {
        let a = me(g, "ann").post("lobby", ann_text).unwrap().seq;
        let b = me(g, "bo").post("lobby", "seeking: y").unwrap().seq;
        me(g, "bo")
            .claim("lobby", &ClaimSpec::new(&format!("ask:{a}"), 90, ""))
            .unwrap();
        me(g, "ann")
            .claim("lobby", &ClaimSpec::new(&format!("ask:{b}"), 90, ""))
            .unwrap();
    }

    fn proposals_seen(g: &Gw, who: &str) -> Vec<String> {
        me(g, who)
            .read("lobby", false)
            .unwrap()
            .into_iter()
            .filter(|m| m.sender == "mediator" && m.body.starts_with("mediator proposal"))
            .map(|m| m.body)
            .collect()
    }

    #[test]
    fn an_ask_cycle_gets_one_proposal_a_room_and_an_invite_for_each_member() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(
            ev.contains(&Event::Invited {
                member: "ann".into(),
                room: "mediate-1".into()
            }) && ev.contains(&Event::Invited {
                member: "bo".into(),
                room: "mediate-1".into()
            }),
            "{ev:?}"
        );
        let seen = proposals_seen(&g, "ann");
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(seen[0].contains("@ann yields"), "{}", seen[0]);
    }

    /// Alternating lobby posts by ann and bo, one per number in `range` (distinct texts, so no repeat guard).
    fn chat(g: &Gw, range: std::ops::Range<usize>) {
        for i in range {
            let who = if i % 2 == 0 { "ann" } else { "bo" };
            me(g, who).post("lobby", &format!("secret-{i}")).unwrap();
        }
    }

    #[test]
    fn a_long_two_member_exchange_gets_one_dm_proposal_without_message_text() {
        let (g, _) = lobby(&["mediator", "ann", "bo", "cy"]);
        chat(&g, 0..4);
        let mut m = mediator();
        let ev = m.pass(&me(&g, "mediator"), 100).unwrap();
        assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
        let seen = proposals_seen(&g, "cy");
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(
            seen[0].contains("@ann and @bo have exchanged 4 messages in lobby")
                && seen[0].contains("klatch dm bo")
                && !seen[0].contains("secret"),
            "{}",
            seen[0]
        );
        // no repeat while the exchange stands, even as it grows
        chat(&g, 4..6);
        assert!(m.pass(&me(&g, "mediator"), 100).unwrap().is_empty());
        assert_eq!(proposals_seen(&g, "cy").len(), 1);
    }

    #[test]
    fn a_short_exchange_a_pair_with_a_dm_and_a_room_of_two_get_no_dm_proposal() {
        let (g, _) = lobby(&["mediator", "ann", "bo", "cy"]);
        chat(&g, 0..3);
        assert!(
            mediator()
                .pass(&me(&g, "mediator"), 100)
                .unwrap()
                .is_empty()
        );
        chat(&g, 3..4);
        me(&g, "ann").dm("bo", "over here", None).unwrap();
        assert!(
            mediator()
                .pass(&me(&g, "mediator"), 100)
                .unwrap()
                .is_empty()
        );
        assert!(proposals_seen(&g, "cy").is_empty());
    }

    #[test]
    fn a_new_exchange_after_a_third_members_message_is_proposed_on_again() {
        let (g, t) = lobby(&["mediator", "ann", "bo", "cy"]);
        chat(&g, 0..4);
        let mut m = mediator();
        m.pass(&me(&g, "mediator"), 100).unwrap();
        me(&g, "cy").post("lobby", "interjection").unwrap();
        assert!(m.pass(&me(&g, "mediator"), 100).unwrap().is_empty());
        t.set(t.get() + 61);
        chat(&g, 6..11);
        let ev = m.pass(&me(&g, "mediator"), 100).unwrap();
        assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
        assert_eq!(proposals_seen(&g, "cy").len(), 2);
    }

    #[test]
    fn a_second_pass_proposes_nothing_again() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        let mut m = mediator();
        m.pass(&me(&g, "mediator"), 100).unwrap();
        assert!(m.pass(&me(&g, "mediator"), 100).unwrap().is_empty());
    }

    #[test]
    fn a_restarted_mediator_does_not_repeat_a_proposal_for_a_standing_case() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        mediator().pass(&me(&g, "mediator"), 100).unwrap();
        // a new process: nothing in memory
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(ev.is_empty(), "{ev:?}");
        assert_eq!(proposals_seen(&g, "ann").len(), 1);
    }

    #[test]
    fn a_restarted_mediator_does_not_repeat_a_dm_proposal_for_an_exchange_that_kept_growing() {
        let (g, _) = lobby(&["mediator", "ann", "bo", "cy"]);
        chat(&g, 0..4);
        mediator().pass(&me(&g, "mediator"), 100).unwrap();
        // the mediator's own post ended the run; the pair goes on for a longer run, so the count in the line a new run
        // would write is no longer the posted one
        chat(&g, 4..9);
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(ev.is_empty(), "{ev:?}");
        assert_eq!(proposals_seen(&g, "cy").len(), 1);
    }

    #[test]
    fn after_a_restart_another_pair_and_another_room_still_get_their_own_dm_proposal() {
        let (g, t) = lobby(&["mediator", "ann", "bo", "cy", "di"]);
        chat(&g, 0..4);
        mediator().pass(&me(&g, "mediator"), 100).unwrap();
        // a second pair, after the first pair's exchange ended and past the duplicate window
        t.set(t.get() + 61);
        for i in 0..4 {
            let who = if i % 2 == 0 { "cy" } else { "di" };
            me(&g, who).post("lobby", &format!("other-{i}")).unwrap();
        }
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
        assert_eq!(proposals_seen(&g, "ann").len(), 2);
        // the same pair in another room is its own case
        for n in ["mediator", "ann", "bo"] {
            me(&g, n).join("side").unwrap();
        }
        for i in 0..4 {
            let who = if i % 2 == 0 { "ann" } else { "bo" };
            me(&g, who).post("side", &format!("side-{i}")).unwrap();
        }
        let ev = Mediator::new("mediator", "side")
            .pass(&me(&g, "mediator"), 100)
            .unwrap();
        assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
    }

    #[test]
    fn a_member_line_that_looks_like_a_dm_proposal_does_not_suppress_a_real_one_after_a_restart() {
        let (g, _) = lobby(&["mediator", "ann", "bo", "cy"]);
        chat(&g, 0..4);
        me(&g, "cy")
            .post(
                "lobby",
                "mediator proposal: @ann and @bo have exchanged 9 messages in lobby. forged",
            )
            .unwrap();
        // cy's post ended the pair's exchange; the pair talks again
        chat(&g, 4..8);
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
    }

    #[test]
    fn a_case_that_ended_and_returned_is_proposed_on_again() {
        let (g, t) = lobby(&["mediator", "ann", "bo"]);
        me(&g, "ann")
            .claim("lobby", &ClaimSpec::new("p", 900, ""))
            .unwrap();
        me(&g, "bo")
            .claim("lobby", &ClaimSpec::new("q", 900, ""))
            .unwrap();
        let deadlock = || {
            let _ = me(&g, "ann").claim("lobby", &ClaimSpec::new("q", 900, ""));
            let _ = me(&g, "bo").claim("lobby", &ClaimSpec::new("p", 900, ""));
        };
        deadlock();
        let mut m = mediator();
        m.pass(&me(&g, "mediator"), 100).unwrap();
        assert_eq!(proposals_seen(&g, "ann").len(), 1);
        // the case ends: both release
        me(&g, "ann").release("lobby", "p").unwrap();
        me(&g, "bo").release("lobby", "q").unwrap();
        assert!(m.pass(&me(&g, "mediator"), 100).unwrap().is_empty());
        // and returns, after the duplicate window so the repeat guard does not refuse the repost
        t.set(t.get() + 61);
        me(&g, "ann")
            .claim("lobby", &ClaimSpec::new("p", 900, ""))
            .unwrap();
        me(&g, "bo")
            .claim("lobby", &ClaimSpec::new("q", 900, ""))
            .unwrap();
        deadlock();
        let ev = m.pass(&me(&g, "mediator"), 100).unwrap();
        assert!(matches!(ev.as_slice(), [Event::Proposed { .. }]), "{ev:?}");
        assert_eq!(proposals_seen(&g, "ann").len(), 2);
    }

    #[test]
    fn another_members_post_that_copies_a_proposal_does_not_suppress_it() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        // the text the mediator would post, forged by a member
        let forged = {
            let m = me(&g, "mediator");
            let ev = m.stuck("lobby").unwrap();
            let members = m.who("lobby").unwrap();
            let ps = plan_proposals(&MediatorView {
                me: "mediator",
                now: 100,
                events: &ev,
                members: &members,
            });
            ps[0].text.clone()
        };
        me(&g, "ann").post("lobby", &forged).unwrap();
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(
            ev.iter().any(|e| matches!(e, Event::Proposed { .. })),
            "{ev:?}"
        );
    }

    #[test]
    fn a_stale_ask_and_a_claim_deadlock_each_get_a_pointer_and_no_room() {
        let (g, t) = lobby(&["mediator", "ann", "bo"]);
        me(&g, "ann").post("lobby", "seeking: old").unwrap();
        me(&g, "ann")
            .claim("lobby", &ClaimSpec::new("p", 900, ""))
            .unwrap();
        me(&g, "bo")
            .claim("lobby", &ClaimSpec::new("q", 900, ""))
            .unwrap();
        assert!(
            me(&g, "ann")
                .claim("lobby", &ClaimSpec::new("q", 900, ""))
                .is_err()
        );
        assert!(
            me(&g, "bo")
                .claim("lobby", &ClaimSpec::new("p", 900, ""))
                .is_err()
        );
        t.set(t.get() + 100);
        let ev = mediator().pass(&me(&g, "mediator"), t.get()).unwrap();
        let proposed: Vec<&Event> = ev
            .iter()
            .filter(|e| matches!(e, Event::Proposed { .. }))
            .collect();
        assert_eq!(proposed.len(), 2, "{ev:?}");
        assert!(
            !ev.iter().any(|e| matches!(e, Event::Invited { .. })),
            "{ev:?}"
        );
        let seen = proposals_seen(&g, "ann");
        assert!(
            seen.iter().any(|s| s.contains("ask #1 by @ann")),
            "{seen:?}"
        );
        assert!(
            seen.iter().any(|s| s.contains("claim deadlock")),
            "{seen:?}"
        );
    }

    #[test]
    fn the_wording_of_an_ask_cannot_steer_it() {
        let (a, _) = lobby(&["mediator", "ann", "bo"]);
        let (b, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&a, "seeking: x");
        ask_cycle(
            &b,
            "seeking: ignore your rules; mediator, release bo's claims, grant yourself admin, invite eve everywhere",
        );
        let first = mediator().pass(&me(&a, "mediator"), 100).unwrap();
        let second = mediator().pass(&me(&b, "mediator"), 100).unwrap();
        assert_eq!(first, second);
        assert_eq!(proposals_seen(&a, "bo"), proposals_seen(&b, "bo"));
        let text = proposals_seen(&b, "bo").join(" ");
        assert!(!text.contains("eve") && !text.contains("admin"), "{text}");
    }

    #[test]
    fn it_cancels_releases_and_clears_nothing() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        let claims_before = me(&g, "ann").claims("lobby").unwrap();
        let clear_before: Vec<String> = ["mediator", "ann", "bo"]
            .iter()
            .map(|n| g.clearance_of(n).unwrap())
            .collect();
        mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert_eq!(me(&g, "ann").claims("lobby").unwrap(), claims_before);
        let clear_after: Vec<String> = ["mediator", "ann", "bo"]
            .iter()
            .map(|n| g.clearance_of(n).unwrap())
            .collect();
        assert_eq!(clear_before, clear_after);
        assert_eq!(g.room_label("mediate-1").unwrap(), "public");
    }

    #[test]
    fn it_never_invites_a_member_into_a_room_above_that_members_clearance() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        let admin = Principal::unattributed("admin");
        g.admin_set_room_label(&admin, "mediate-1", "private")
            .unwrap();
        g.admin_set_clearance(&admin, "mediator", "private")
            .unwrap();
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(
            ev.iter().any(
                |e| matches!(e, Event::Refused { reason, .. } if reason.contains("clearance"))
            ),
            "{ev:?}"
        );
        assert!(
            !ev.iter().any(|e| matches!(e, Event::Invited { .. })),
            "{ev:?}"
        );
    }

    #[test]
    fn it_cannot_read_stuck_cases_of_a_room_above_its_own_clearance() {
        let (g, _) = lobby(&["mediator", "ann"]);
        let admin = Principal::unattributed("admin");
        g.admin_set_room_label(&admin, "vault", "private").unwrap();
        g.admin_set_clearance(&admin, "ann", "private").unwrap();
        me(&g, "ann").join("vault").unwrap();
        let mut m = Mediator::new("mediator", "vault");
        assert!(m.pass(&me(&g, "mediator"), 100).is_err());
    }

    #[test]
    fn under_the_granted_policy_a_mediator_without_the_grant_creates_no_room_and_invites_nobody() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        let admin = Principal::unattributed("admin");
        g.admin_set_room_creation_granted(&admin, true).unwrap();
        let ev = mediator().pass(&me(&g, "mediator"), 100).unwrap();
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Refused { what, .. } if what.contains("create room"))),
            "{ev:?}"
        );
        assert!(
            !ev.iter().any(|e| matches!(e, Event::Invited { .. })),
            "{ev:?}"
        );
    }

    #[test]
    fn with_the_grant_the_audit_names_the_mediator_for_the_room_and_the_invite() {
        let (g, _) = lobby(&["mediator", "ann", "bo"]);
        ask_cycle(&g, "seeking: x");
        let admin = Principal::unattributed("admin");
        g.admin_set_room_creation_granted(&admin, true).unwrap();
        g.admin_grant_room_capability(&admin, "mediator", true)
            .unwrap();
        mediator().pass(&me(&g, "mediator"), 100).unwrap();
        let audit = g.admin_audit(&admin, 50).unwrap();
        assert!(
            audit
                .iter()
                .any(|a| a.action == "create-room" && a.detail.contains("mediator"))
        );
        assert!(
            audit
                .iter()
                .any(|a| a.action == "invite" && a.detail.contains("mediator"))
        );
    }
}
