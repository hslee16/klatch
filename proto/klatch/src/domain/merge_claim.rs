//! The `merge:main` claim (#865, epic #863): one integrator merges at a time. The claim is an ordinary claim with
//! three rules of its own, all here so they read in one place:
//!
//! - who may take it: an integrator or the human (the guard, `gateway/gateway/roles.rs`, rule `integrator`);
//! - its lease is short: at most `MERGE_CLAIM_MAX_SECS`, renewed by the holder's own activity for the lease it was
//!   made with, and never stretched by the liveness probe (a holder that waits on a tool is not working, #533);
//! - a park (`parked: <why>, back by <time>`) stretches it only up to `MERGE_PARK_MAX_SECS` from the park, not to the
//!   return time. A holder that parks to wait for an event and then goes silent lapses at that bound.

/// The name of the claim, as stored.
pub const MERGE_CLAIM: &str = "merge:main";

/// The longest lease of the claim: half an hour. A longer request gets this, as an ask does; the holder renews by
/// being active or by claiming again.
pub const MERGE_CLAIM_MAX_SECS: i64 = 1800;

/// The most a park stretches the claim, counted from the park: half an hour. A member parks other claims for as long
/// as it says (up to a week); a merge queue must not wait that long on a holder that may be dead, so the return time
/// does not apply to this claim.
pub const MERGE_PARK_MAX_SECS: i64 = 1800;

/// Whether `path` names the merge claim in any spelling a member may type: trimmed, in any case.
pub fn is_merge_claim(path: &str) -> bool {
    path.trim().eq_ignore_ascii_case(MERGE_CLAIM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_the_name_is_the_claim_and_nothing_else_is() {
        for p in ["merge:main", " Merge:Main ", "MERGE:MAIN"] {
            assert!(is_merge_claim(p), "{p:?}");
        }
        for p in ["merge:main2", "merge:", "src/merge:main", "ticket:1", ""] {
            assert!(!is_merge_claim(p), "{p:?}");
        }
    }
}
