//! Delivery receipts for the sender of a message (#546).

use super::*;

impl<S: Store> Gateway<S> {
    /// What the sender of message `seq` can learn about each recipient (#546): liveness states only (parked, seen
    /// after the post, not seen), never a cursor or a hook shown-mark, and only for the sender's own message. The
    /// text of the message is not returned.
    pub fn receipt(
        &self,
        who: &Principal,
        room: &str,
        seq: i64,
    ) -> Result<crate::domain::Receipts> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        let not_found = || Error::NotFound(format!("no message #{seq} in {room}"));
        if seq < 1 {
            return Err(not_found());
        }
        let message = self
            .store
            .tail(room, seq - 1, 1)?
            .into_iter()
            .find(|m| {
                m.seq == seq && m.kind.as_deref() != Some(super::super::tripwire::CANARY_KIND)
            })
            .ok_or_else(not_found)?;
        if message.sender != who.member {
            return Err(Error::Refused {
                rule: "receipt-sender-only".into(),
                reason: "only the sender of a message can ask for its receipts".into(),
            });
        }
        let members = self.store.members(room)?;
        let cursors = self.store.member_cursors(room)?;
        Ok(crate::domain::receipts_of(
            &members,
            &cursors,
            &message,
            self.now(),
        ))
    }
}
