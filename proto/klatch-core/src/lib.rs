//! klatch-core: the types and pure logic every other crate shares.
//! No I/O and no dependencies, so it can be tested without a database.

mod error;
mod hook;
mod model;
mod text;

pub use error::{Error, Result};
pub use hook::{render_hook, RoomDigest};
pub use model::{Claim, MemberInfo, Message};
pub use text::{flat, norm, overlaps, valid_name};
