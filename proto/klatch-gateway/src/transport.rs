use crate::{Batch, ClaimSpec, DEFAULT_WINDOW, Digest, Gateway, Subscription};
use klatch_core::{Claim, MemberInfo, Message, Posted, Principal, Result};
use klatch_store::Store;
use std::rc::Rc;

/// The client side of the gateway, bound to one identity when it is created. Callers never pass who they
/// are per call, so a remote transport can take identity from the connection (DESIGN section 20 rule 11).
pub trait Transport {
    fn join(&self, room: &str) -> Result<()>;
    fn post(&self, room: &str, body: &str) -> Result<Posted> {
        self.post_labeled(room, body, None)
    }
    /// Send a direct message: post to the two-member room shared with `to`. Returns the room and the post.
    fn dm(&self, to: &str, body: &str, label: Option<&str>) -> Result<(String, Posted)>;
    /// Post under a label lower than the room's own (`None` for the room's).
    fn post_labeled(&self, room: &str, body: &str, label: Option<&str>) -> Result<Posted>;
    fn read(&self, room: &str, advance: bool) -> Result<Vec<Message>>;
    /// Unread messages a wait has not looked at yet; moves no cursor, refreshes presence.
    fn wait_poll(&self, room: &str) -> Result<Vec<Message>>;
    /// A wait has looked at the room up to `seq`; the messages stay unread.
    fn wait_seen(&self, room: &str, seq: i64) -> Result<()>;
    fn who(&self, room: &str) -> Result<Vec<MemberInfo>>;
    /// Take a claim (optionally shared with every scope).
    fn claim(&self, room: &str, spec: &ClaimSpec) -> Result<()>;
    /// Declare the scope the caller works in.
    fn set_scope(&self, scope: &str) -> Result<()>;
    /// The scope the caller declared or was given.
    fn scope(&self) -> Result<Option<String>>;
    fn release(&self, room: &str, path: &str) -> Result<()>;
    /// `release` with a note on what the caller changed.
    fn release_with_note(&self, room: &str, path: &str, note: &str) -> Result<()>;
    /// The latest release of a path that carried a note.
    fn last_release(&self, room: &str, path: &str) -> Result<Option<klatch_core::ReleaseNote>>;
    /// Release every claim the caller holds in every room (its session ended); returns how many.
    fn release_all(&self) -> Result<usize>;
    /// Administrative acts (`klatch admin`, R11); who may call them is the caller's decision, not the gateway's.
    fn admin_close_room(&self, room: &str, closed: bool) -> Result<()>;
    fn admin_pause(&self, paused: bool) -> Result<()>;
    fn admin_remove_member(&self, room: &str, member: &str) -> Result<()>;
    fn admin_allow_member(&self, room: &str, member: &str) -> Result<()>;
    fn admin_audit(&self, limit: usize) -> Result<Vec<klatch_core::AuditRecord>>;
    /// Labels and clearance (epic #9): a room's label, a member's clearance, the agent cap and the label order.
    fn admin_set_room_label(&self, room: &str, label: &str) -> Result<()>;
    fn admin_set_clearance(&self, member: &str, label: &str) -> Result<()>;
    fn admin_set_cap(&self, label: &str) -> Result<()>;
    fn admin_set_label_chain(&self, names: &[&str]) -> Result<()>;
    /// Plant a canary, a fake secret, in a room (#46); text carrying it into any other room trips a wire.
    fn admin_plant_canary(&self, room: &str) -> Result<()>;
    /// The scrubber (#44): list a private term under a rule name, stop it, and show what is listed.
    fn admin_add_scrub_term(&self, name: &str, term: &str) -> Result<()>;
    fn admin_remove_scrub_term(&self, name: &str) -> Result<()>;
    fn admin_scrub_terms(&self) -> Result<Vec<(String, String)>>;
    /// The latest refused calls, newest first.
    fn admin_refusals(&self, limit: usize) -> Result<Vec<klatch_core::RefusalRecord>>;
    fn claims(&self, room: &str) -> Result<Vec<Claim>>;
    fn digest(&self) -> Result<Digest>;
    /// `digest`, with the backlog of open unclaimed asks from others when `backlog` is set.
    fn digest_with(&self, backlog: bool) -> Result<Digest>;
    /// Record a hook run (event and bytes printed) for the caller.
    fn record_hook_run(&self, event: &str, bytes: usize) -> Result<()>;
    /// The latest run of each hook event recorded for `member`.
    fn hook_runs(&self, member: &str) -> Result<Vec<klatch_core::HookRun>>;
    /// Remember that the caller is the member of harness session `session_id`.
    fn note_session(&self, session_id: &str) -> Result<()>;
    /// Housekeeping: remove members not seen for `older_than` seconds; returns how many.
    fn prune_members(&self, older_than: i64) -> Result<usize>;
    /// Set the caller's own status in a room.
    fn set_status(&self, room: &str, status: &str) -> Result<()>;
    /// The rooms the caller has joined, read without consuming any message.
    fn rooms(&self) -> Result<Vec<String>>;
    /// Mute or unmute a room for the caller: its hook then shows only messages that mention the caller.
    fn set_muted(&self, room: &str, muted: bool) -> Result<()>;
    /// The room's board (open asks, current offers, members with status).
    fn board(&self, room: &str) -> Result<klatch_core::Board>;
    /// Asks in a room nobody has answered or closed.
    fn open_asks(&self, room: &str) -> Result<Vec<klatch_core::OpenAsk>>;
    /// Start a stream of a room from `from_seq` (0 for the beginning). Poll it with `poll`.
    fn subscribe(&self, room: &str, from_seq: i64) -> Result<Subscription>;
    /// The next batch of a subscription, at most `max` messages.
    fn poll(&self, subscription: &mut Subscription, max: usize) -> Result<Batch>;
    /// Acknowledge messages up to `seq`: advances this member's read cursor.
    fn ack(&self, room: &str, seq: i64) -> Result<()>;
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
    fn dm(&self, to: &str, body: &str, label: Option<&str>) -> Result<(String, Posted)> {
        self.gateway.dm(&self.principal, to, body, label)
    }
    fn post_labeled(&self, room: &str, body: &str, label: Option<&str>) -> Result<Posted> {
        self.gateway
            .post_labeled(&self.principal, room, body, label)
    }
    fn read(&self, room: &str, advance: bool) -> Result<Vec<Message>> {
        self.gateway.read(&self.principal, room, advance)
    }
    fn wait_poll(&self, room: &str) -> Result<Vec<Message>> {
        self.gateway.wait_poll(&self.principal, room)
    }
    fn wait_seen(&self, room: &str, seq: i64) -> Result<()> {
        self.gateway.wait_seen(&self.principal, room, seq)
    }
    fn who(&self, room: &str) -> Result<Vec<MemberInfo>> {
        self.gateway.who(&self.principal, room)
    }
    fn claim(&self, room: &str, spec: &ClaimSpec) -> Result<()> {
        self.gateway.claim(&self.principal, room, spec)
    }

