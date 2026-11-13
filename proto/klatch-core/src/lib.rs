//! klatch-core: the types and pure logic every other crate shares.
//! No I/O and no dependencies, so it can be tested without a database.

mod asks;
mod board;
mod convention;
mod error;
mod hook;
mod model;
mod scope;
mod text;

pub use asks::{
    ASK_PREFIX, NO_CAPABLE_AFTER, OpenAsk, Outcome, OutcomeKind, STALE_AFTER_SECS, answer_target,
    closed_target, is_ask, open_asks, parse_outcome,
};
pub use board::{
    BOARD_LIST_LIMIT, Board, BoardMember, MEMBER_ACTIVE_SECS, MEMBER_RETENTION_SECS, MemberStatus,
    OFFER_PREFIX, OFFER_TTL_SECS, Offer, STATUS_PREFIX, WITHDRAW_PREFIX, current_offers, is_offer,
    render_board, render_who, statuses,
};
pub use convention::{CONVENTION_MARKER, convention};
pub use error::{Error, Result};
pub use hook::{RoomDigest, hook_context_json, hook_output_json, render_hook};
pub use model::{
    Attribution, Claim, ClaimChanges, HookRun, MemberInfo, Message, Posted, Principal,
    ReleasedClaim,
};
pub use scope::{SHARED_SCOPE, claim_scope, scopes_collide, valid_scope};
pub use text::{flat, norm, overlaps, valid_name};
