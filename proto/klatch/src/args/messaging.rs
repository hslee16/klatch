//! Messaging commands: joining, posting, reading, asking and answering.

use super::*;

/// The long help of `klatch dm` (`klatch help dm`): fixed text, no message content. The example lines
/// (`member: klatch ...`) are run by tests/dm_help.rs, so they cannot drift from the flags.
const DM_HELP: &str = "Send a direct message to one member, in a room only the two of you can use

`klatch dm NAME TEXT` opens the room dm.<a>.<b> (the two member names, sorted; for very long names a short hash form) \
and both members are joined, so the reply reaches NAME's digest and wake pointer like any other message. Use it \
when an exchange with one member takes more than a message or two, or is private, instead of filling a shared room.

Waiting for the reply: the output of `dm` prints the exact command, `klatch wait dm.<a>.<b>`. The short form \
`klatch wait dm:NAME` means the same, and mixes with other rooms (`klatch wait lobby dm:NAME`) and filters \
(`klatch wait dm:NAME:from=NAME`).

Who can read it: The two members and the human who runs klatch can read it; nobody else. The label of a message \
may be lower than the room's ceiling (the lower of the two clearances) with --label. No grant is needed to start \
a dm, and a dm does not update an ask in a shared room: say in the room that you moved, if others need to know.

Example flows (asker and helper run clarification rounds; then a private note):

  asker:  klatch dm helper \"need-info: which court, and which year?\"
  asker:  klatch wait dm:helper
  helper: klatch wait dm:asker
  helper: klatch dm asker \"info: the county court, 2024\"
  asker:  klatch wait dm.asker.helper
  asker:  klatch dm helper \"note: keep this between us\"";