    fn set_scope(&self, scope: &str) -> Result<()> {
        self.gateway.set_scope(&self.principal, scope)
    }
    fn scope(&self) -> Result<Option<String>> {
        self.gateway.scope_of(&self.principal)
    }
    fn release(&self, room: &str, path: &str) -> Result<()> {
        self.gateway.release(&self.principal, room, path)
    }
    fn release_with_note(&self, room: &str, path: &str, note: &str) -> Result<()> {
        self.gateway
            .release_with_note(&self.principal, (room, path), note)
    }
    fn last_release(&self, room: &str, path: &str) -> Result<Option<klatch_core::ReleaseNote>> {
        self.gateway.last_release(&self.principal, room, path)
    }
    fn release_all(&self) -> Result<usize> {
        self.gateway.release_all(&self.principal)
    }
    fn admin_close_room(&self, room: &str, closed: bool) -> Result<()> {
        self.gateway.admin_close_room(&self.principal, room, closed)
    }
    fn admin_pause(&self, paused: bool) -> Result<()> {
        self.gateway.admin_pause(&self.principal, paused)
    }
    fn admin_remove_member(&self, room: &str, member: &str) -> Result<()> {
        self.gateway
            .admin_remove_member(&self.principal, room, member)
    }
    fn admin_allow_member(&self, room: &str, member: &str) -> Result<()> {
        self.gateway
            .admin_allow_member(&self.principal, room, member)
    }
    fn admin_audit(&self, limit: usize) -> Result<Vec<klatch_core::AuditRecord>> {
        self.gateway.admin_audit(&self.principal, limit)
    }
    fn admin_set_room_label(&self, room: &str, label: &str) -> Result<()> {
        self.gateway
            .admin_set_room_label(&self.principal, room, label)
    }
    fn admin_set_clearance(&self, member: &str, label: &str) -> Result<()> {
        self.gateway
            .admin_set_clearance(&self.principal, member, label)
    }
    fn admin_set_cap(&self, label: &str) -> Result<()> {
        self.gateway.admin_set_cap(&self.principal, label)
    }
    fn admin_set_label_chain(&self, names: &[&str]) -> Result<()> {
        self.gateway.admin_set_label_chain(&self.principal, names)
    }
    fn admin_plant_canary(&self, room: &str) -> Result<()> {
        self.gateway.admin_plant_canary(&self.principal, room)
    }
    fn admin_add_scrub_term(&self, name: &str, term: &str) -> Result<()> {
        self.gateway
            .admin_add_scrub_term(&self.principal, name, term)
    }
    fn admin_remove_scrub_term(&self, name: &str) -> Result<()> {
        self.gateway.admin_remove_scrub_term(&self.principal, name)
    }
    fn admin_scrub_terms(&self) -> Result<Vec<(String, String)>> {
        self.gateway.admin_scrub_terms(&self.principal)
    }
    fn admin_refusals(&self, limit: usize) -> Result<Vec<klatch_core::RefusalRecord>> {
        self.gateway.admin_refusals(&self.principal, limit)
    }
    fn claims(&self, room: &str) -> Result<Vec<Claim>> {
        self.gateway.claims(&self.principal, room)
    }
    fn digest(&self) -> Result<Digest> {
        self.gateway.digest(&self.principal)
    }
    fn digest_with(&self, backlog: bool) -> Result<Digest> {
        self.gateway.digest_with(&self.principal, backlog)
    }
    fn record_hook_run(&self, event: &str, bytes: usize) -> Result<()> {
        self.gateway.record_hook_run(&self.principal, event, bytes)
    }
    fn hook_runs(&self, member: &str) -> Result<Vec<klatch_core::HookRun>> {
        self.gateway.hook_runs(&self.principal, member)
    }
    fn note_session(&self, session_id: &str) -> Result<()> {
        self.gateway.note_session(&self.principal, session_id)
    }
    fn prune_members(&self, older_than: i64) -> Result<usize> {
        self.gateway.prune_members(&self.principal, older_than)
    }
    fn set_status(&self, room: &str, status: &str) -> Result<()> {
        self.gateway.set_status(&self.principal, room, status)
    }
    fn rooms(&self) -> Result<Vec<String>> {
        self.gateway.rooms(&self.principal)
    }
    fn set_muted(&self, room: &str, muted: bool) -> Result<()> {
        self.gateway.set_muted(&self.principal, room, muted)
    }
    fn board(&self, room: &str) -> Result<klatch_core::Board> {
        self.gateway.board(&self.principal, room)
    }
    fn open_asks(&self, room: &str) -> Result<Vec<klatch_core::OpenAsk>> {
        self.gateway.open_asks(&self.principal, room)
    }
    fn subscribe(&self, room: &str, from_seq: i64) -> Result<Subscription> {
        Ok(Subscription::new(room, from_seq, DEFAULT_WINDOW))
    }
    fn poll(&self, subscription: &mut Subscription, max: usize) -> Result<Batch> {
        subscription.poll(&self.gateway, &self.principal, max)
    }
    fn ack(&self, room: &str, seq: i64) -> Result<()> {
        self.gateway.ack(&self.principal, room, seq)
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
        alice.claim("r", &ClaimSpec::new("src", 60, "")).unwrap();
        assert!(matches!(
            bob.claim("r", &ClaimSpec::new("src/a.rs", 60, "")),
            Err(klatch_core::Error::Conflict { .. })
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
