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

/// One lease setting a human may change: the admin action name, its policy key and the most it may be.
struct Knob {
    action: &'static str,
    key: &'static str,
    max: i64,
}

/// The cap: at most seven days, the most a park may stretch a claim.
const CAP_KNOB: Knob = Knob {
    action: "lease-cap",
    key: "lease:cap",
    max: 7 * 24 * 3600,
};
/// The grace: at most a day.
const GRACE_KNOB: Knob = Knob {
    action: "lease-grace",
    key: "lease:grace",
    max: 24 * 3600,
};

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

    /// Only the human: set the busy-without-progress cap, in seconds, or with `None` put it back to its default
    /// (#537). One second to seven days, the longest a park may stretch a claim.
    pub fn admin_set_lease_cap(&self, who: &Principal, secs: Option<i64>) -> Result<()> {
        self.set_lease_key(who, &CAP_KNOB, secs)
    }

    /// Only the human: set how long a gone holder's claims linger, in seconds, or with `None` put it back to its
    /// default (#537). One second to a day.
    pub fn admin_set_lease_grace(&self, who: &Principal, secs: Option<i64>) -> Result<()> {
        self.set_lease_key(who, &GRACE_KNOB, secs)
    }

    fn set_lease_key(&self, who: &Principal, knob: &Knob, secs: Option<i64>) -> Result<()> {
        let Knob { action, key, max } = *knob;
        if let Some(s) = secs
            && !(1..=max).contains(&s)
        {
            return Err(Error::Invalid(format!(
                "the {action} is 1 to {max} seconds, or off for the default"
            )));
        }
        let target = secs.map_or("off".to_string(), |s| s.to_string());
        self.administer(who, (action, &target), |now| {
            self.store
                .policy_set(key, secs.map(|s| s.to_string()).as_deref(), now)?;
            Ok(String::new())
        })
    }

    /// The cap and grace in force: a human's setting, else the default.
    pub fn lease_policy(&self) -> Result<LeasePolicy> {
        let d = LeasePolicy::default();
        Ok(LeasePolicy {
            extend_secs: d.extend_secs,
            grace_secs: positive_secs(self.store.policy_get("lease:grace")?, d.grace_secs),
            cap_secs: positive_secs(self.store.policy_get("lease:cap")?, d.cap_secs),
        })
    }

    /// Whether the process behind `member`'s session has exited (#790, DESIGN item 96): true only when the
    /// liveness probe says `Gone` for a member of `room` that has a recorded session. Everything else is false,
    /// because unknown counts as alive: no lease source, a member with no session (a bot), a record the probe
    /// cannot read, a table `ps` cannot give, another machine. The caller must be a member of the room (as for
    /// `who`), and only members of that room are answered about. It changes nothing and moves no lease.
    pub fn process_exited(&self, who: &Principal, room: &str, member: &str) -> Result<bool> {
        self.admit(who, room, Operation::Who { room: room.into() })?;
        let Some(source) = &self.leases else {
            return Ok(false);
        };
        let in_room = self.store.members(room)?.iter().any(|m| m.name == member);
        let session = self.store.session_of_member(member)?;
        let (true, Some(session)) = (in_room, session) else {
            return Ok(false);
        };
        let now_ms = u64::try_from(self.now()).unwrap_or(0) * 1000;
        let seen = source.observe(member, &session, now_ms);
        Ok(seen.is_some_and(|o| o.verdict == Verdict::Gone))
    }

    /// The live claims of `room` whose holder has a registry record and a process the probe finds provably gone, as
    /// `dead_holder` stuck events (#1182): by claim path, in order. Flag only: it moves no lease and releases nothing.
    /// Unknown counts as alive, so no lease source, no session record or a record the probe cannot read flags nothing.
    pub(super) fn dead_holder_events(
        &self,
        room: &str,
        claims: &[crate::domain::Claim],
    ) -> Result<Vec<crate::domain::StuckEvent>> {
        let Some(source) = &self.leases else {
            return Ok(Vec::new());
        };
        let now_ms = u64::try_from(self.now()).unwrap_or(0) * 1000;
        let mut gone: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        let mut out = Vec::new();
        for c in claims {
            let is_gone = match gone.get(&c.owner) {
                Some(g) => *g,
                None => {
                    let g = match self.store.session_of_member(&c.owner)? {
                        Some(session) => source
                            .observe(&c.owner, &session, now_ms)
                            .is_some_and(|o| o.verdict == Verdict::Gone),
                        None => false,
                    };
                    gone.insert(c.owner.clone(), g);
                    g
                }
            };
            if is_gone {
                out.push(crate::domain::StuckEvent::DeadHolder {
                    room: room.to_string(),
                    claim: crate::domain::claim_label(&c.path),
                    holder: c.owner.clone(),
                });
            }
        }
        out.sort_by(|a, b| a.key().cmp(&b.key()));
        Ok(out)
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
