//! Open asks and the stuck cases of a room.

use super::*;

impl<S: Store> Gateway<S> {
    /// Asks in a room nobody has answered or closed.
    pub fn open_asks(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::OpenAsk>> {
        self.admit(who, room, Operation::read(room, false))?;
        self.store.open_asks(room, self.now())
    }

    /// The stuck cases of a room as structured events (#271): stale asks, asks nobody capable took, cycles through asks
    /// and cycles of claim waits. A read like `open_asks`, guarded the same way (so a room above the caller's clearance
    /// refuses it), and it carries ids, member names and numbers, never message text.
    pub fn stuck(&self, who: &Principal, room: &str) -> Result<Vec<crate::domain::StuckEvent>> {
        self.admit(who, room, Operation::read(room, false))?;
        let now = self.now();
        let asks = self.store.open_asks(room, now)?;
        let claims = self.store.claims(room, now)?;
        let cycles = self.store.wait_cycles(room, now)?;
        let stale_after = self.settings_of(room)?.stale_secs;
        Ok(crate::domain::stuck_events(
            &asks,
            &claims,
            &cycles,
            now,
            stale_after,
        ))
    }
}
