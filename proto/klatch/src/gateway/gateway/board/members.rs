//! The members section of the board, and `who`.

use super::*;

/// The members of a board: who is listed, and who is left out because they are done or have gone quiet.
pub(super) struct Roster {
    pub(super) members: Vec<crate::domain::BoardMember>,
    /// Members whose status says done.
    pub(super) done: Vec<String>,
    /// Members not seen for a while and not parked.
    pub(super) absent: Vec<String>,
    /// How many members are counted instead of listed.
    pub(super) stale: usize,
}

impl<S: Store> Gateway<S> {
    /// The roster of a room as the board shows it: the recent members with their last announced status (`messages`
    /// are the board words), the done ones and the absent ones.
    pub(super) fn board_roster(
        &self,
        room: &str,
        now: i64,
        messages: &[Message],
    ) -> Result<Roster> {
        let statuses = crate::domain::statuses(messages);
        let mut all = self.store.members(room)?;
        self.with_kinds(&mut all)?;
        self.with_places(&mut all)?;
        self.with_advisory(room, &mut all)?;
        let done: Vec<String> = all
            .iter()
            .filter(|m| m.status.as_deref().is_some_and(|s| s.starts_with("done")))
            .map(|m| m.name.clone())
            .collect();
        let (recent, gone): (Vec<_>, Vec<_>) = all.into_iter().partition(|m| {
            now - m.last_seen <= crate::domain::MEMBER_ACTIVE_SECS
                || m.parked.as_ref().is_some_and(|p| p.holds(now))
        });
        let absent: Vec<String> = gone.iter().map(|m| m.name.clone()).collect();
        let members = recent
            .into_iter()
            .map(|m| crate::domain::BoardMember {
                status_ts: m.status_ts,
                status: m.status.clone().or_else(|| {
                    statuses
                        .iter()
                        .find(|s| s.member == m.name)
                        .map(|s| s.status.clone())
                }),
                name: m.name,
                last_seen: m.last_seen,
                parent: m.parent,
                parked: m.parked.filter(|p| p.holds(now)),
                kind: m.kind,
                place: m.place,
                advisory: m.advisory,
            })
            .collect::<Vec<crate::domain::BoardMember>>();
        Ok(Roster {
            members,
            done,
            absent,
            stale: gone.len(),
        })
    }

    pub fn who(&self, who: &Principal, room: &str) -> Result<Vec<MemberInfo>> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        self.settle_leases(&who.member)?;
        let mut members = self.store.members(room)?;
        self.with_kinds(&mut members)?;
        self.with_places(&mut members)?;
        self.with_modes(&mut members)?;
        self.with_advisory(room, &mut members)?;
        Ok(members)
    }
}
