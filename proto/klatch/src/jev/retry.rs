//! The retry policy, a pure component that mirrors the SDK's `RetryPolicy` (sdk__python__api__retries.md) where it
//! applies and is narrower where safety asks: only HTTP 429 and 529 are retried (api__errors.md: back off and retry),
//! never 400, 401, 403, 413, 415 or 422, and not a bare network failure or the other 5xx statuses the SDK retries.
//! Defaults are the SDK's: 2 retries, 0.5 s first delay doubling to 5 s, a quarter of each delay randomly subtracted,
//! `retry-after-ms` then `retry-after` honoured, and a 30 s budget for the call including its delays (a retry whose
//! delay would reach the budget is not made). The clock, the sleeper and the jitter source are injected, so a test runs
//! instantly and deterministically; this file never waits and never reads a clock itself.

use super::error::{JevTransportError, RetryHeaders};
use std::time::{Duration, Instant};

/// Only a rate limit and an overload are retried.
pub fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 529)
}

/// Time as a duration since some fixed start.
pub trait Clock {
    fn now(&self) -> Duration;
}

pub trait Sleeper {
    fn sleep(&self, d: Duration);
}

/// A source of randomness in [0, 1): the fraction of the jitter to subtract.
pub trait Jitter {
    fn unit(&self) -> f64;
}

/// The three injected pieces together.
pub struct Timing<'a> {
    clock: &'a dyn Clock,
    sleeper: &'a dyn Sleeper,
    jitter: &'a dyn Jitter,
}

impl<'a> Timing<'a> {
    pub fn new(clock: &'a dyn Clock, sleeper: &'a dyn Sleeper, jitter: &'a dyn Jitter) -> Self {
        Self {
            clock,
            sleeper,
            jitter,
        }
    }
}

/// The real clock (monotonic) and sleeper, for the real transport. The jitter source needs randomness, which the real
/// transport brings; [`NoJitter`] is the safe stand in.
pub struct SystemClock(Instant);

impl SystemClock {
    pub fn new() -> Self {
        SystemClock(Instant::now())
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

pub struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// No jitter: every delay is the full backoff.
pub struct NoJitter;

impl Jitter for NoJitter {
    fn unit(&self) -> f64 {
        0.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    max_retries: u32,
    backoff_initial: Duration,
    backoff_max: Duration,
    jitter: f64,
    respect_retry_after: bool,
    budget: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_retries: 2,
            backoff_initial: Duration::from_millis(500),
            backoff_max: Duration::from_secs(5),
            jitter: 0.25,
            respect_retry_after: true,
            budget: Duration::from_secs(30),
        }
    }
}

/// The most retries a policy may name.
pub const MAX_RETRIES: u32 = 10;

impl RetryPolicy {
    /// One try, no retries.
    pub fn none() -> RetryPolicy {
        RetryPolicy {
            max_retries: 0,
            ..RetryPolicy::default()
        }
    }

    pub fn new(
        max_retries: u32,
        backoff_initial: Duration,
        backoff_max: Duration,
        jitter: f64,
        respect_retry_after: bool,
        budget: Duration,
    ) -> Result<RetryPolicy, String> {
        if max_retries > MAX_RETRIES {
            return Err(format!("at most {MAX_RETRIES} retries"));
        }
        if backoff_initial > backoff_max {
            return Err("the first delay is above the maximum delay".into());
        }
        if !jitter.is_finite() || !(0.0..=1.0).contains(&jitter) {
            return Err("the jitter must be a fraction from 0 to 1".into());
        }
        if budget.is_zero() {
            return Err("the budget must be above zero (use none() for no retries)".into());
        }
        Ok(RetryPolicy {
            max_retries,
            backoff_initial,
            backoff_max,
            jitter,
            respect_retry_after,
            budget,
        })
    }

    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }
    pub fn backoff_initial(&self) -> Duration {
        self.backoff_initial
    }
    pub fn backoff_max(&self) -> Duration {
        self.backoff_max
    }
    pub fn jitter(&self) -> f64 {
        self.jitter
    }
    pub fn respects_retry_after(&self) -> bool {
        self.respect_retry_after
    }
    pub fn budget(&self) -> Duration {
        self.budget
    }

    /// The exponential backoff before retry number `retry` (0 is the first): the first delay doubled each time up to the
    /// maximum, less a random fraction (`jitter * unit`) of itself. `unit` is clamped to [0, 1]; NaN counts as 0.
    pub fn backoff(&self, retry: u32, unit: f64) -> Duration {
        let unit = if unit.is_nan() {
            0.0
        } else {
            unit.clamp(0.0, 1.0)
        };
        let base = self.backoff_initial.as_secs_f64() * 2f64.powi(retry.min(30) as i32);
        let capped = base.min(self.backoff_max.as_secs_f64());
        Duration::from_secs_f64(capped * (1.0 - self.jitter * unit))
    }

    /// How long to wait before retry number `retry`: the server's `retry-after-ms`, else its `retry-after`, when the
    /// policy honours them and they parse; otherwise the backoff.
    pub fn delay(&self, retry: u32, headers: &RetryHeaders, unit: f64) -> Duration {
        if self.respect_retry_after
            && let Some(d) = parse_retry_after(headers)
        {
            return d;
        }
        self.backoff(retry, unit)
    }
}

/// The wait a server asked for. `retry-after-ms` is milliseconds and `retry-after` is seconds (decimals allowed); a
/// value that is empty, negative, not finite, not a number or absurdly large is no hint. An HTTP date in `retry-after`
/// is not supported (the backoff is used), which is documented in the design item.
pub fn parse_retry_after(h: &RetryHeaders) -> Option<Duration> {
    fn secs(text: &str, scale: f64) -> Option<Duration> {
        let n: f64 = text.trim().parse().ok()?;
        let s = n * scale;
        (s.is_finite() && (0.0..=86_400.0).contains(&s)).then(|| Duration::from_secs_f64(s))
    }
    h.retry_after_ms
        .as_deref()
        .and_then(|t| secs(t, 0.001))
        .or_else(|| h.retry_after.as_deref().and_then(|t| secs(t, 1.0)))
}

/// Run `op`, retrying a 429 or 529 as the policy says. Anything else, and the last error when the retries or the budget
/// run out, is returned as it came.
pub fn run_with_retry<T>(
    policy: &RetryPolicy,
    timing: &Timing<'_>,
    mut op: impl FnMut() -> Result<T, JevTransportError>,
) -> Result<T, JevTransportError> {
    let start = timing.clock.now();
    let mut retry = 0;
    loop {
        match op() {
            Err(JevTransportError::Status {
                status,
                body,
                headers,
            }) if retryable_status(status) && retry < policy.max_retries => {
                let wait = policy.delay(retry, &headers, timing.jitter.unit());
                let spent = timing.clock.now().saturating_sub(start);
                if spent.saturating_add(wait) >= policy.budget {
                    return Err(JevTransportError::Status {
                        status,
                        body,
                        headers,
                    });
                }
                timing.sleeper.sleep(wait);
                retry += 1;
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests;
