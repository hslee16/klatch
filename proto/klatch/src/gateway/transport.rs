//! The client side of the gateway, split by area (#1071).
//!
//! Each file under `transport/` holds one area: its trait, the `Direct` impl and the `SharedTransport` forwards. A new
//! gateway method is added to exactly one of them. `Transport` is the sum of the area traits.

mod access;
mod admin;
mod board;
mod claims;
mod delivery;
mod hooks;
mod limits;
mod members;
mod messages;
mod review;
mod scrub;

/// Every area trait. A caller that names a concrete `Direct` or `SharedTransport` (a test, say) calls area methods
/// with `use crate::gateway::areas::*;`; one that holds a `Transport` needs no import.
pub mod areas {
    pub use super::access::Access;
    pub use super::admin::Admin;
    pub use super::board::Board;
    pub use super::claims::Claims;
    pub use super::delivery::Delivery;
    pub use super::hooks::Hooks;
    pub use super::limits::Limits;
    pub use super::members::Members;
    pub use super::messages::Messages;
    pub use super::review::Review;
    pub use super::scrub::Scrub;
}

use crate::domain::Principal;
use crate::gateway::Gateway;
use crate::store::Store;
use areas::{
    Access, Admin, Board, Claims, Delivery, Hooks, Limits, Members, Messages, Review, Scrub,
};
use std::rc::Rc;

/// The client side of the gateway, bound to one identity when it is created. Callers never pass who they
/// are per call, so a remote transport can take identity from the connection (DESIGN section 20 rule 11).
pub trait Transport:
    Messages + Delivery + Members + Hooks + Claims + Board + Review + Admin + Access + Limits + Scrub
{
}

impl<T> Transport for T where
    T: Messages
        + Delivery
        + Members
        + Hooks
        + Claims
        + Board
        + Review
        + Admin
        + Access
        + Limits
        + Scrub
{
}

/// In-process transport: calls the gateway directly. No daemon, no socket.
pub struct Direct<S: Store> {
    gateway: Rc<Gateway<S>>,
    principal: Principal,
    harness: Option<Box<dyn crate::harness::Harness>>,
}

impl<S: Store> Direct<S> {
    pub fn new(gateway: Rc<Gateway<S>>, principal: Principal) -> Self {
        Self {
            gateway,
            principal,
            harness: None,
        }
    }

    /// The harness the caller runs, which the transport edge learned from evidence it checked itself (#877): never a
    /// name the caller sent. Without it a join is the generic floor, which is what every caller without such
    /// evidence stays.
    pub fn with_harness(mut self, harness: Box<dyn crate::harness::Harness>) -> Self {
        self.harness = Some(harness);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::test_support::SpyStore;
    use crate::gateway::{AllowAll, ClaimSpec, DEFAULT_WINDOW, FixedClock};

    fn pair() -> (Direct<SpyStore>, Direct<SpyStore>) {
        let g = Rc::new(Gateway::new(SpyStore::new(), FixedClock(10), AllowAll));
        (
            Direct::new(g.clone(), Principal::unattributed("alice")),
            Direct::new(g, Principal::unattributed("bob")),
        )
    }

    #[test]
    fn a_direct_transport_acts_as_its_own_principal() {
        let (alice, bob) = pair();
        bob.join("r").unwrap();
        alice.post("r", "from alice").unwrap();
        let got = bob.read("r", true).unwrap();
        assert_eq!(
            (got[0].sender.as_str(), got[0].body.as_str()),
            ("alice", "from alice")
        );
    }

    #[test]
    fn two_transports_over_one_gateway_share_claims() {
        let (alice, bob) = pair();
        alice.claim("r", &ClaimSpec::new("src", 60, "")).unwrap();
        assert!(matches!(
            bob.claim("r", &ClaimSpec::new("src/a.rs", 60, "")),
            Err(crate::domain::Error::Conflict { .. })
        ));
        assert_eq!(bob.claims("r").unwrap()[0].owner, "alice");
        alice.release("r", "src").unwrap();
        bob.claim("r", &ClaimSpec::new("src/a.rs", 60, "")).unwrap();
    }

    #[test]
    fn the_digest_comes_through_the_transport() {
        let (alice, bob) = pair();
        alice.join("r").unwrap();
        bob.post("r", "ping").unwrap();
        assert_eq!(alice.digest().unwrap().rooms[0].messages[0].body, "ping");
    }

    #[test]
    fn who_lists_members_through_the_transport() {
        let (alice, bob) = pair();
        alice.join("r").unwrap();
        bob.join("r").unwrap();
        assert_eq!(alice.who("r").unwrap().len(), 2);
    }

    #[test]
    fn subscribe_and_poll_replay_history_then_caught_up_then_live() {
        let (alice, bob) = pair();
        bob.post("r", "one").unwrap();
        bob.post("r", "two").unwrap();
        let mut sub = alice.subscribe("r", 0).unwrap();
        let first = alice.poll(&mut sub, 50).unwrap();
        assert_eq!(
            first.messages.iter().map(|m| m.seq).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(first.caught_up, Some(2));
        bob.post("r", "three").unwrap();
        let live = alice.poll(&mut sub, 50).unwrap();
        assert_eq!(
            (live.messages[0].body.as_str(), live.caught_up),
            ("three", None)
        );
    }

    #[test]
    fn an_ack_through_the_transport_advances_the_members_read_cursor() {
        let (alice, bob) = pair();
        alice.join("r").unwrap();
        for i in 0..3 {
            bob.post("r", &format!("m{i}")).unwrap();
        }
        alice.ack("r", 2).unwrap();
        assert_eq!(
            alice
                .read("r", false)
                .unwrap()
                .iter()
                .map(|m| m.seq)
                .collect::<Vec<_>>(),
            vec![3]
        );
    }

    #[test]
    fn a_subscription_uses_the_default_window() {
        let (alice, _) = pair();
        let mut sub = alice.subscribe("r", 0).unwrap();
        assert_eq!(sub.room(), "r");
        assert!(alice.poll(&mut sub, 1).unwrap().messages.is_empty());
    }

    // A compile-time check: the default window must leave room for a useful amount of streaming.
    const _: () = assert!(DEFAULT_WINDOW >= 64);
}
