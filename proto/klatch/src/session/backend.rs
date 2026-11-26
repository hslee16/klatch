//! What a session needs from a gateway, as a trait (#102): the clock, the store's change token, the head and the read
//! cursor of a room, one poll of a subscription, and one plain request. A `Gateway` in the calling thread implements it
//! by calling itself; a handle to a gateway that another thread owns (`SharedGateway::session_backend`) implements it by
//! sending each call to the owner. The session loop is the same code over both, so the frames, their order and the
//! sequence numbers are the same.

use super::methods::{Call, handle};
use crate::domain::{Principal, Result};
use crate::gateway::{Batch, Gateway, Subscription};
use crate::protocol::ServerFrame;
use crate::store::Store;

/// The calls a session loop makes on a gateway. Every call names the principal it acts for.
pub trait Backend {
    /// The gateway's clock.
    fn now(&self) -> i64;
    /// The store's change token (see `Transport::change_token`).
    fn change_token(&self) -> Option<u64>;
    /// The newest sequence number of a room the principal may read.
    fn head(&self, who: &Principal, room: &str) -> Result<i64>;
    /// Move the principal's read cursor.
    fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()>;
    /// One poll of a subscription (`Subscription::poll`).
    fn poll(&self, sub: &mut Subscription, who: &Principal, max: usize) -> Result<Batch>;
    /// Answer one plain request: the frames that answer it.
    fn handle(&self, who: &Principal, call: Call) -> Vec<ServerFrame>;
    /// `handle` for a connection whose edge verified a harness (#1096): a `join` is judged by it. `None` is the
    /// generic floor, which is what `handle` is.
    fn handle_as(
        &self,
        who: &Principal,
        call: Call,
        _harness: Option<&dyn crate::harness::Harness>,
    ) -> Vec<ServerFrame> {
        self.handle(who, call)
    }
    /// Prove a member from the text of its credential token: `Verified` when it is live, else the refusal. Nothing
    /// weaker is ever returned for a bad token.
    fn verify_credential(&self, member: &str, token: &str) -> Result<Principal>;
    /// Keep a `member-fixed` refusal the session made for the principal (#1084): a refusal row and a look at the breaker.
    fn note_member_fixed(&self, who: &Principal);
    /// Keep the server's counts-only note of forwarded identity headers it ignored (#1490): one audit row, text of
    /// header names and counts only (anything else is refused). Called by a timer, never per request. A backend that
    /// keeps no audit (a test double) keeps nothing.
    fn note_forwarded_ignored(&self, _counts: &str) -> Result<()> {
        Ok(())
    }
}

impl<S: Store> Backend for Gateway<S> {
    fn now(&self) -> i64 {
        Gateway::now(self)
    }
    fn change_token(&self) -> Option<u64> {
        Gateway::change_token(self)
    }
    fn head(&self, who: &Principal, room: &str) -> Result<i64> {
        Gateway::head(self, who, room)
    }
    fn ack(&self, who: &Principal, room: &str, seq: i64) -> Result<()> {
        Gateway::ack(self, who, room, seq)
    }
    fn poll(&self, sub: &mut Subscription, who: &Principal, max: usize) -> Result<Batch> {
        sub.poll(self, who, max)
    }
    fn handle(&self, who: &Principal, call: Call) -> Vec<ServerFrame> {
        handle(self, who, call, None)
    }
    fn handle_as(
        &self,
        who: &Principal,
        call: Call,
        harness: Option<&dyn crate::harness::Harness>,
    ) -> Vec<ServerFrame> {
        handle(self, who, call, harness)
    }
    fn verify_credential(&self, member: &str, token: &str) -> Result<Principal> {
        let token =
            crate::keys::Token::parse(token).ok_or_else(|| crate::domain::Error::Refused {
                rule: "credential".into(),
                reason: "the credential is not valid".into(),
            })?;
        Gateway::verify_credential(self, member, &token)
    }
    fn note_member_fixed(&self, who: &Principal) {
        Gateway::note_member_fixed(self, who);
    }
    fn note_forwarded_ignored(&self, counts: &str) -> Result<()> {
        Gateway::note_forwarded_ignored(self, counts)
    }
}
