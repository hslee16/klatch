//! The receiver budget (R22): what a room will take in from one member.

use super::*;

impl<S: Store> Gateway<S> {
    /// The receiver budget (R22): a size cap, a burst limit and a refusal of an identical repeat, each with a stated
    /// reason. Looks only at the room's recent messages.
    pub(super) fn check_budget(&self, who: &Principal, room: &str, body: &str) -> Result<()> {
        use crate::domain::budget::*;
        if body.len() > MAX_POST_BYTES {
            return Err(Error::Invalid(format!(
                "message too large: {} bytes, at most {MAX_POST_BYTES}; put the detail in a file and post where it is",
                body.len()
            )));
        }
        let now = self.now();
        let head = self.store.head(room)?;
        let recent = self.store.tail(
            room,
            (head - RECENT_LOOKBACK).max(0),
            RECENT_LOOKBACK as usize,
        )?;
        let mine = recent.iter().filter(|m| {
            m.sender == who.member && now - m.ts < BURST_WINDOW_SECS.max(DUPLICATE_WINDOW_SECS)
        });
        let mut in_window = 0;
        let mut oldest_in_window = now;
        for m in mine.clone() {
            if now - m.ts < DUPLICATE_WINDOW_SECS && m.body.trim() == body.trim() {
                return Err(Error::Invalid(format!(
                    "same as your message #{} posted {}s ago; say something new or do not repeat it",
                    m.seq,
                    crate::domain::secs_between(now, m.ts)
                )));
            }
            if now - m.ts < BURST_WINDOW_SECS {
                in_window += 1;
                oldest_in_window = oldest_in_window.min(m.ts);
            }
        }
        if in_window >= MAX_POSTS_PER_WINDOW {
            return Err(Error::Invalid(format!(
                "too many posts: at most {MAX_POSTS_PER_WINDOW} posts in {BURST_WINDOW_SECS} seconds in one room; wait {}s",
                (BURST_WINDOW_SECS - (now - oldest_in_window)).max(1)
            )));
        }
        Ok(())
    }
}
