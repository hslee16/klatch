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
    /// Prove a member from the text of its credential token: `Verified` when it is live, else the refusal. Nothing
    /// weaker is ever returned for a bad token.
    fn verify_credential(&self, member: &str, token: &str) -> Result<Principal>;
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
        handle(self, who, call)
    }
    fn verify_credential(&self, member: &str, token: &str) -> Result<Principal> {
        let token =
            crate::keys::Token::parse(token).ok_or_else(|| crate::domain::Error::Refused {
                rule: "credential".into(),
                reason: "the credential is not valid".into(),
            })?;
        Gateway::verify_credential(self, member, &token)
    }
}
