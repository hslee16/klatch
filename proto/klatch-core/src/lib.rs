//! klatch-core: the types and pure logic every other crate shares.
//! No I/O and no dependencies, so it can be tested without a database.

mod asks;
mod board;
mod convention;
mod credentials;
mod dm;
mod error;
mod hook;
mod labels;
mod model;
mod scope;
mod text;

pub use asks::{
    ASK_LEASE_DEFAULT_SECS, ASK_LEASE_MAX_SECS, ASK_PREFIX, AskRoom, CLAIM_DEFAULT_SECS,
    Clarification, NO_CAPABLE_AFTER, OpenAsk, Outcome, OutcomeKind, ROOM_ACTIVE_SECS,
    STALE_AFTER_SECS, answer_target, claim_ttl, closed_target, is_ask, open_asks,
    parse_clarification, parse_outcome,
};
pub use board::{
    BOARD_LIST_LIMIT, Board, BoardMember, MEMBER_ACTIVE_SECS, MEMBER_RETENTION_SECS, MemberStatus,
    OFFER_PREFIX, OFFER_TTL_SECS, Offer, STATUS_PREFIX, WITHDRAW_PREFIX, current_offers, is_offer,
    render_board, render_who, statuses,
};
pub mod budget;
pub use convention::{CONVENTION_MARKER, convention};
pub use credentials::credential_kind;
pub use dm::{DM_PREFIX, dm_room};
pub use error::{Error, Result};
pub use hook::{RoomDigest, hook_context_json, hook_output_json, render_hook};
pub use labels::{DEFAULT_LABELS, LabelChain};
pub use model::{
    Attribution, AuditRecord, Claim, ClaimChanges, HookRun, MemberInfo, Message, Posted, Principal,
    RefusalRecord, ReleaseNote, ReleasedClaim,
};
pub use scope::{SHARED_SCOPE, claim_scope, scopes_collide, valid_scope};
pub use text::{flat, mentions, norm, overlaps, secs_between, valid_name};