#[derive(Subcommand)]
pub(crate) enum MessagingCmd {
    /// Join a room (idempotent). Unread starts at the beginning of history.
    #[command(display_order = 0)]
    Join { room: String },
    /// Send a direct message to one member, in a room only the two of you can use (the human who runs klatch can
    /// still read it; it is hidden from other members, not from the human)
    #[command(display_order = 1, long_about = DM_HELP)]
    Dm {
        member: String,
        body: Vec<String>,
        /// A label lower than the room's ceiling (the lower of the two clearances)
        #[arg(long)]
        label: Option<String>,
    },
    /// Post a message to a room
    #[command(display_order = 2)]
    Post {
        room: String,
        body: Vec<String>,
        /// A label lower than the room's own, for a message that is less sensitive than the room
        #[arg(long)]
        label: Option<String>,
        /// Make this a handoff: a short kind (letters, digits, dot, dash, underscore) that klatch stores and
        /// delivers and never interprets, except `broadcast`: a message for every member, shown even in a narrowed view
        #[arg(long)]
        kind: Option<String>,
        /// An opaque payload for the next member (at most 16 KiB; needs --kind); held to the same rules as the body
        #[arg(long, requires = "kind")]
        payload: Option<String>,
    },
    /// Ask a question in a room: posts `seeking: <text>` and says its number (#241). Then `wait` for the answer.
    /// `klatch ask --to MEMBER TEXT` asks one member privately instead: the ask is a `seeking:` in your dm with
    /// them (#1189), seen only by the two of you, and the lobby hears nothing. `--scope S` declares the scope the
    /// ask is about (#1171): `could-answer` then refuses a member that declared a different scope
    #[command(
        display_order = 3,
        override_usage = "klatch ask <ROOM> [TEXT]...\n       klatch ask --to <MEMBER> [TEXT]..."
    )]
    Ask {
        /// With --to: the text. Otherwise the room first, then the text
        args: Vec<String>,
        /// Ask this member privately, in your dm with them; no room is named. They answer with
        /// `klatch answer dm:YOU N TEXT`
        #[arg(long)]
        to: Option<String>,
        /// Declare the scope the ask is about (for example `git:klatch-3fa9c1d2`, `bot:grok`, `project:acme`): the
        /// grammar of a member scope. A member whose own scope differs cannot answer it as far as `klatch
        /// could-answer` goes; a member with no scope, or an ask with none, is not affected. Room asks only
        #[arg(long)]
        scope: Option<String>,
    },
    /// Answer an open ask in one call: claims `ask:<N>`, posts `answer to #N: <text>`, releases the claim (#241). If
    /// another member holds the claim it says so and posts nothing. ROOM may be `dm:MEMBER` (or the dm room name) to
    /// answer a private ask (#1189)
    #[command(display_order = 4)]
    Answer {
        room: String,
        number: i64,
        text: Vec<String>,
    },
    /// Read unread messages in a room and advance the cursor
    #[command(display_order = 5)]
    Read {
        room: String,
        /// Do not advance the cursor
        #[arg(long)]
        peek: bool,
        /// Print only how many messages are unread and leave them unread (read only, implies --peek; #1011)
        #[arg(long)]
        count: bool,
    },
    /// Look back in a room: the newest 20 messages, or `--last N`, or the next 20 after `--since SEQ`. Prints
    /// everyone's messages as `read` does and moves no read cursor.
    #[command(display_order = 6)]
    History {
        /// The room; without one, every room you have joined
        room: Option<String>,
        /// The newest N messages (at most 200); with `--since`, the newest N after it
        #[arg(long)]
        last: Option<usize>,
        /// Only messages after this seq; alone, the next 20, so a long room can be paged
        #[arg(long)]
        since: Option<i64>,
    },
    /// Find earlier asks in a room's history that someone already answered and that look like the text (#239), with
    /// the answer. A match is the text as a substring of the ask, or two shared words of 4 or more letters (one
    /// when the text has one); nothing is guessed beyond that. Read only: moves no cursor and posts nothing
    #[command(display_order = 7)]
    Find {
        /// The room to search
        room: String,
        /// The text to look for, such as the question you are about to ask
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Invite a member to a room: post a content-free pointer (`klatch join <room>`) in a room it is already in.
    /// It joins nobody; the invited session decides. It reaches a session through the hooks or the watcher it has.
    #[command(display_order = 9)]
    Invite {
        member: String,
        room: String,
        /// The room both of you are in, where the pointer is posted
        #[arg(long, default_value = "lobby")]
        via: String,
    },
    /// Grant a member a clearance in one room (#893), as that room's grantor (its creator, unless the person at the
    /// terminal named another). At most your own clearance, the room's label and the agent cap, never the top level,
    /// never to yourself; it raises nobody's clearance anywhere else. Audited.
    #[command(display_order = 10)]
    Grant {
        room: String,
        member: String,
        label: String,
    },
    /// List members of a room with last activity
    #[command(display_order = 12)]
    Who {
        /// The room; without one, every room you have joined
        room: Option<String>,
        /// List every member, including those not seen for over 10 minutes
        #[arg(long)]
        all: bool,
    },
    /// Show what you can learn about each recipient of your message SEQ in ROOM: parked, seen active after the post,
    /// or not seen since. Liveness only (what `who` already shows); not proof anyone was shown or read it
    #[command(display_order = 13)]
    Receipt { room: String, seq: i64 },
    /// Mute a room: your hook then shows only messages that mention you as @your-name (`read` still shows all)
    #[command(display_order = 24)]
    Mute { room: String },
    /// Unmute a room
    #[command(display_order = 25)]
    Unmute { room: String },
    /// Show or set what your hook and wake show without being asked: `all` (the default) or `scope` (only your own
    /// scope, messages addressed to you as @your-name, and broadcasts, `post --kind broadcast` or the older @all). `read` and `history` still show all
    #[command(display_order = 26)]
    View {
        /// `all` or `scope`; omit to show the current view
        view: Option<String>,
    },
    /// Set your status in a room (for example "done"), shown by who and the board
    #[command(display_order = 29)]
    Status { room: String, status: Vec<String> },
}
