//! klatch-store: the storage seam. Everything above this crate talks to `Store`, never to a database,
//! so SQLite can later be swapped (DESIGN.md D2, R31). Time is passed in so behaviour is testable.

mod ids;
mod sqlite;

pub use sqlite::SqliteStore;

use klatch_core::{Claim, MemberInfo, Message, Posted, Result};

pub trait Store {
    /// Join a room (idempotent). Unread starts at the beginning of history.
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()>;
    /// Append a message; also marks the sender as present. Returns its unique id and its per-room `seq`
    /// (dense, starting at 1).
    fn post(&self, room: &str, sender: &str, body: &str, now: i64) -> Result<Posted>;
    /// Messages from others after the member's cursor. `advance` moves the cursor to the room's end.
    fn read(&self, room: &str, member: &str, advance: bool, now: i64) -> Result<Vec<Message>>;
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>>;
    /// Take an advisory claim; refuse with `Error::Conflict` if another member holds an overlapping one.
    /// Expired claims are purged first. Re-claiming your own path replaces it.
    fn claim(
        &self,
        room: &str,
        owner: &str,
        path: &str,
        ttl: i64,
        reason: &str,
        now: i64,
    ) -> Result<()>;
    /// Release a claim you hold; `Error::NotFound` if you hold none on that exact path.
    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()>;
    /// Unexpired claims in a room, ordered by path.
    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>>;
    /// Rooms a member has joined, ordered by name.
    fn rooms_of(&self, member: &str) -> Result<Vec<String>>;
    /// Messages with `seq` greater than `after_seq`, oldest first, at most `limit`, from every sender
    /// (including the caller, so `seq` has no gaps). No cursor and no presence side effects.
    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>>;
    /// The highest `seq` in a room, 0 when it is empty.
    fn head(&self, room: &str) -> Result<i64>;
    /// Move a member's read cursor forward to `seq`, never backward and never past the room's head.
    fn advance_cursor(&self, room: &str, member: &str, seq: i64, now: i64) -> Result<()>;
}
