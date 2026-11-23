//! Passive claims (#1401): what the opt-in PostToolUse step does through the gateway. A member that edits or writes a
//! file claims its repo-relative path for a short lease that every further edit renews; the Stop and SessionEnd steps
//! release it. It never blocks (that is `init --enforce`'s separate mode): a refusal is returned as a value for the hook
//! to print once.
//!
//! The claim goes in one room: the default lobby when the member is in it (the convention: `klatch claim lobby
//! <path>`), else the first room by name that the member joined that is not an ask room or a direct message. A room
//! whose label is sealed is never used (a claim name is plain text, and a sealed label is where text must not go
//! unprotected), nor one the member's clearance does not reach or a policy refuses.

use super::*;
use crate::domain::{
    PASSIVE_CLAIM_SECS, PASSIVE_REASON, PassiveOutcome, PassiveRefusal, PassiveTaken, covers,
    passive_path, valid_name,
};

/// The room the default claims go to.
const DEFAULT_ROOM: &str = "lobby";

impl<S: Store> Gateway<S> {
    /// Claim the repo-relative `path` a member just edited or wrote. Any failure is counted (a number, no text) and
    /// returned; the hook swallows it, because a hook failure never fails the tool call.
    pub fn passive_claim(&self, who: &Principal, path: &str) -> Result<PassiveOutcome> {
        let out = self.try_passive_claim(who, path);
        if out.is_err() {
            self.store.note_passive_failure(self.now());
        }
        out
    }

    fn try_passive_claim(&self, who: &Principal, path: &str) -> Result<PassiveOutcome> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        let Some(path) = passive_path(path) else {
            return Ok(PassiveOutcome::Skipped);
        };
        let member = who.member.as_str();
        let now = self.now();
        let rooms = self.store.rooms_of(member)?;
        // an explicit claim of the member's own that covers the path is the claim: nothing is added beside it
        for room in &rooms {
            for c in self.store.claims(room, now)? {
                if c.owner == member && c.reason != PASSIVE_REASON && covers(&c.path, &path) {
                    return Ok(PassiveOutcome::Covered);
                }
            }
        }
        let Some(room) = self.passive_room(who, &rooms, &path)? else {
            return Ok(PassiveOutcome::Skipped);
        };
        let scope =
            crate::domain::claim_scope(&path, false, self.store.member_scope(member)?.as_deref());
        let request = ClaimRequest::new((&room, member), &path, PASSIVE_CLAIM_SECS, PASSIVE_REASON)
            .scoped(&scope);
        Ok(match self.store.take_passive_claim(&request, now)? {
            PassiveTaken::Granted => PassiveOutcome::Granted,
            PassiveTaken::Renewed => PassiveOutcome::Renewed,
            PassiveTaken::Refused {
                owner,
                path,
                held_secs,
                secs_left,
            } => PassiveOutcome::Refused(PassiveRefusal {
                room,
                owner,
                path,
                held_secs,
                secs_left,
            }),
        })
    }

    /// The room a passive claim for `path` goes in, if there is one the member may claim in.
    fn passive_room(
        &self,
        who: &Principal,
        rooms: &[String],
        path: &str,
    ) -> Result<Option<String>> {
        let mut candidates = Vec::new();
        for room in rooms {
            if room == DEFAULT_ROOM || (!room.starts_with("ask-") && self.dm_pair(room)?.is_none())
            {
                candidates.push(room.clone());
            }
        }
        let room = if candidates.iter().any(|r| r == DEFAULT_ROOM) {
            DEFAULT_ROOM.to_string()
        } else if let Some(first) = candidates.into_iter().next() {
            first
        } else {
            return Ok(None);
        };
        if !valid_name(&room) {
            return Ok(None);
        }
        let label = self.room_label(&room)?;
        if self.store.policy_get(&format!("sealed:{label}"))?.is_some() {
            return Ok(None);
        }
        let allowed = self.authorize_quietly(
            who,
            &Operation::Claim {
                room: room.clone(),
                path: path.into(),
            },
        );
        Ok(allowed.is_ok().then_some(room))
    }

    /// Release the passive claims the caller holds (its turn ended, or its session did). Never an explicit claim.
    /// Returns how many were live.
    pub fn release_passive_claims(&self, who: &Principal) -> Result<usize> {
        check_member(who)?;
        self.store.release_passive_claims(&who.member, self.now())
    }
}
