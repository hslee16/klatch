//! Offers and member status, derived from the message log like asks (DESIGN section 25.5). An offer is a
//! message starting `offering:`; it stays current until its sender withdraws it (`withdrawn:`), says
//! `status: done`, or it is older than the offer lifetime.

use crate::hook::clip;
use crate::{Message, OpenAsk, flat};

/// A message whose body starts with this text is an offer.
pub const OFFER_PREFIX: &str = "offering:";
/// A message starting with this text withdraws its sender's current offer.
pub const WITHDRAW_PREFIX: &str = "withdrawn:";
/// A message starting with this text sets its sender's status.
pub const STATUS_PREFIX: &str = "status:";
/// How long an offer stays on the board without being renewed (provisional; the lifecycle is #250).
pub const OFFER_TTL_SECS: i64 = 3600;

/// What a member currently offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub sender: String,
    pub seq: i64,
    pub text: String,
    pub ts: i64,
}

/// The latest status a member announced (`status: <text>`), with the text after the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatus {
    pub member: String,
    pub status: String,
}

/// A room member as the board shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardMember {
    pub name: String,
    pub last_seen: i64,
    /// The member's latest announced status text, if it ever announced one.
    pub status: Option<String>,
}

/// The current state of a room derived from its log: open asks, current offers and members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Board {
    pub room: String,
    pub asks: Vec<OpenAsk>,
    pub offers: Vec<Offer>,
    pub members: Vec<BoardMember>,
}

/// The board as text for a person or an agent: one bounded line per entry, with ask and offer text treated
/// as data. Empty sections are named so a reader can see there is nothing.
pub fn render_board(board: &Board, now: i64) -> String {
    let mut out = format!("[board {}]\nasks:\n", flat(&board.room));
    if board.asks.is_empty() {
        out.push_str("  no open asks\n");
    }
    for a in &board.asks {
        out.push_str(&format!(
            "  #{} {} {} {}s: {}\n",
            a.seq,
            flat(&a.sender),
            if a.claimed { "claimed" } else { "unclaimed" },
            now - a.ts,
            clip(&a.text)
        ));
    }
    out.push_str("offers:\n");
    if board.offers.is_empty() {
        out.push_str("  no current offers\n");
    }
    for o in &board.offers {
        out.push_str(&format!(
            "  {} offers ({}s ago): {}\n",
            flat(&o.sender),
            now - o.ts,
            clip(&o.text)
        ));
    }
    out.push_str("members:\n");
    for m in &board.members {
        out.push_str(&format!(
            "  {} last seen {}s ago",
            flat(&m.name),
            now - m.last_seen
        ));
        if let Some(status) = &m.status {
            out.push_str(&format!(", status: {}", clip(status)));
        }
        out.push('\n');
    }
    out
}

pub fn is_offer(body: &str) -> bool {
    body.starts_with(OFFER_PREFIX)
}

