//! Offers: a message starting `offering:` stays current until its sender withdraws it or says done, or it is older
//! than the backstop lifetime.

use super::members::status_text;
use super::{BOARD_LIST_LIMIT, Board, Now, more, section};
use crate::domain::hook::clip;
use crate::domain::{Message, flat};

/// A message whose body starts with this text is an offer.
pub const OFFER_PREFIX: &str = "offering:";
/// A message starting with this text withdraws its sender's current offer.
pub const WITHDRAW_PREFIX: &str = "withdrawn:";
/// The longest an offer stays on the board whatever its sender does: a backstop. An offer normally follows its
/// sender's presence (section 25.8), so a member that stays present need not renew it.
pub const OFFER_TTL_SECS: i64 = 86_400;

/// What a member currently offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub sender: String,
    pub seq: i64,
    pub text: String,
    pub ts: i64,
}

/// The `offers:` section.
pub(super) fn render(board: &Board, now: Now, out: &mut String) {
    let none = board.offers.is_empty().then_some("no current offers");
    section(out, "offers:", none);
    for o in board.offers.iter().take(BOARD_LIST_LIMIT) {
        out.push_str(&format!(
            "  {} offers ({}s ago): {}\n",
            flat(&o.sender),
            now.since(o.ts),
            clip(&o.text)
        ));
    }
    more(out, board.offers.len(), "offers");
}

/// Whether `body` is an offer.
pub fn is_offer(body: &str) -> bool {
    body.starts_with(OFFER_PREFIX)
}

/// The current offers in `messages` at time `now`, one per sender at most, oldest first.
pub fn current_offers(messages: &[Message], now: i64, ttl: i64) -> Vec<Offer> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut by_sender: Vec<(String, Option<Offer>)> = Vec::new();
    for m in ordered {
        let next = if is_offer(&m.body) {
            Some(Some(Offer {
                sender: m.sender.clone(),
                seq: m.seq,
                text: m.body.clone(),
                ts: m.ts,
            }))
        } else if m.body.starts_with(WITHDRAW_PREFIX)
            || status_text(&m.body).is_some_and(|t| t.starts_with("done"))
        {
            Some(None)
        } else {
            None
        };
        let Some(next) = next else { continue };
        match by_sender.iter_mut().find(|(s, _)| *s == m.sender) {
            Some(slot) => slot.1 = next,
            None => by_sender.push((m.sender.clone(), next)),
        }
    }
    let mut offers: Vec<Offer> = by_sender
        .into_iter()
        .filter_map(|(_, o)| o)
        .filter(|o| now - o.ts <= ttl)
        .collect();
    offers.sort_by_key(|o| o.seq);
    offers
}
