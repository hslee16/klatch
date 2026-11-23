//! The machine-derived member status (#1403): what the opt-in PostToolUse and Stop hooks write to the member's status
//! field, in the rooms it can read, instead of the member posting `working:` lines. State only: no message is posted,
//! and nothing of a tool call (its name, command or content) reaches this code.

use super::*;
use crate::domain::{MACHINE_STATUS_WINDOW_SECS, MachineClass, machine_owned};

impl<S: Store> Gateway<S> {
    /// Set the caller's machine status of `class` (`working`, `idle` or `parked`) in every room it has joined and may
    /// read, where it is due: a status the member set itself stays, and a repeat inside the window is not written.
    /// Posts nothing. Returns how many rooms were written.
    pub fn set_machine_status(&self, who: &Principal, class: &str) -> Result<usize> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        let class = MachineClass::from_word(class)
            .ok_or_else(|| Error::Invalid("bad machine status class".into()))?;
        let (now, member) = (self.now(), who.member.as_str());
        let mut written = 0;
        for room in self.store.rooms_of(member)? {
            // a room the member cannot read, or may not set a status in, gets nothing; the refusal is not recorded,
            // since it is the hook's doing and not the member's
            let op = Operation::Status {
                room: room.clone(),
                status: class.text().into(),
            };
            if self.check_clearance(who, &room).is_err() || self.check_guard(who, &op).is_err() {
                continue;
            }
            let put = self.store.put_machine_status(
                Seat::new(&room, member),
                (class.text(), class.word()),
                &machine_owned(),
                (now, MACHINE_STATUS_WINDOW_SECS),
            )?;
            written += usize::from(put);
        }
        Ok(written)
    }
}
