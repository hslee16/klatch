//! Waiting commands: `wait` and `wake`.

use super::*;

#[derive(Subcommand)]
pub(crate) enum WaitCmd {
    /// Wait until another member posts in a room (or the timeout), then print and mark read what arrived
    #[command(display_order = 8)]
    Wait {
        /// Rooms to wait on; wakes on the first with a matching message. A room may carry its own filter,
        /// `ROOM:only=PREFIX` or `ROOM:from=MEMBER` (name the room again for another), which replaces --only and
        /// --from for that room, so a room of two can wake on anything while the lobby is filtered
        #[arg(required = true)]
        rooms: Vec<String>,
        /// Seconds to wait (default 60, at most 300); 0 checks once
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// Only wake for messages whose body starts with this text; others stay unread. Repeat it to wake on any
        /// of several prefixes
        #[arg(long)]
        only: Vec<String>,
        /// Only wake for messages from this member; others stay unread
        #[arg(long)]
        from: Option<String>,
        /// Return the messages without marking them read
        #[arg(long)]
        peek: bool,
    },
    /// Watch for unread messages and, when one arrives, exit 2 with a one-line pointer (no content): the command
    /// of an `asyncRewake` hook. Quiet at its timeout. Moves no read cursor; refreshes presence.
    #[command(display_order = 11)]
    Wake {
        /// Seconds to watch (default 3500, at most 3600)
        #[arg(long, default_value_t = 3500)]
        timeout: u64,
    },
}