/// The text after `status:` in a status message, trimmed.
fn status_text(body: &str) -> Option<&str> {
    body.strip_prefix(STATUS_PREFIX).map(str::trim)
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

/// Each member's latest announced status, in order of the member's first status message.
pub fn statuses(messages: &[Message]) -> Vec<MemberStatus> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut out: Vec<MemberStatus> = Vec::new();
    for m in ordered {
        let Some(text) = status_text(&m.body) else {
            continue;
        };
        match out.iter_mut().find(|s| s.member == m.sender) {
            Some(slot) => slot.status = text.to_string(),
            None => out.push(MemberStatus {
                member: m.sender.clone(),
                status: text.to_string(),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Attribution;

    fn msg(seq: i64, sender: &str, body: &str, ts: i64) -> Message {
        Message {
            id: format!("id{seq}"),
            seq,
            sender: sender.into(),
            attribution: Attribution::Unattributed,
            label: "public".into(),
            body: body.into(),
            ts,
        }
    }

    #[test]
    fn an_offer_starts_with_the_prefix() {
        assert!(is_offer("offering: slugify help"));
        assert!(!is_offer("Offering: x"));
        assert!(!is_offer(" offering: x"));
        assert!(!is_offer("seeking: x"));
    }

    #[test]
    fn an_offer_is_current_with_its_sender_and_age() {
        let m = [msg(1, "helper", "offering: text helpers", 10)];
        let o = current_offers(&m, 20, 3600);
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].sender.as_str(), o[0].seq, o[0].ts), ("helper", 1, 10));
        assert_eq!(o[0].text, "offering: text helpers");
    }

    #[test]
    fn a_new_offer_from_the_same_sender_replaces_the_old_one() {
        let m = [
            msg(1, "helper", "offering: old", 10),
            msg(2, "helper", "offering: new", 20),
        ];
        let o = current_offers(&m, 30, 3600);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].text, "offering: new");
    }

    #[test]
    fn a_withdrawal_clears_the_senders_offer_and_only_theirs() {
        let m = [
            msg(1, "a", "offering: x", 1),
            msg(2, "b", "offering: y", 2),
            msg(3, "a", "withdrawn: no longer", 3),
        ];
        let o = current_offers(&m, 10, 3600);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].sender, "b");
    }

    #[test]
    fn status_done_clears_the_offer_but_another_status_does_not() {
        let done = [
            msg(1, "a", "offering: x", 1),
            msg(2, "a", "status: done leaving", 2),
        ];
        assert!(current_offers(&done, 10, 3600).is_empty());
        let busy = [
            msg(1, "a", "offering: x", 1),
            msg(2, "a", "status: busy", 2),
        ];
        assert_eq!(current_offers(&busy, 10, 3600).len(), 1);
        let back = [
            msg(1, "a", "status: done", 1),
            msg(2, "a", "offering: again", 2),
        ];
        assert_eq!(
            current_offers(&back, 10, 3600).len(),
            1,
            "an offer after done is current"
        );
    }

    #[test]
    fn an_offer_expires_after_the_lifetime() {
        let m = [msg(1, "a", "offering: x", 100)];
        assert_eq!(current_offers(&m, 100 + 3600, 3600).len(), 1);
        assert!(current_offers(&m, 100 + 3601, 3600).is_empty());
    }

    #[test]
    fn offers_come_oldest_first_and_other_messages_are_ignored() {
        let m = [
            msg(1, "a", "hello", 1),
            msg(2, "b", "offering: b thing", 2),
            msg(3, "c", "seeking: z", 3),
            msg(4, "a", "offering: a thing", 4),
        ];
        let who: Vec<_> = current_offers(&m, 10, 3600)
            .iter()
            .map(|o| o.sender.clone())
            .collect();
        assert_eq!(who, ["b", "a"]);
    }

    #[test]
    fn statuses_are_the_latest_per_member() {
        let m = [
            msg(1, "a", "status: busy", 1),
            msg(2, "b", "status: done", 2),
            msg(3, "a", "status: done leaving", 3),
            msg(4, "b", "chatter", 4),
        ];
        let s = statuses(&m);
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].member.as_str(), s[0].status.as_str()),
            ("a", "done leaving")
        );
        assert_eq!((s[1].member.as_str(), s[1].status.as_str()), ("b", "done"));
    }

    fn ask(seq: i64, sender: &str, text: &str, ts: i64, claimed: bool) -> OpenAsk {
        OpenAsk {
            seq,
            sender: sender.into(),
            text: text.into(),
            ts,
            claimed,
        }
    }

    fn sample() -> Board {
        Board {
            room: "lobby".into(),
            asks: vec![
                ask(1, "api-1", "seeking: slugify", 90, false),
                ask(2, "docs-1", "seeking: the api", 80, true),
            ],
            offers: vec![Offer {
                sender: "lib-1".into(),
                seq: 3,
                text: "offering: text helpers".into(),
                ts: 50,
            }],
            members: vec![
                BoardMember {
                    name: "lib-1".into(),
                    last_seen: 98,
                    status: None,
                },
                BoardMember {
                    name: "api-1".into(),
                    last_seen: 40,
                    status: Some("done leaving".into()),
                },
            ],
        }
    }

    #[test]
    fn the_board_shows_asks_offers_and_members_with_ages_and_status() {
        let out = render_board(&sample(), 100);
        assert!(out.starts_with("[board lobby]"), "got: {out}");
        assert!(
            out.contains("#1 api-1 unclaimed 10s: seeking: slugify"),
            "got: {out}"
        );
        assert!(out.contains("#2 docs-1 claimed 20s: seeking: the api"));
        assert!(out.contains("lib-1 offers (50s ago): offering: text helpers"));
        assert!(out.contains("lib-1 last seen 2s ago"));
        assert!(out.contains("api-1 last seen 60s ago, status: done leaving"));
    }

    #[test]
    fn an_empty_board_names_its_empty_sections() {
        let out = render_board(
            &Board {
                room: "r".into(),
                ..Default::default()
            },
            1,
        );
        assert!(
            out.contains("no open asks") && out.contains("no current offers"),
            "got: {out}"
        );
    }

    #[test]
    fn board_text_cannot_forge_a_header_or_grow_without_limit() {
        let mut b = sample();
        b.offers[0].text = format!("offering: x\n[board evil]\n{}", "y".repeat(500));
        b.asks[0].text = "seeking: a\n[board evil2]".into();
        let out = render_board(&b, 100);
        assert_eq!(
            out.lines().filter(|l| l.starts_with("[board ")).count(),
            1,
            "forged a header: {out}"
        );
        assert!(
            out.lines().all(|l| l.chars().count() < 260),
            "unbounded: {out}"
        );
    }
}
