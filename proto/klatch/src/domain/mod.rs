//! The domain module (the `klatch-core` crate before the crates became modules): the types and pure logic every other crate shares.
//! No I/O and no dependencies, so it can be tested without a database.

mod asks;
mod board;
mod convention;
mod credentials;
mod dm;
mod error;
mod hook;
mod judge;
mod labels;
mod model;
pub(crate) mod parked;
mod receipt;
mod scope;
mod tasks;
mod text;
mod toolguard;

pub use asks::{
    ASK_LEASE_DEFAULT_SECS, ASK_LEASE_MAX_SECS, ASK_PREFIX, AskRoom, CLAIM_DEFAULT_SECS,
    CLAIM_MAX_SECS, Clarification, NO_CAPABLE_AFTER, OpenAsk, Outcome, OutcomeKind,
    ROOM_ACTIVE_SECS, STALE_AFTER_SECS, answer_target, claim_ttl, closed_target, is_ask, open_asks,
    parse_clarification, parse_outcome,
};
pub use board::{
    BOARD_LIST_LIMIT, Board, BoardMember, MEMBER_ACTIVE_SECS, MEMBER_RETENTION_SECS, MemberStatus,
    OFFER_PREFIX, OFFER_TTL_SECS, Offer, STATUS_PREFIX, WITHDRAW_PREFIX, current_offers, is_offer,
    render_board, render_who, statuses,
};
pub use parked::{PARKED_DEFAULT_SECS, PARKED_MAX_SECS, PARKED_PREFIX, Parked, parse_parked};
pub use tasks::{
    CHECKPOINT_PREFIX, Checkpoint, FINISHED_PREFIX, HANDOFF_PREFIX, TASK_HOLD_SECS, Task,
    WORKING_PREFIX, open_tasks,
};
pub mod budget;
pub use convention::{CONVENTION_MARKER, convention, subagent_brief};
pub use credentials::credential_kind;
pub use dm::{DM_PREFIX, dm_room};
pub use error::{Error, Result};
pub use hook::{RoomDigest, hook_context_json, hook_output_json, render_hook};
pub use judge::{Judge, NullJudge, ReviewItem, Verdict};
pub use labels::{DEFAULT_LABELS, LabelChain};
pub use model::{
    Attribution, AuditRecord, Claim, ClaimChanges, HookRun, MemberInfo, Message, Posted, Principal,
    RefusalRecord, ReleaseNote, ReleasedClaim,
};
pub use receipt::{Receipt, Receipts, receipt_of, receipts_of, render_receipts};
pub use scope::{SHARED_SCOPE, claim_scope, scopes_collide, valid_scope};
pub use text::{
    MAX_PAYLOAD_BYTES, flat, mentions, norm, overlaps, secs_between, valid_kind, valid_name,
};
pub use toolguard::{
    Asker, EDIT_TOOLS, Held, ToolCall, ToolVerdict, clean_path, command_words, judge_tool,
    patch_targets,
};
