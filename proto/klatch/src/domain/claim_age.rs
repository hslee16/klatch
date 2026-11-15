//! How long a claim has been held, and when its owner is told (#547). A claim that its owner renews by being active
//! never lapses, so nothing else asks whether it is still wanted. The owner gets one line at each age milestone
//! (2 hours, 8 hours, then every day), and a peer who runs into the claim is told its age. Content-free: a path and
//! a number, never message text.

/// The first age at which the owner is reminded.
pub const FIRST_MILESTONE_SECS: i64 = 2 * 3600;
const SECOND_MILESTONE_SECS: i64 = 8 * 3600;
const DAY_SECS: i64 = 86_400;

/// The highest milestone an age has passed: 2 hours, 8 hours, then whole days; 0 before the first.
pub fn milestone(age_secs: i64) -> i64 {
    if age_secs >= DAY_SECS {
        age_secs / DAY_SECS * DAY_SECS
    } else if age_secs >= SECOND_MILESTONE_SECS {
        SECOND_MILESTONE_SECS
    } else if age_secs >= FIRST_MILESTONE_SECS {
        FIRST_MILESTONE_SECS
    } else {
        0
    }
}

/// The milestone to announce for a claim held since `since` whose owner was last told about `noted` (0 for never),
/// if a new one has passed. A `since` of 0 means unknown (a claim from before the column existed): never announced.
pub fn due_milestone(since: i64, noted: i64, now: i64) -> Option<i64> {
    if since <= 0 {
        return None;
    }
    Some(milestone(now - since)).filter(|m| *m > noted)
}

/// A claim of the caller that passed a milestone: shown once, with how long it has been held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgedClaim {
    pub path: String,
    pub held_secs: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones_are_two_hours_eight_hours_then_each_day() {
        assert_eq!(milestone(7_199), 0);
        assert_eq!(milestone(7_200), 7_200);
        assert_eq!(milestone(28_799), 7_200);
        assert_eq!(milestone(28_800), 28_800);
        assert_eq!(milestone(86_399), 28_800);
        assert_eq!(milestone(86_400), 86_400);
        assert_eq!(milestone(86_400 * 3 + 5), 86_400 * 3);
    }

    #[test]
    fn a_milestone_is_due_once_and_only_when_a_new_one_has_passed() {
        assert_eq!(due_milestone(1_000, 0, 1_000 + 7_199), None);
        assert_eq!(due_milestone(1_000, 0, 1_000 + 7_200), Some(7_200));
        assert_eq!(due_milestone(1_000, 7_200, 1_000 + 20_000), None);
        assert_eq!(due_milestone(1_000, 7_200, 1_000 + 28_800), Some(28_800));
    }

    #[test]
    fn a_missed_milestone_is_announced_as_the_latest_one_not_each() {
        assert_eq!(due_milestone(1, 7_200, 1 + 86_400 * 2), Some(86_400 * 2));
    }

    #[test]
    fn an_unknown_start_is_never_announced() {
        assert_eq!(due_milestone(0, 0, 10_000_000), None);
    }
}
