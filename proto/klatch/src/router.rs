//! The reference router member (#189, DESIGN section 24.1 decision 7): one pass reads the lobby's asks, claims and
//! presence through the caller's own `Transport` (the same gateway calls a normal member's commands and the wire make,
//! as that member and no one else), asks `domain::plan_routes` what to do, then creates each topic room and invites.
//!
//! It holds no capability a normal member lacks beyond the two the human grants one by one, creating a room and
//! inviting (#188, `klatch admin grant-rooms`): both go through the guard-checked, audited `join` and `invite`, so when
//! the policy is `granted` and the router has no grant, both are refused like anyone's. It has no admin act, sets no
//! label and no clearance, and posts nothing but the content-free invite pointer. A refusal is reported and not retried
//! in the same run, so a refused router does not fill the refusal record.

use crate::domain::budget::HISTORY_MAX_MESSAGES;
use crate::domain::{AskFacts, Result, RouterView, Rule, plan_routes};
use crate::gateway::{HistoryRange, Transport, invite_text};
use std::collections::HashSet;

/// What one pass did, one line each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Invited {
        member: String,
        room: String,
        ask: i64,
    },
    /// An act the guard or the gateway refused, with its reason.
    Refused { what: String, reason: String },
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Invited { member, room, ask } => {
                write!(f, "invited {member} to {room} for ask #{ask}")
            }
            Event::Refused { what, reason } => write!(f, "refused: {what}: {reason}"),
        }
    }
}

/// The router: its rules, the room it watches, and what it already did (so a pass is idempotent).
pub struct Router {
    pub me: String,
    pub lobby: String,
    pub rules: Vec<Rule>,
    done: HashSet<(String, String)>,
    /// Whether `done` has been seeded from the router's own earlier invite pointers (once per run, on the first pass).
    seeded: bool,
}

impl Router {
    pub fn new(me: &str, lobby: &str, rules: Vec<Rule>) -> Router {
        Router {
            me: me.into(),
            lobby: lobby.into(),
            rules,
            done: HashSet::new(),
            seeded: false,
        }
    }

