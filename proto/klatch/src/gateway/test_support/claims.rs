//! The claim and ask parts of the spy store.

use super::*;

impl ClaimStore for SpyStore {
    fn claim(&self, request: &ClaimRequest, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.claim(request, now)
    }

    fn take_passive_claim(
        &self,
        request: &ClaimRequest,
        now: i64,
    ) -> Result<crate::domain::PassiveTaken> {
        self.gate()?;
        self.inner.take_passive_claim(request, now)
    }

    fn release_passive_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.gate()?;
        self.inner.release_passive_claims(member, now)
    }

    fn note_passive_failure(&self, now: i64) {
        self.inner.note_passive_failure(now);
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.gate()?;
        self.inner.release(room, owner, path)
    }

    fn release_at(&self, room: &str, owner: &str, path: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.release_at(room, owner, path, now)
    }

    fn release_with(&self, release: &Release, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.release_with(release, now)
    }

    fn last_release(&self, room: &str, path: &str) -> Result<Option<crate::domain::ReleaseNote>> {
        self.gate()?;
        self.inner.last_release(room, path)
    }

    fn release_member_claims(&self, member: &str, now: i64) -> Result<usize> {
        self.gate()?;
        self.inner.release_member_claims(member, now)
    }

    fn claim_owners(&self) -> Result<Vec<String>> {
        self.gate()?;
        self.inner.claim_owners()
    }

    fn retime_member_claims(&self, member: &str, until: i64, extend: bool) -> Result<usize> {
        self.gate()?;
        self.inner.retime_member_claims(member, until, extend)
    }

    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        self.gate()?;
        self.inner.claims(room, now)
    }

    fn release_anchored(&self, anchor: &str, now: i64) -> Result<Vec<(String, String, String)>> {
        self.inner.release_anchored(anchor, now)
    }

    fn anchors_in_use(&self, now: i64) -> Result<Vec<String>> {
        self.inner.anchors_in_use(now)
    }

    fn claim_changes(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<crate::domain::ClaimChanges> {
        self.gate()?;
        self.inner.claim_changes(room, member, now)
    }

    fn claim_ages(
        &self,
        room: &str,
        owner: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::AgedClaim>> {
        self.gate()?;
        self.inner.claim_ages(room, owner, now)
    }

    fn wait_cycles(&self, room: &str, now: i64) -> Result<Vec<Vec<String>>> {
        self.gate()?;
        self.inner.wait_cycles(room, now)
    }

    fn note_yield_release(&self, room: &str, owner: &str, path: &str, now: i64) -> Result<()> {
        self.gate()?;
        self.inner.note_yield_release(room, owner, path, now)
    }

    fn claim_overwrites(&self, room: &str, now: i64) -> Result<Vec<(String, String, String)>> {
        self.gate()?;
        self.inner.claim_overwrites(room, now)
    }

    fn lapsed_claims(
        &self,
        room: &str,
        now: i64,
        window: (i64, usize),
    ) -> Result<Vec<(String, String, i64)>> {
        self.gate()?;
        self.inner.lapsed_claims(room, now, window)
    }

    fn deadlock_notices(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<Vec<crate::domain::CycleNotice>> {
        self.gate()?;
        self.inner.deadlock_notices(room, member, now)
    }
}

impl AskStore for SpyStore {
    fn stale_asks(&self, at: Seat, now: i64, after: i64) -> Result<Vec<crate::domain::OpenAsk>> {
        self.gate()?;
        self.inner.stale_asks(at, now, after)
    }

    fn open_asks(&self, room: &str, now: i64) -> Result<Vec<crate::domain::OpenAsk>> {
        self.gate()?;
        self.inner.open_asks(room, now)
    }

    fn lapsed_asks(
        &self,
        room: &str,
        member: &str,
        now: i64,
    ) -> Result<Vec<(crate::domain::OpenAsk, String)>> {
        self.gate()?;
        self.inner.lapsed_asks(room, member, now)
    }

    fn rooms_with_prefix(&self, prefix: &str) -> Result<Vec<crate::domain::AskRoom>> {
        self.gate()?;
        self.inner.rooms_with_prefix(prefix)
    }

    fn route_ask(
        &self,
        room: &str,
        seq: i64,
        to: &[(String, crate::domain::RouteKind)],
        now: i64,
    ) -> Result<bool> {
        self.gate()?;
        self.inner.route_ask(room, seq, to, now)
    }

    fn add_ask_routes(
        &self,
        room: &str,
        seq: i64,
        to: &[(String, crate::domain::RouteKind)],
        now: i64,
    ) -> Result<()> {
        self.gate()?;
        self.inner.add_ask_routes(room, seq, to, now)
    }

    fn ask_routes(&self, room: &str) -> Result<Vec<crate::domain::AskRoute>> {
        self.gate()?;
        self.inner.ask_routes(room)
    }

    fn note_route_outcome(
        &self,
        room: &str,
        seq: i64,
        outcome: &str,
        n: i64,
        now: i64,
    ) -> Result<bool> {
        self.gate()?;
        self.inner.note_route_outcome(room, seq, outcome, n, now)
    }

    fn untold_routes(&self, room: &str, member: &str, bit: i64) -> Result<Vec<i64>> {
        self.gate()?;
        self.inner.untold_routes(room, member, bit)
    }

    fn mark_told(&self, room: &str, member: &str, seqs: &[i64], bit: i64) -> Result<()> {
        self.gate()?;
        self.inner.mark_told(room, member, seqs, bit)
    }

    fn route_events(&self, room: &str) -> Result<Vec<(i64, String, i64)>> {
        self.gate()?;
        self.inner.route_events(room)
    }

    fn route_outcomes(&self, room: &str, seq: i64) -> Result<Vec<String>> {
        self.gate()?;
        self.inner.route_outcomes(room, seq)
    }

    fn set_ask_scope(&self, room: &str, seq: i64, scope: &str) -> Result<()> {
        self.gate()?;
        self.inner.set_ask_scope(room, seq, scope)
    }

    fn ask_scopes(&self, room: &str) -> Result<Vec<(i64, String)>> {
        self.gate()?;
        self.inner.ask_scopes(room)
    }

    fn set_ask_refs(&self, room: &str, seq: i64, refs: &[crate::domain::AskRef]) -> Result<()> {
        self.gate()?;
        self.inner.set_ask_refs(room, seq, refs)
    }

    fn ask_refs(&self, room: &str) -> Result<Vec<(i64, Vec<crate::domain::AskRef>)>> {
        self.gate()?;
        self.inner.ask_refs(room)
    }
}
