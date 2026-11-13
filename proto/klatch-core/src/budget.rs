//! The receiver budget (R22, ticket #22): what a room will take in and how much of it reaches a session. A room
//! must not be a weaker path than Claude Code's own, which already caps size, queue length and bursts.

/// The largest message body a member may post, in bytes. Longer material belongs in a file the message points to.
pub const MAX_POST_BYTES: usize = 32 * 1024;
/// Posts one member may make in one room inside `BURST_WINDOW_SECS`.
pub const MAX_POSTS_PER_WINDOW: usize = 100;
pub const BURST_WINDOW_SECS: i64 = 60;
/// An identical body from the same member in the same room inside this window is refused as a repeat.
pub const DUPLICATE_WINDOW_SECS: i64 = 60;
/// How many of a room's latest messages the post checks look at.
pub const RECENT_LOOKBACK: i64 = 500;
/// Messages one hook digest shows per room; the rest stay unread and are counted.
pub const DIGEST_MAX_MESSAGES: usize = 20;
/// Characters of one message a hook digest shows; `klatch read` shows all of it.
pub const DIGEST_MESSAGE_CHARS: usize = 1000;
