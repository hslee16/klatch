//! Open asks and the stuck cases of a room.

use super::*;

impl<S: Store> Gateway<S> {
    /// Asks in a room nobody has answered or closed.
    pub fn open_asks(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::OpenAsk>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store.open_asks(room, self.now())
    }

    /// The stuck cases of a room as structured events (#271): stale asks, asks nobody capable took, cycles through asks,
    /// cycles of claim waits, a long two-member exchange with no dm (#1191) a live claim whose holder's process is provably gone (#1182) and a claim taken on a path just released after a deadlock yield (#909). A read like `open_asks`, guarded the same way (so a room above the caller's clearance
    /// refuses it), and it carries ids, member names and numbers, never message text.
    pub fn stuck(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::StuckEvent>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.sweep_ask_routes(room);
        let now = self.now();
        let asks = self.store.open_asks(room, now)?;
        let claims = self.store.claims(room, now)?;
        let cycles = self.store.wait_cycles(room, now)?;
        let stale_after = self.settings_of(room)?.stale_secs;
        let mut events = crate::domain::stuck_events(&asks, &claims, &cycles, now, stale_after);
        events.extend(self.exchange_event(room)?);
        events.extend(self.dead_holder_events(room, &claims)?);
        events.extend(self.overwrite_events(room, now)?);
        Ok(events)
    }
}
