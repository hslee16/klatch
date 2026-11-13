//! klatch-core: the types and pure logic every other crate shares.
//! No I/O and no dependencies, so it can be tested without a database.

mod asks;
mod error;
mod hook;
mod model;
mod text;

pub use asks::{
    ASK_PREFIX, OpenAsk, STALE_AFTER_SECS, answer_target, closed_target, is_ask, open_asks,
};
pub use error::{Error, Result};
pub use hook::{RoomDigest, hook_context_json, render_hook};
pub use model::{
    Attribution, Claim, ClaimChanges, MemberInfo, Message, Posted, Principal, ReleasedClaim,
};
pub use text::{flat, norm, overlaps, valid_name};
