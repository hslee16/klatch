//! Lease rules driven by the liveness probe (#304, DESIGN section 29 item 4). A claim's time to live was built for
//! minutes; a tool call can run for hours, during which no hook fires. So when someone asks (a claim attempt, a
//! claims listing, `who`), the gateway asks the probe about every other member that holds a claim and moves the
//! expiry of that member's claims by the verdict:
//!
//! - working, waiting on a tool, waiting on a human: the claims hold (pushed out to a horizon, and an expired row
//!   nobody has taken yet is put back);
//! - gone: the claims lapse a grace period later;
//! - stuck: the claims lapse now;
//! - busy with no progress for the cap: the claims lapse now (a sleeping tool looks like a hung one);
//! - unknown: nothing changes, so a member the probe cannot read keeps only its explicit time to live.
//!
//! A member with no recorded session (a bot) is never probed and keeps its explicit TTL (#302).

use super::*;
use crate::harness::Verdict;

/// How far ahead the claims of a live holder are kept.
pub const DEFAULT_EXTEND_SECS: i64 = 600;
/// How long after a holder is found gone its claims linger.
pub const DEFAULT_GRACE_SECS: i64 = 300;
/// Busy without progress for this long ends the lease whatever the probe says otherwise. Raisable by a human
/// through the policy key `lease:cap` (seconds).
pub const DEFAULT_CAP_SECS: i64 = 4 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeasePolicy {
    pub extend_secs: i64,
    pub grace_secs: i64,
    pub cap_secs: i64,
}

impl Default for LeasePolicy {
    fn default() -> Self {
        LeasePolicy {
            extend_secs: DEFAULT_EXTEND_SECS,
            grace_secs: DEFAULT_GRACE_SECS,
            cap_secs: DEFAULT_CAP_SECS,
        }
    }
}

/// What to do to a holder's claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseAction {
    /// Make the claims last at least until this time.
    Extend(i64),
    /// Make the claims end no later than this time.
    Shorten(i64),
    /// End the claims now.
    Release,
    Leave,
}

/// What the probe saw of one holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    pub verdict: Verdict,
    /// How long since the last sign of activity, while busy.
    pub quiet_for_ms: Option<u64>,
}

/// Where the gateway learns whether a holder is alive. The real one probes the harness; tests make one up.
pub trait LeaseSource {
    /// `None` when the member cannot be probed (its session is not in the registry, another machine).
    fn observe(&self, member: &str, session_id: &str, now_ms: u64) -> Option<Observed>;
}

pub fn lease_action(seen: Observed, now: i64, p: &LeasePolicy) -> LeaseAction {
    let capped = seen
        .quiet_for_ms
        .is_some_and(|q| q / 1000 >= p.cap_secs.max(0) as u64);
    match seen.verdict {
        Verdict::Stuck => LeaseAction::Release,
        Verdict::Gone => LeaseAction::Shorten(now + p.grace_secs),
        Verdict::Working | Verdict::WaitingOnTool if capped => LeaseAction::Release,
        Verdict::Working | Verdict::WaitingOnTool | Verdict::WaitingOnHuman => {
            LeaseAction::Extend(now + p.extend_secs)
        }
        Verdict::Unknown => LeaseAction::Leave,
    }
}

fn positive_secs(value: Option<String>, default: i64) -> i64 {
    value
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

impl<S: Store> Gateway<S> {
    /// Probe liveness for the lease rules through `source` whenever claims are asked about.
    pub fn with_leases(mut self, source: impl LeaseSource + 'static) -> Self {
        self.leases = Some(Box::new(source));
        self
    }

    fn lease_policy(&self) -> Result<LeasePolicy> {
        let d = LeasePolicy::default();
        Ok(LeasePolicy {
            extend_secs: d.extend_secs,
            grace_secs: positive_secs(self.store.policy_get("lease:grace")?, d.grace_secs),
            cap_secs: positive_secs(self.store.policy_get("lease:cap")?, d.cap_secs),
        })
    }

    /// Apply the lease rules to every member holding a claim except `asker` (alive, since it is asking). Does
    /// nothing without a lease source.
    pub(super) fn settle_leases(&self, asker: &str) -> Result<()> {
        let Some(source) = &self.leases else {
            return Ok(());
        };
        let policy = self.lease_policy()?;
        let now = self.now();
        let now_ms = u64::try_from(now).unwrap_or(0) * 1000;
        for owner in self.store.claim_owners()? {
            if owner == asker {
                continue;
            }
            let Some(session) = self.store.session_of_member(&owner)? else {
                continue;
            };
            let Some(seen) = source.observe(&owner, &session, now_ms) else {
                continue;
            };
            match lease_action(seen, now, &policy) {
                LeaseAction::Extend(until) => {
                    self.store.retime_member_claims(&owner, until, true)?;
                }
                LeaseAction::Shorten(until) => {
                    self.store.retime_member_claims(&owner, until, false)?;
                }
                LeaseAction::Release => {
                    self.store.release_member_claims(&owner, now)?;
                }
                LeaseAction::Leave => {}
            }
        }
        Ok(())
    }
}
