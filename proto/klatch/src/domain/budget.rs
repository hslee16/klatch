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
/// The most messages one `klatch read` or MCP `klatch_read` shows; the rest stay unread and are counted (#1390). A
/// session reads these into its context, and 40 whole messages is twice a digest and one fifth of what `history`
/// allows a caller to ask for.
pub const READ_MAX_MESSAGES: usize = 40;
/// The most bytes one `klatch read` or MCP `klatch_read` shows, measured as printed (#1390). The size of an explicit
/// `history` read (`HISTORY_MAX_BYTES`), so the two explicit reads cost a session the same; a hook digest, which is
/// pushed rather than asked for, is smaller (`DIGEST_MAX_BYTES`).
pub const READ_MAX_BYTES: usize = 16_000;
/// The most messages one `klatch wait` or MCP `klatch_wait` shows for a room (#640, #1390): a wake is a digest, so the
/// digest's count.
pub const WAIT_MAX_MESSAGES: usize = DIGEST_MAX_MESSAGES;
/// The most bytes one `klatch wait` or MCP `klatch_wait` shows for a room (#1390).
pub const WAIT_MAX_BYTES: usize = READ_MAX_BYTES;
/// The most bytes `klatch history` prints (whole messages; the first is always shown, even if it alone is larger).
pub const HISTORY_MAX_BYTES: usize = 16_000;
/// The most bytes one hook digest prints, measured as the JSON the hook prints (escapes count). Claude Code keeps
/// only a 2 KB preview of hook output of about 9.8 KB or more (RESEARCH-NOTES 48.2, #549); this leaves room for the
/// convention and notice a session start adds.
pub const DIGEST_MAX_BYTES: usize = 8000;
/// The most bytes of text one hook prints in all, digest, convention and boards together, measured escaped. The
/// 9.7 KB measured safe less the JSON wrapper and the notice around the text (#557).
pub const HOOK_MAX_BYTES: usize = 9400;
/// The most bytes of board text `render_board` returns (the `board` wire method and `klatch board`), cut by entry
/// lines with a count of the entries left out and every section header kept (#1162). An explicit board read is sized
/// like an explicit `history` read (`HISTORY_MAX_BYTES`), not like the hook's digest; only a board of many long
/// lines is cut, the hook cuts its own copy further (`board_within`).
pub const BOARD_MAX_BYTES: usize = 16_000;
/// What a session start keeps for the boards before the digest takes the rest: a board is cut line by line before
/// messages are, but never to nothing.
pub const BOARD_FLOOR_BYTES: usize = 1000;
/// The smallest budget a hook digest prints within (#602): room for the one line that says how many rooms have news.
/// Below it the digest prints nothing rather than overshoot. No caller reaches it with the constants of today; the
/// session start budget is `HOOK_MAX_BYTES` less the convention, the board floor and the refused-room notes (at most `REFUSED_MAX_BYTES`).
/// The most bytes the notes on refused rooms print at a session start (#612); the rest is a count.
pub const REFUSED_MAX_BYTES: usize = 600;
pub const DIGEST_MIN_BYTES: usize = 100;
