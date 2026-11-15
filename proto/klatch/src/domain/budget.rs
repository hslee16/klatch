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
/// Messages `history` returns when the caller names no count (#518).
pub const HISTORY_DEFAULT_MESSAGES: usize = 20;
/// The most messages one `history` call returns, however many the caller asks for.
pub const HISTORY_MAX_MESSAGES: usize = 200;
/// The most bytes `klatch history` prints (whole messages; the first is always shown, even if it alone is larger).
pub const HISTORY_MAX_BYTES: usize = 16_000;
/// The most bytes one hook digest prints, measured as the JSON the hook prints (escapes count). Claude Code keeps
/// only a 2 KB preview of hook output of about 9.8 KB or more (RESEARCH-NOTES 48.2, #549); this leaves room for the
/// convention and notice a session start adds.
pub const DIGEST_MAX_BYTES: usize = 8000;
/// The most bytes of text one hook prints in all, digest, convention and boards together, measured escaped. The
/// 9.7 KB measured safe less the JSON wrapper and the notice around the text (#557).
pub const HOOK_MAX_BYTES: usize = 9400;
/// What a session start keeps for the boards before the digest takes the rest: a board is cut line by line before
/// messages are, but never to nothing.
pub const BOARD_FLOOR_BYTES: usize = 1000;
/// The smallest budget a hook digest prints within (#602): room for the one line that says how many rooms have news.
/// Below it the digest prints nothing rather than overshoot. No caller reaches it with the constants of today; the
/// session start budget is `HOOK_MAX_BYTES` less the convention, the board floor and one line per refused room.
pub const DIGEST_MIN_BYTES: usize = 100;
