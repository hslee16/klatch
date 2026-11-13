use crate::{Digest, Gateway};
use klatch_core::{Claim, MemberInfo, Message, Posted, Principal, Result};
use klatch_store::Store;
use std::rc::Rc;

/// The client side of the gateway, bound to one identity when it is created. Callers never pass who they
/// are per call, so a remote transport can take identity from the connection (DESIGN section 20 rule 11).
pub trait Transport {
    fn join(&self, room: &str) -> Result<()>;
    fn post(&self, room: &str, body: &str) -> Result<Posted>;
    fn read(&self, room: &str, advance: bool) -> Result<Vec<Message>>;
    fn who(&self, room: &str) -> Result<Vec<MemberInfo>>;
    fn claim(&self, room: &str, path: &str, ttl: i64, reason: &str) -> Result<()>;
    fn release(&self, room: &str, path: &str) -> Result<()>;
    fn claims(&self, room: &str) -> Result<Vec<Claim>>;
    fn digest(&self) -> Result<Digest>;
}

/// In-process transport: calls the gateway directly. No daemon, no socket.
pub struct Direct<S: Store> {
    gateway: Rc<Gateway<S>>,
    principal: Principal,
}

impl<S: Store> Direct<S> {
    pub fn new(gateway: Rc<Gateway<S>>, principal: Principal) -> Self {
        Self { gateway, principal }
    }
}

impl<S: Store> Transport for Direct<S> {
    fn join(&self, room: &str) -> Result<()> {
        self.gateway.join(&self.principal, room)
    }
    fn post(&self, room: &str, body: &str) -> Result<Posted> {
        self.gateway.post(&self.principal, room, body)
    }
    fn read(&self, room: &str, advance: bool) -> Result<Vec<Message>> {
        self.gateway.read(&self.principal, room, advance)
    }
    fn who(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.gateway.who(&self.principal, room)
    }
    fn claim(&self, room: &str, path: &str, ttl: i64, reason: &str) -> Result<()> {
        self.gateway.claim(&self.principal, room, path, ttl, reason)
    }
    fn release(&self, room: &str, path: &str) -> Result<()> {
        self.gateway.release(&self.principal, room, path)
    }
    fn claims(&self, room: &str) -> Result<Vec<Claim>> {
        self.gateway.claims(&self.principal, room)
    }
    fn digest(&self) -> Result<Digest> {
        self.gateway.digest(&self.principal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SpyStore;
    use crate::{AllowAll, FixedClock};

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
        alice.claim("r", "src", 60, "").unwrap();
        assert!(matches!(
            bob.claim("r", "src/a.rs", 60, ""),
            Err(klatch_core::Error::Conflict { .. })
        ));
        assert_eq!(bob.claims("r").unwrap()[0].owner, "alice");
        alice.release("r", "src").unwrap();
        bob.claim("r", "src/a.rs", 60, "").unwrap();
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
}
