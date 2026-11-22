//! The gateway module (the `klatch-gateway` crate before the crates became modules): the single write path (DESIGN section 19 D6, section 20 rules 10 to 14). Every
//! operation takes a `Principal`, validates its inputs, passes the `Guard` exactly once, and only then
//! touches the store. The label check will live in the guard and nowhere else.

mod claude_judge;
mod clock;
// The Gateway itself keeps its own file; it was the crate's namesake before the crates became modules.
#[allow(clippy::module_inception)]
mod gateway;
pub(crate) use gateway::CANARY_KIND;
mod guard;
mod identity;
mod judge;
mod listener;
#[cfg(test)]
mod listener_tests;
mod notifier;
mod shared;
#[cfg(test)]
mod shared_fairness_tests;
#[cfg(test)]
mod shared_session_tests;
#[cfg(test)]
mod shared_tests;
mod ssh_key;
mod subscription;
mod sys_lease;
mod transport;
mod wake;

pub use claude_judge::{
    BelowCeiling, ClaudeConfig, ClaudeJudge, DEFAULT_CONFIDENCE_BAND, Invocation, Parsed, Pick,
    ProcessOutput, ProcessRunner, RealRunner, Session, fence_for, parse_reply, run_command,
    user_prompt,
};
pub use clock::{Clock, FixedClock, SystemClock};
pub use gateway::{
    AnchorAdapter, Archive, ClaimSpec, CredentialInfo, CredentialSpec, DEFAULT_CAP_SECS,
    DEFAULT_CREDENTIAL_MAX_LIFE_SECS, DEFAULT_EXTEND_SECS, DEFAULT_GRACE_SECS, DeliveryView,
    Digest, Gateway, Handoff, History, HistoryRange, LeaseAction, LeasePolicy, LeaseSource,
    MAX_CREDENTIAL_TTL_SECS, Observed, RateLimitEntry, Refused, RunInfo, check_credential,
    invite_text, lease_action, run_prefix, valid_run_id,
};
pub use guard::{AllowAll, ForbidFloors, Guard, Operation, RequireHuman};
pub use identity::{DeclaredName, FixedIdentity, Identity, LocalIdentity, PeerEvidence, Secret};
pub use judge::{DRAIN_LIMIT, Drained, drain, rules_judge};
pub use listener::{Connection, Connector, EvidenceTable, Listener};
pub use notifier::{NoNotifier, Notifier};
pub use shared::{SharedBackend, SharedGateway, SharedTransport};
pub use ssh_key::{SshKeyIdentity, ssh_key_member, valid_label as valid_ssh_key_label};
pub use subscription::{Batch, DEFAULT_WINDOW, Subscription};
pub use sys_lease::SysLeaseSource;
pub use transport::{Direct, Transport, areas};
pub use wake::{Pause, WAKE_FALLBACK, WAKE_TICK, pause_for_change};

#[cfg(test)]
mod test_support;
