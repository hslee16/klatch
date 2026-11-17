//! A rate limit per member (#187, R45): at most N posts by one member in S seconds, counted over every room (the
//! receiver budget, R22, limits one member in one room). Off until the human sets one (`klatch admin rate-limit N S`),
//! so nothing changes by default and a store pays for the count only when a limit exists. The count comes from the
//! store, not from memory, because every command is its own process. It is a refusal with the rule `rate-limit` and a
//! wait time, and not a safety rule, so it never trips the circuit breaker.

use super::*;

const KEY: &str = "rate-limit";
const MAX_POSTS: usize = 10_000;
const MAX_SECS: i64 = 3600;

/// Whether a limit's posts and seconds are both within the allowed range.
fn in_bounds((posts, secs): (usize, i64)) -> bool {
    (1..=MAX_POSTS).contains(&posts) && (1..=MAX_SECS).contains(&secs)
}

impl<S: Store> Gateway<S> {
    /// Only the human: limit every member to `posts` posts in `secs` seconds, or with `None` remove the limit.
    pub fn admin_set_rate_limit(&self, who: &Principal, limit: Option<(usize, i64)>) -> Result<()> {
        if limit.is_some_and(|l| !in_bounds(l)) {
            return Err(Error::Invalid(format!(
                "a rate limit is 1 to {MAX_POSTS} posts in 1 to {MAX_SECS} seconds"
            )));
        }
        let target = limit.map_or("off".to_string(), |(p, s)| format!("{p}/{s}"));
        self.administer(who, ("rate-limit", &target), |now| {
            self.store
                .policy_set(KEY, limit.map(|(p, s)| format!("{p}/{s}")).as_deref(), now)?;
            Ok(String::new())
        })
    }

    /// The limit in force: posts and seconds, if one is set.
    pub fn rate_limit(&self) -> Result<Option<(usize, i64)>> {
        let Some(text) = self.store.policy_get(KEY)? else {
            return Ok(None);
        };
        Ok(text
            .split_once('/')
            .and_then(|(p, s)| Some((p.parse().ok()?, s.parse().ok()?))))
    }

    /// Refuse a post from a member that has used up its limit; called before the post is stored.
    pub(super) fn check_rate(&self, who: &Principal, room: &str) -> Result<()> {
        let Some((limit, secs)) = self.rate_limit()? else {
            return Ok(());
        };
        let now = self.now();
        let recent = self.store.posts_since(&who.member, now - secs + 1)?;
        if recent.len() < limit {
            return Ok(());
        }
        let oldest = recent.iter().copied().min().unwrap_or(now);
        let e = Error::Refused {
            rule: "rate-limit".into(),
            reason: format!(
                "{} has posted {limit} posts in {secs} seconds, the most allowed in all rooms together; wait {}s",
                who.member,
                (oldest + secs - now).max(1)
            ),
        };
        self.note_refusal(who, (room, "post"), &e);
        Err(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_is_in_bounds_only_when_both_numbers_are() {
        assert!(in_bounds((1, 1)));
        assert!(in_bounds((MAX_POSTS, MAX_SECS)));
        assert!(!in_bounds((0, 60)));
        assert!(!in_bounds((30, 0)));
        assert!(!in_bounds((MAX_POSTS + 1, 60)));
        assert!(!in_bounds((30, MAX_SECS + 1)));
    }
}