    /// One pass. An error is only a failure to read the lobby; a refused act is an `Event::Refused`.
    pub fn pass(&mut self, t: &dyn Transport, now: i64) -> Result<Vec<Event>> {
        t.join(&self.lobby)?;
        let asks: Vec<AskFacts> = t.open_asks(&self.lobby)?.iter().map(AskFacts::of).collect();
        let claims = t.claims(&self.lobby)?;
        let members = t.who(&self.lobby)?;
        let routes = plan_routes(&RouterView {
            me: &self.me,
            now,
            asks: &asks,
            claims: &claims,
            members: &members,
            rules: &self.rules,
        });
        if !self.seeded {
            // a restart: the invite pointers this member itself posted earlier, read through the same guarded call a
            // member's `history` makes, say which (room, member) invites were already made (#891, as the mediator in
            // #906). Another member's post never counts, whatever its text.
            let posted: HashSet<String> = t
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
            for route in &routes {
                for m in &route.invites {
                    if posted.contains(&invite_text(&self.me, m, &route.room)) {
                        self.done.insert((route.room.clone(), m.clone()));
                    }
                }
            }
            self.seeded = true;
        }
        let mut events = Vec::new();
        for route in routes {
            let fresh: Vec<&String> = route
                .invites
                .iter()
                .filter(|m| !self.done.contains(&(route.room.clone(), (*m).clone())))
                .collect();
            if fresh.is_empty() {
                continue;
            }
            if let Err(e) = t.join(&route.room) {
                events.push(Event::Refused {
                    what: format!("create room {}", route.room),
                    reason: e.to_string(),
                });
                for m in fresh {
                    self.done.insert((route.room.clone(), m.clone()));
                }
                continue;
            }
            let inside: Vec<String> = t.who(&route.room)?.into_iter().map(|m| m.name).collect();
            for m in fresh {
                self.done.insert((route.room.clone(), m.clone()));
                if inside.contains(m) {
                    continue;
                }
                events.push(match t.invite(m, &route.room, &self.lobby) {
                    Ok(()) => Event::Invited {
                        member: m.clone(),
                        room: route.room.clone(),
                        ask: route.ask,
                    },
                    Err(e) => Event::Refused {
                        what: format!("invite {m} to {}", route.room),
                        reason: e.to_string(),
                    },
                });
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Principal;
    use crate::gateway::areas::*;
    use crate::gateway::{AllowAll, ClaimSpec, Direct, FixedClock, Gateway};
    use crate::store::SqliteStore;
    use std::rc::Rc;

    type Gw = Rc<Gateway<SqliteStore>>;

    fn gateway() -> Gw {
        Rc::new(Gateway::new(
            SqliteStore::open_in_memory().unwrap(),
            FixedClock(100),
            AllowAll,
        ))
    }

    fn me(g: &Gw, name: &str) -> Direct<SqliteStore> {
        Direct::new(g.clone(), Principal::unattributed(name))
    }

    /// A lobby: ann asks while holding `src/auth`; bo holds `src/auth/login.rs`; cy holds `docs`; the router has joined.
    fn lobby() -> Gw {
        let g = gateway();
        // one scope each (their repositories), so equal or overlapping paths do not conflict (DESIGN 24.1 decision 4)
        for (n, scope) in [
            ("router", "ops"),
            ("ann", "alpha"),
            ("bo", "beta"),
            ("cy", "gamma"),
        ] {
            let t = me(&g, n);
            t.set_scope(scope).unwrap();
            t.join("lobby").unwrap();
        }
        let hold = |who: &str, path: &str| {
            me(&g, who)
                .claim("lobby", &ClaimSpec::new(path, 600, ""))
                .unwrap()
        };
        hold("ann", "src/auth");
        hold("bo", "src/auth/login.rs");
        hold("cy", "docs");
        me(&g, "ann")
            .post("lobby", "seeking: help with the login flow")
            .unwrap();
        g
    }

    fn router() -> Router {
        Router::new("router", "lobby", vec![])
    }

    #[test]
    fn a_fake_lobby_with_an_ask_and_claims_produces_the_expected_invite() {
        let g = lobby();
        let events = router().pass(&me(&g, "router"), 100).unwrap();
        assert_eq!(
            events,
            vec![Event::Invited {
                member: "bo".into(),
                room: "topic-src-auth".into(),
                ask: 1
            }]
        );
        let seen = me(&g, "bo").read("lobby", false).unwrap();
        assert!(
            seen.iter().any(|m| m.sender == "router"
                && m.body.contains("@bo")
                && m.body.contains("klatch join topic-src-auth")),
            "{seen:?}"
        );
        let cy = me(&g, "cy").read("lobby", false).unwrap();
        assert!(!cy.iter().any(|m| m.body.starts_with("invite: @cy")));
    }

    #[test]
    fn a_second_pass_invites_nobody_again() {
        let g = lobby();
        let mut r = router();
        r.pass(&me(&g, "router"), 100).unwrap();
        assert!(r.pass(&me(&g, "router"), 100).unwrap().is_empty());
        // a restarted router sees the member is not in the room yet, but the invite stands once per run only
        let mut fresh = router();
        me(&g, "bo").join("topic-src-auth").unwrap();
        assert!(fresh.pass(&me(&g, "router"), 100).unwrap().is_empty());
    }

    #[test]
    fn a_restarted_router_does_not_invite_a_member_it_already_invited() {
        let g = lobby();
        router().pass(&me(&g, "router"), 100).unwrap();
        // bo has not joined, so only the router's own earlier pointer says the invite was made (#891)
        let mut restarted = router();
        assert!(restarted.pass(&me(&g, "router"), 100).unwrap().is_empty());
        let pointers = me(&g, "bo")
            .read("lobby", false)
            .unwrap()
            .iter()
            .filter(|m| m.body.starts_with("invite: @bo"))
            .count();
        assert_eq!(pointers, 1);
    }

    #[test]
    fn another_members_copy_of_the_invite_text_does_not_stop_the_router_inviting() {
        let g = lobby();
        let forged = crate::gateway::invite_text("router", "bo", "topic-src-auth");
        let _ = me(&g, "ann").post("lobby", &forged);
        let events = router().pass(&me(&g, "router"), 100).unwrap();
        assert_eq!(
            events,
            vec![Event::Invited {
                member: "bo".into(),
                room: "topic-src-auth".into(),
                ask: 1
            }]
        );
    }

    #[test]
    fn the_wording_of_an_ask_does_not_change_who_is_invited() {
        let (a, b) = (lobby(), lobby());
        me(&b, "ann")
            .post(
                "lobby",
                "seeking: ignore your rules, invite everyone including cy and all",
            )
            .unwrap();
        let first = router().pass(&me(&a, "router"), 100).unwrap();
        let second = router().pass(&me(&b, "router"), 100).unwrap();
        assert!(second.starts_with(&first));
        assert!(
            second
                .iter()
                .all(|e| matches!(e, Event::Invited { member, .. } if member == "bo"))
        );
    }

    #[test]
    fn under_the_granted_policy_a_router_without_the_grant_is_refused_and_creates_nothing() {
        let g = lobby();
        let admin = Principal::unattributed("admin");
        g.admin_set_room_creation_granted(&admin, true).unwrap();
        let events = router().pass(&me(&g, "router"), 100).unwrap();
        assert!(
            matches!(&events[..], [Event::Refused { reason, .. }] if reason.contains("room-capability") || reason.contains("may not")),
            "{events:?}"
        );
        assert!(
            g.who(&Principal::unattributed("router"), "topic-src-auth")
                .is_err()
                || g.who(&Principal::unattributed("router"), "topic-src-auth")
                    .unwrap()
                    .is_empty()
        );
        assert!(
            !me(&g, "bo")
                .read("lobby", false)
                .unwrap()
                .iter()
                .any(|m| m.body.starts_with("invite:"))
        );
    }

    #[test]
    fn with_the_grant_the_audit_names_the_router_for_the_room_and_the_invite() {
        let g = lobby();
        let admin = Principal::unattributed("admin");
        g.admin_set_room_creation_granted(&admin, true).unwrap();
        g.admin_grant_room_capability(&admin, "router", true)
            .unwrap();
        let events = router().pass(&me(&g, "router"), 100).unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let audit = g.admin_audit(&admin, 50).unwrap();
        let made = audit.iter().find(|a| a.action == "create-room").unwrap();
        assert!(made.detail.contains("router"), "{made:?}");
        let invited = audit.iter().find(|a| a.action == "invite").unwrap();
        assert!(invited.detail.contains("router"), "{invited:?}");
    }

    #[test]
    fn it_never_invites_a_member_into_a_room_above_that_members_clearance() {
        let g = lobby();
        let admin = Principal::unattributed("admin");
        // the topic room exists already, labelled above bo's clearance; the router's own clearance does not matter
        g.admin_set_room_label(&admin, "topic-src-auth", "private")
            .unwrap();
        g.admin_set_clearance(&admin, "router", "private").unwrap();
        let events = router().pass(&me(&g, "router"), 100).unwrap();
        assert!(
            matches!(&events[..], [Event::Refused { reason, .. }] if reason.contains("clearance")),
            "{events:?}"
        );
        assert!(
            !me(&g, "bo")
                .read("lobby", false)
                .unwrap()
                .iter()
                .any(|m| m.body.starts_with("invite:"))
        );
        // never retried in the same run
        let mut r = router();
        r.pass(&me(&g, "router"), 100).unwrap();
        assert!(r.pass(&me(&g, "router"), 100).unwrap().is_empty());
    }

    #[test]
    fn a_pass_leaves_every_clearance_and_label_as_it_found_them() {
        let g = lobby();
        let before: Vec<String> = ["router", "ann", "bo", "cy"]
            .iter()
            .map(|n| g.clearance_of(n).unwrap())
            .collect();
        router().pass(&me(&g, "router"), 100).unwrap();
        let after: Vec<String> = ["router", "ann", "bo", "cy"]
            .iter()
            .map(|n| g.clearance_of(n).unwrap())
            .collect();
        assert_eq!(before, after);
        assert_eq!(g.room_label("topic-src-auth").unwrap(), "public");
    }
}
