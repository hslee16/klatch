//! Ask routing (#1407): an ask finds its own helper. When an ask is posted the gateway asks `could_answer` of every member
//! of the room (the same questions it asks of a member that posts the answer: the ask is open, not the member's own, no
//! claim holds it, the member is not busy, its credential is live, it is not stopped, its clearance covers the room's
//! label and its role may post), orders the members that could by the structured fields of the ask (see
//! `domain::ask_routing`) and addresses the ask to at most three of them. When nobody takes it within the room's ask
//! lease, or a claim on it lapsed, it is addressed to the coordinators too, once.
//!
//! Where it runs, and why: in the gateway, at post time and lazily afterwards, not in the mediator process. The
//! mediator (`klatch mediator`) is a separate loop that proposes posts from stuck events; routing has to work for a room
//! with no such process, must be cheap and deterministic, and needs to be there before the first digest of the member it
//! addresses. Everything is computed from rows the gateway already holds, so no extra process, socket or timer exists:
//! the lease is checked when a digest, a wake count, a stuck listing or an ask claim next looks at the room.
//!
//! What it never does: it posts nothing, claims nothing, answers nothing and closes nothing on a member's behalf, holds
//! no admin act, reads no word of a message, and addresses nobody `could_answer` refuses (so never a member whose
//! clearance does not cover the ask's label, one removed from the room or one that cannot post). An addressing is a row
//! of names and numbers; the digest ranks it as an ask for the member (#1405), a narrowed view and a mute let it through,
//! and a wake count includes it, so an idle member that opted into wake is woken by it.

use super::*;
use crate::domain::{
    AskRoute, Candidate, ESCALATED, LAPSED, OpenAsk, ROUTED, RouteKind, TAKEN, pick_routes,
    route_affinity,
};

/// Whether a room switched routing off with the policy key `room:ROOM:ask-routing` set to `off`. Routing is on
/// otherwise. No admin act sets it yet (ticketed); the key is read at every use.
const OFF: &str = "off";

/// The blockers a coordinator is addressed despite: it is expected to be busy with claims of its own, and it need not
/// share the ask's scope. Everything else (the label, a stop, its role) still refuses it.
const COORDINATOR_MAY: [&str; 2] = ["busy", "scope"];

