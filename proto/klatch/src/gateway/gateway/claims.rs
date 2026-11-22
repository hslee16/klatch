//! Claims on paths: taking, releasing and listing them.

use super::*;

impl<S: Store> Gateway<S> {
    /// Take a claim. A shared claim, like any `ask:<N>`, conflicts with every scope.
    pub fn claim(&self, who: &Principal, room: &str, spec: &ClaimSpec) -> Result<()> {
        let ClaimSpec {
            path,
            ttl,
            reason,
            shared,
            anchor,
        } = *spec;
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        // a work item name is compared in one form, so `Design-Item:66 ` and `design-item:66` are one claim (#632)
        let path = crate::domain::canonical_claim_path(path);
        let path = path.as_str();
        check_reason(reason)?;
        if let Some(a) = anchor
            && !valid_anchor(a)
        {
            return Err(Error::Invalid(format!("bad anchor: {a:?}")));
        }
        self.refuse_text(who, (room, "claim"), reason)?;
        if ttl <= 0 {
            return Err(Error::Invalid("ttl must be positive".into()));
        }
        if ttl > crate::domain::CLAIM_MAX_SECS {
            return Err(Error::Invalid(format!(
                "ttl is at most {} seconds (24 hours); claim again to renew",
                crate::domain::CLAIM_MAX_SECS
            )));
        }
        self.authorize(
            who,
            &Operation::Claim {
                room: room.into(),
                path: path.into(),
            },
        )?;
        let ttl = crate::domain::claim_ttl(path, Some(ttl));
        // before the attempt deletes expired rows: a holder the probe finds alive keeps its claim (#304)
        self.settle_leases(&who.member)?;
        let member_scope = self.store.member_scope(&who.member)?;
        let scope = crate::domain::claim_scope(path, shared, member_scope.as_deref());
        let anchor = normalize_anchor(anchor.unwrap_or(""));
        self.store.claim(
            &ClaimRequest::new((room, &who.member), path, ttl, reason)
                .scoped(&scope)
                .anchored(&anchor),
            self.now(),
        )?;
        // a member that claims is back at work: a `done` it set earlier no longer holds (#260)
        self.store
            .clear_done(Seat::new(room, &who.member), self.now())
    }

    /// The `possible_overwrite` stuck events of `room` (#909): a claim taken, inside the window, on a path another member
    /// released after a claim deadlock. By path, then names. Flag only; read only.
    pub(super) fn overwrite_events(
        &self,
        room: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::StuckEvent>> {
        Ok(self
            .store
            .claim_overwrites(room, now)?
            .into_iter()
            .map(
                |(path, previous, new)| crate::domain::StuckEvent::PossibleOverwrite {
                    room: room.to_string(),
                    path: crate::domain::claim_label(&path),
                    previous,
                    new,
                },
            )
            .collect())
    }

    pub fn release(&self, who: &Principal, room: &str, path: &str) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        let path = crate::domain::canonical_claim_path(path);
        let path = path.as_str();
        self.authorize(who, &Operation::release(room, path))?;
        self.store
            .note_yield_release(room, &who.member, path, self.now())?;
        self.store.release(room, &who.member, path)
    }

    /// `release` with a note on what the caller changed (#272): shown to members who saw the claim and returned to
    /// the next claimant. A note is one line of at most 300 characters.
    pub fn release_with_note(
        &self,
        who: &Principal,
        (room, path): (&str, &str),
        note: &str,
    ) -> Result<()> {
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        let path = crate::domain::canonical_claim_path(path);
        let path = path.as_str();
        self.refuse_text(who, (room, "release"), note)?;
        let note = crate::domain::flat(note);
        if note.chars().count() > 300 {
            return Err(Error::Invalid(
                "a release note is at most 300 characters".into(),
            ));
        }
        self.authorize(who, &Operation::release(room, path))?;
        self.store
            .note_yield_release(room, &who.member, path, self.now())?;
        self.store.release_with(
            &Release::new((room, &who.member), path, note.trim()),
            self.now(),
        )
    }

    /// The latest release of a path that carried a note.
    pub fn last_release(
        &self,
        who: &Principal,
        room: &str,
        path: &str,
    ) -> Result<Option<crate::domain::ReleaseNote>> {
        self.admit(who, room, Operation::claims(room))?;
        check_path(path)?;
        self.store
            .last_release(room, &crate::domain::canonical_claim_path(path))
    }

    /// Release every claim the caller holds, in every room (its session ended, #277). Only ever the caller's own.
    pub fn release_all(&self, who: &Principal) -> Result<usize> {
        check_member(who)?;
        self.store.release_member_claims(&who.member, self.now())
    }

    /// The member's own refused claims that a claim being taken by someone else turned into a cycle of waits, each
    /// given once (#648). The digest carries the same notices; this reads them without the other effects of a digest
    /// (it renews nothing and moves no cursor).
    pub fn cycle_notices(
        &self,
        who: &Principal,
        room: &str,
    ) -> Result<Vec<crate::domain::CycleNotice>> {
        self.admit(who, room, Operation::claims(room))?;
        self.store.deadlock_notices(room, &who.member, self.now())
    }

    pub fn claims(&self, who: &Principal, room: &str) -> Result<Vec<Claim>> {
        self.admit(who, room, Operation::claims(room))?;
        self.settle_leases(&who.member)?;
        self.store.claims(room, self.now())
    }
}
