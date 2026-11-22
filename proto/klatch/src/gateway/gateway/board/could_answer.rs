//! "Could member X answer ask N, and if not, why" (#671, part 1 of the decision on #627). A read only. It asks the
//! same questions the gateway asks of a member that posts the answer, for that member and without acting as it, so a
//! supervisor outside klatch can start a pre-issued helper only when the answer is yes. klatch itself starts nothing
//! and issues nothing.

use super::*;
use crate::domain::{Blocker, CouldAnswer};

impl<S: Store> Gateway<S> {
    /// Whether `member` could answer open ask `ask` in `room` now, with every reason it could not.
    ///
    /// Who may ask: a member checks itself (a room-guarded read, so a room above its clearance is refused); asking
    /// about any other member is the admin act `could-answer` (the human, never a plain member, and not the admin
    /// agent), because it reveals another member's clearance, role and claims.
    ///
    /// The checks: the ask is open; the member is not its asker; when both the ask and the member declare a scope they are the same (#1171); no live claim holds it; the member holds no claim
    /// anywhere (busy); its credential, if it has one, is live; it is not stopped (breaker, removed from the room,
    /// paused, room closed or tripped, run and direct-message rules); its clearance in the room (global or a room
    /// grant) dominates the room's label; and its role may post an answer. The answer carries rule words, member
    /// names and numbers, never message text.
    pub fn could_answer(
        &self,
        who: &Principal,
        room: &str,
        member: &str,
        ask: i64,
    ) -> Result<CouldAnswer> {
        check_member(who)?;
        check_room(room)?;
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        if member == who.member {
            self.authorize(who, &Operation::read(room, false))?;
        } else {
            self.check_guard(who, &Operation::admin("could-answer", member))
                .inspect_err(|e| self.note_refusal(who, (room, "could_answer"), e))?;
        }
        let now = self.now();
        let mut out = CouldAnswer {
            member: member.into(),
            ask,
            blockers: Vec::new(),
            refs: Vec::new(),
        };
        let Some(open) = self
            .store
            .open_asks(room, now)?
            .into_iter()
            .find(|a| a.seq == ask)
        else {
            out.blockers.push(blocker(
                "no-such-ask",
                format!("ask #{ask} is not open in room {room}"),
            ));
            return Ok(out);
        };
        if open.sender == member {
            out.blockers.push(blocker(
                "own-ask",
                format!("{member} asked #{ask} itself and cannot answer it"),
            ));
        }
        // what the ask points at (#1318), for the supervisor; no snippet, and it changes no verdict
        out.refs = open
            .refs
            .iter()
            .map(|r| crate::domain::AskRef {
                snippet: None,
                ..r.clone()
            })
            .collect();
        self.scope_blocker(&mut out, open.scope.as_deref())?;
        self.claim_blockers(&mut out, room, ask, now)?;
        self.credential_blocker(&mut out)?;
        self.standing_blockers(&mut out, room, ask)?;
        Ok(out)
    }

    /// The ask declares a scope and the member declares a different one (#1171). When either side declares none,
    /// nothing is added: scope is advisory in klatch, so a missing declaration never refuses.
    fn scope_blocker(&self, out: &mut CouldAnswer, ask_scope: Option<&str>) -> Result<()> {
        let Some(wanted) = ask_scope else {
            return Ok(());
        };
        let Some(theirs) = self.store.member_scope(&out.member)? else {
            return Ok(());
        };
        if theirs != wanted {
            out.blockers.push(blocker(
                "scope",
                format!(
                    "ask #{} declares scope {wanted} and {} works in scope {theirs}",
                    out.ask, out.member
                ),
            ));
        }
        Ok(())
    }

    /// The ask is held, or the member is busy with a claim of its own anywhere.
    fn claim_blockers(&self, out: &mut CouldAnswer, room: &str, ask: i64, now: i64) -> Result<()> {
        let member = out.member.clone();
        let path = format!("ask:{ask}");
        if let Some(c) = self
            .store
            .claims(room, now)?
            .into_iter()
            .find(|c| c.path == path)
        {
            out.blockers.push(blocker(
                "ask-claimed",
                format!("ask #{ask} is already claimed by {}", c.owner),
            ));
        }
        let mut busy = false;
        for r in self.store.rooms_of(&member)? {
            busy |= self
                .store
                .claims(&r, now)?
                .iter()
                .any(|c| c.owner == member && c.path != path);
        }
        if busy {
            out.blockers.push(blocker(
                "busy",
                format!("{member} holds a live claim and is busy"),
            ));
        }
        Ok(())
    }

    /// An issued credential that is revoked or expired: the member could not act, whoever started it.
    fn credential_blocker(&self, out: &mut CouldAnswer) -> Result<()> {
        let Some(rec) = self.store.credential(&out.member)? else {
            return Ok(());
        };
        let why = if rec.revoked {
            "was revoked"
        } else if self.now() >= rec.expires {
            "expired"
        } else {
            return Ok(());
        };
        out.blockers.push(blocker(
            "credential",
            format!(
                "the credential of {} {why}; only the human issues a new one",
                out.member
            ),
        ));
        Ok(())
    }

    /// The stops, the label and the role, each judged for the member as if it posted the answer.
    fn standing_blockers(&self, out: &mut CouldAnswer, room: &str, ask: i64) -> Result<()> {
        let member = out.member.clone();
        let target = Principal::unattributed(&member);
        let post = Operation::Post {
            room: room.into(),
            body: format!("answer to #{ask}: ."),
        };
        let refusals = [
            self.check_stops(&target, room, &post),
            self.check_run(room),
            self.check_dm(&target, room),
            self.check_clearance(&target, room),
            self.check_role(&target, &post),
        ];
        for e in refusals {
            match e {
                Ok(()) => {}
                Err(Error::Refused { rule, reason }) => {
                    let reason = self.third_person(&member, room, &rule, reason)?;
                    out.blockers.push(blocker(&rule, reason));
                }
                Err(other) => return Err(other),
            }
        }
        Ok(())
    }

    /// The reason a refusal gave, worded about the member and not to it.
    fn third_person(&self, member: &str, room: &str, rule: &str, reason: String) -> Result<String> {
        Ok(match rule {
            "breaker" => format!("{member} is stopped by the breaker: {reason}"),
            "admin-block" => format!("{member} was removed from room {room} by the admin"),
            "clearance" => format!(
                "{member} holds {} in room {room}, which is labelled {}; the admin can raise its clearance or the human can grant it for the room",
                self.clearance_in(member, room)?,
                self.room_label(room)?
            ),
            _ => reason,
        })
    }
}

fn blocker(rule: &str, reason: String) -> Blocker {
    Blocker {
        rule: rule.into(),
        reason,
    }
}