impl<S: Store> Gateway<S> {
    /// Who the asks of `room` are addressed to, by ask and member. A read like `open_asks`, guarded the same way, and it
    /// carries names and numbers only. Closed asks stay listed until the record ages out (30 days).
    pub fn ask_routes(&self, who: &Principal, room: &str) -> Result<Vec<AskRoute>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store.ask_routes(room)
    }

    fn routing_on(&self, room: &str) -> Result<bool> {
        Ok(self
            .store
            .policy_get(&format!("room:{room}:ask-routing"))?
            .as_deref()
            != Some(OFF))
    }

    /// Route the ask just posted at `seq` (a failure here never fails the post: routing is an aid, and the ask is
    /// already in the room).
    pub(super) fn route_ask_now(&self, room: &str, seq: i64) {
        let _ = self.try_route_ask(room, seq);
    }

    fn try_route_ask(&self, room: &str, seq: i64) -> Result<()> {
        if !self.routing_on(room)? || self.dm_pair(room)?.is_some() {
            return Ok(());
        }
        let now = self.now();
        let asks = self.store.open_asks(room, now)?;
        let Some(ask) = asks.iter().find(|a| a.seq == seq) else {
            return Ok(());
        };
        let mut candidates = Vec::new();
        for m in self.store.members(room)? {
            if m.name == ask.sender {
                continue;
            }
            if !self
                .could_answer_of((room, &m.name), (seq, Some(ask)), now)?
                .could()
            {
                continue;
            }
            let trail = self.store.trail_of(&m.name)?.files;
            let scope = self.store.member_scope(&m.name)?;
            candidates.push(Candidate {
                affinity: route_affinity(
                    (ask.scope.as_deref(), &ask.refs),
                    (scope.as_deref(), &trail),
                ),
                last_seen: m.last_seen,
                name: m.name,
            });
        }
        let to: Vec<(String, RouteKind)> = pick_routes(candidates)
            .into_iter()
            .map(|n| (n, RouteKind::Capable))
            .collect();
        self.store.route_ask(room, seq, &to, now)?;
        Ok(())
    }

    /// The open asks of `room` addressed to `member` that are still unclaimed: a claimed ask is taken, and routing to
    /// anyone else stops. The member's own rows are read first, so a room that never routed costs one query.
    pub(super) fn routed_asks(&self, room: &str, member: &str) -> Result<Vec<i64>> {
        let mine: Vec<i64> = self
            .store
            .ask_routes(room)?
            .into_iter()
            .filter(|r| r.member == member)
            .map(|r| r.seq)
            .collect();
        if mine.is_empty() {
            return Ok(mine);
        }
        let open = self.store.open_asks(room, self.now())?;
        Ok(mine
            .into_iter()
            .filter(|n| open.iter().any(|a| a.seq == *n && !a.claimed))
            .collect())
    }

    /// The open, unclaimed asks escalated to `member` that surface `bit` (1 digest, 2 wake) has not pointed out yet. They
    /// are behind the member's cursors, so no read, wait or digest of messages shows them again.
    pub(super) fn untold_escalations(
        &self,
        room: &str,
        member: &str,
        bit: i64,
    ) -> Result<Vec<OpenAsk>> {
        let seqs = self.store.untold_routes(room, member, bit)?;
        if seqs.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .store
            .open_asks(room, self.now())?
            .into_iter()
            .filter(|a| a.sender != member && !a.claimed && seqs.contains(&a.seq))
            .collect())
    }

    /// Note what became of the routed asks of `room`, once each: taken (a claim holds it), lapsed (it was taken, and no
    /// claim holds it now), and escalated (nobody took it within the room's ask lease, or its claim lapsed: the
    /// coordinators are addressed too). Run where the room is looked at; an error is dropped, as in `route_ask_now`.
    pub(super) fn sweep_ask_routes(&self, room: &str) {
        let _ = self.try_sweep(room);
    }

    fn try_sweep(&self, room: &str) -> Result<()> {
        let events = self.store.route_events(room)?;
        if events.is_empty() || !self.routing_on(room)? {
            return Ok(());
        }
        let now = self.now();
        let lease = self.settings_of(room)?.ask_lease_secs;
        let asks = self.store.open_asks(room, now)?;
        for ask in &asks {
            let mine: Vec<&(i64, String, i64)> =
                events.iter().filter(|(s, _, _)| *s == ask.seq).collect();
            let Some(&&(_, _, routed_at)) = mine.iter().find(|(_, o, _)| o == ROUTED) else {
                continue;
            };
            let has = |o: &str| mine.iter().any(|(_, w, _)| w == o);
            let mut taken = has(TAKEN);
            if ask.claimed && !taken {
                taken = self
                    .store
                    .note_route_outcome(room, ask.seq, TAKEN, 1, now)?;
            }
            let mut escalate = false;
            if taken && !ask.claimed {
                self.store
                    .note_route_outcome(room, ask.seq, LAPSED, 1, now)?;
                escalate = true;
            } else if !taken && now - routed_at >= lease {
                escalate = true;
            }
            if escalate && !has(ESCALATED) {
                self.escalate(room, ask, now)?;
            }
        }
        Ok(())
    }

    /// Address `ask` to the coordinators of the room (the admin agent and the integrator, the roles that coordinate and
    /// can post) that may read it, and record it once; the count is how many were reached.
    fn escalate(&self, room: &str, ask: &OpenAsk, now: i64) -> Result<()> {
        let mut to = Vec::new();
        for m in self.store.members(room)? {
            if m.name == ask.sender
                || !matches!(self.role_of(&m.name)?, Some(Role::Admin | Role::Integrator))
            {
                continue;
            }
            let c = self.could_answer_of((room, &m.name), (ask.seq, Some(ask)), now)?;
            if c.blockers
                .iter()
                .all(|b| COORDINATOR_MAY.contains(&b.rule.as_str()))
            {
                to.push((m.name, RouteKind::Coordinator));
            }
        }
        if self.store.note_route_outcome(
            room,
            ask.seq,
            ESCALATED,
            i64::try_from(to.len()).unwrap_or(0),
            now,
        )? {
            self.store.add_ask_routes(room, ask.seq, &to, now)?;
        }
        Ok(())
    }
}
