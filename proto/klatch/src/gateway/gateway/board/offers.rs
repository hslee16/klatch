//! The offers section of the board.

use super::members::Roster;
use super::*;

impl<S: Store> Gateway<S> {
    /// The current offers in the board words `messages`. An offer follows its sender's presence: a member that has
    /// gone quiet is not available, and a member that keeps waiting (which refreshes presence) keeps its offer with
    /// no renewal.
    pub(super) fn board_offers(
        &self,
        messages: &[Message],
        now: i64,
        roster: &Roster,
    ) -> Vec<crate::domain::Offer> {
        crate::domain::current_offers(messages, now, crate::domain::OFFER_TTL_SECS)
            .into_iter()
            .filter(|o| !roster.done.contains(&o.sender) && !roster.absent.contains(&o.sender))
            .collect()
    }
}
