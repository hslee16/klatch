//! The domain module (the `klatch-core` crate before the crates became modules): the types and pure logic every other crate shares.
//! No I/O and no dependencies, so it can be tested without a database.

mod ask_bounds;
mod ask_cycle;
mod asker_gone;
mod asks;
mod board;
mod card;
mod claim_age;
mod convention;
mod convention_words;
mod credentials;
mod cycle_notice;
mod dm;
mod error;
mod hook;
mod judge;
mod labels;
mod model;
pub(crate) mod parked;
mod pr_lifecycle;
mod receipt;
mod scope;
mod tasks;
mod text;
mod toolguard;
#[cfg(test)]
mod toolguard_tests;
mod wait_filter;

pub use ask_bounds::{ASK_GIVE_UP_SECS, give_up_notes};
pub use ask_cycle::{AskCycle, ask_cycles, cycle_notes};
pub use asker_gone::{Served, gone_asker_notes, served_by};
pub use asks::{
    ASK_LEASE_DEFAULT_SECS, ASK_LEASE_MAX_SECS, ASK_PREFIX, AskRoom, CLAIM_DEFAULT_SECS,
    CLAIM_MAX_SECS, Clarification, NO_CAPABLE_AFTER, OpenAsk, Outcome, OutcomeKind,
    ROOM_ACTIVE_SECS, STALE_AFTER_SECS, answer_target, claim_ttl, closed_target, is_ask, open_asks,
    parse_clarification, parse_outcome,
};
pub use board::{
    BOARD_LIST_LIMIT, Board, BoardMember, MEMBER_ACTIVE_SECS, MEMBER_RETENTION_SECS, MemberStatus,
    OFFER_PREFIX, OFFER_TTL_SECS, Offer, STATUS_PREFIX, WITHDRAW_PREFIX, ask_reach_note,
    current_offers, is_offer, render_board, render_who, statuses,
};
pub use card::{Card, card_of, render_card};
pub use parked::{PARKED_DEFAULT_SECS, PARKED_MAX_SECS, PARKED_PREFIX, Parked, parse_parked};
pub use pr_lifecycle::{MERGED_PREFIX, OpenPr, PR_PREFIX, PR_WORDS, is_merged_note, open_prs};
pub use tasks::{
    CHECKPOINT_PREFIX, Checkpoint, FINISHED_PREFIX, HANDOFF_PREFIX, TASK_HOLD_SECS, TASK_WORDS,
    Task, WORKING_PREFIX, open_tasks,
};
pub mod budget;
pub use claim_age::{AgedClaim, FIRST_MILESTONE_SECS, due_milestone, milestone};
pub use convention::{CONVENTION_MARKER, convention, convention_in, subagent_brief};
pub use convention_words::{CONVENTION_WORDS, ConventionWord};
pub use credentials::credential_kind;
pub use cycle_notice::CycleNotice;
pub use dm::{DM_PREFIX, dm_room};
pub use error::{Error, NEXT_STEP, Result};
pub use hook::{
    RoomDigest, cut_lines_to_fit, hook_context_json, hook_output_json, printed_len, render_hook,
    render_hook_within,
};
pub use judge::{Judge, NullJudge, ReviewItem, Verdict};
#[cfg(kani)]
pub(crate) use labels::dominates_in;
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
#[cfg(kani)]
pub(crate) use toolguard::clean_bytes;
pub use toolguard::{
    Asker, EDIT_TOOLS, Held, ToolCall, ToolVerdict, clean_path, command_words, judge_tool,
    patch_targets,
};
pub use wait_filter::{Filter as WaitFilter, room_filters};
