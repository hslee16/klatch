//! Read-only views over a room: the board, member cards, open asks, stuck cases, who could answer, the digest.

use super::Direct;
use crate::domain::Result;
use crate::gateway::Digest;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;

/// Read-only views over a room: the board, member cards, open asks, stuck cases, who could answer, the digest.
pub trait Board {
    fn digest(&self) -> Result<Digest>;
    /// `digest`, with the backlog of open unclaimed asks from others when `backlog` is set.
    fn digest_with(&self, backlog: bool) -> Result<Digest>;
    /// The room's board (open asks, current offers, members with status).
    fn board(&self, room: &str) -> Result<crate::domain::Board>;
    /// One member's card from the board and the live claims, or `None` when it is not listed (#251).
    fn member_card(&self, room: &str, member: &str) -> Result<Option<crate::domain::Card>>;
    /// Asks in a room nobody has answered or closed.
    fn open_asks(&self, room: &str) -> Result<Vec<crate::domain::OpenAsk>>;
    /// The stuck cases of a room as structured events, ids and numbers only (#271).
    fn stuck(&self, room: &str) -> Result<Vec<crate::domain::StuckEvent>>;
    /// Whether `member` could answer open ask `ask` in `room` now, and every reason it could not (#671). A read only.
    fn could_answer(
        &self,
        room: &str,
        member: &str,
        ask: i64,
    ) -> Result<crate::domain::CouldAnswer>;
}

impl<S: Store> Board for Direct<S> {
    fn digest(&self) -> Result<Digest> {
        self.gateway.digest(&self.principal)
    }
    fn digest_with(&self, backlog: bool) -> Result<Digest> {
        self.gateway.digest_with(&self.principal, backlog)
    }
    fn board(&self, room: &str) -> Result<crate::domain::Board> {
        self.gateway.board(&self.principal, room)
    }
    fn member_card(&self, room: &str, member: &str) -> Result<Option<crate::domain::Card>> {
        self.gateway.member_card(&self.principal, room, member)
    }
    fn open_asks(&self, room: &str) -> Result<Vec<crate::domain::OpenAsk>> {
        self.gateway.open_asks(&self.principal, room)
    }
    fn stuck(&self, room: &str) -> Result<Vec<crate::domain::StuckEvent>> {
        self.gateway.stuck(&self.principal, room)
    }
    fn could_answer(
        &self,
        room: &str,
        member: &str,
        ask: i64,
    ) -> Result<crate::domain::CouldAnswer> {
        self.gateway
            .could_answer(&self.principal, room, member, ask)
    }
}

impl Board for SharedTransport {
    forward! {
        digest() -> Result<crate::gateway::Digest>;
        digest_with(backlog: bool) -> Result<crate::gateway::Digest>;
        board(room: &str) -> Result<crate::domain::Board>;
        member_card(room: &str, member: &str) -> Result<Option<crate::domain::Card>>;
        open_asks(room: &str) -> Result<Vec<crate::domain::OpenAsk>>;
        stuck(room: &str) -> Result<Vec<crate::domain::StuckEvent>>;
        could_answer(room: &str, member: &str, ask: i64) -> Result<crate::domain::CouldAnswer>;
    }
}
