//! The gateway module (the `klatch-gateway` crate before the crates became modules): the single write path (DESIGN section 19 D6, section 20 rules 10 to 14). Every
//! operation takes a `Principal`, validates its inputs, passes the `Guard` exactly once, and only then
//! touches the store. The label check will live in the guard and nowhere else.

mod clock;
// The Gateway itself keeps its own file; it was the crate's namesake before the crates became modules.
#[allow(clippy::module_inception)]
mod gateway;
mod guard;
mod identity;
mod subscription;
mod transport;

pub use clock::{Clock, FixedClock, SystemClock};
pub use gateway::{
    AnchorAdapter, Archive, ClaimSpec, CredentialInfo, CredentialSpec, Digest, Gateway, Handoff,
    MAX_CREDENTIAL_TTL_SECS, Refused, RunInfo, check_credential, run_prefix, valid_run_id,
};
pub use guard::{AllowAll, Guard, Operation};
pub use identity::{DeclaredName, FixedIdentity, Identity, LocalIdentity, PeerEvidence};
pub use subscription::{Batch, DEFAULT_WINDOW, Subscription};
pub use transport::{Direct, Transport};

#[cfg(test)]
mod test_support;
