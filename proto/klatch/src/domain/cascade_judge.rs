//! A cascade of two judges in the one judge slot (#936, DESIGN D4, built on the hosted judge of #53). The first judge
//! is fast and weak (Jev: about 150 ms, but it lets 21 to 29 percent of what should not cross through); the second is
//! slow and strong (a Claude model: about 3 s, nearly no leaks). The cascade takes the first judge's reading only
//! where the [`CascadePolicy`] says it is safe to, and sends everything else to the second, whose verdict is final.
//!
//! The rules, all in [`CascadePolicy::accepts`] and [`CascadeJudge::decide`]:
//! - A first-stage deny is accepted when the probability of the deny reaches the deny threshold. A deny only holds a
//!   message back, so the bar is the lower one.
//! - A first-stage allow is accepted only when both the probability and the confidence reach the (higher) allow
//!   threshold: an allow is the one outcome that lets text cross, so it needs the first judge to be sure.
//! - A first-stage redact is escalated by default: the first judge has no text to redact with (Jev returns none), so
//!   its redaction would be a placeholder. `redact_threshold` can accept one as a narrowing placeholder.
//! - Anything else, and a first stage that gave no reading (gated, unreachable, a reply that did not parse), goes to
//!   the second judge. Its verdict is final. If it defers or fails, the cascade defers: it never falls back to the
//!   first judge's weaker pick, and it never allows by default.
//!
//! So the cascade can allow only what the second judge allowed, or what the first judge was very sure of; it can
//! narrow from either stage. Each stage that is hosted keeps its own label gate: text at or above the ceiling is never
//! sent to either, and with both gated the cascade defers. The gate is not repeated here; [`GatedReader`] is the first
//! stage's, the second's is its own (`HostedJudge`).

use super::{HostedConfig, Judge, LabelChain, ReviewItem, Verdict, label_below_ceiling};

/// What the first judge leans to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lean {
    Allow,
    Redact,
    Deny,
}

/// The first judge's own reading of one item: its pick, the probability of that pick, and its confidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub lean: Lean,
    /// The probability that the pick is right.
    pub probability: f64,
    /// How concentrated the judge's probabilities are; never above the probability for a choice.
    pub confidence: f64,
}

/// The first stage: reads an item and reports a [`Reading`], or `None` when it could not (it must apply its own label
/// gate and return `None` for text it may not see).
pub trait Reader {
    fn read(&self, item: &ReviewItem) -> Option<Reading>;
}

/// Which knobs the cascade takes a first-stage reading on. `None` for a knob means "never accept that outcome without
/// escalating".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CascadePolicy {
    /// A deny is accepted when its probability reaches this.
    pub deny_threshold: Option<f64>,
    /// An allow is accepted when its probability and its confidence both reach this.
    pub allow_threshold: Option<f64>,
    /// A redact is accepted (as a placeholder) when its probability reaches this. Off by default.
    pub redact_threshold: Option<f64>,
}

impl Default for CascadePolicy {
    /// Conservative starting points, to be tuned on the dev split and scored on test (design item for #936): a deny at
    /// 0.9, an allow at 0.98 (both numbers), a redact always escalated.
    fn default() -> Self {
        CascadePolicy {
            deny_threshold: Some(0.9),
            allow_threshold: Some(0.98),
            redact_threshold: None,
        }
    }
}

/// The placeholder a first-stage redact carries when `redact_threshold` accepts one.
pub const FIRST_STAGE_REDACTION: &str = "[redacted by the first judge]";

impl CascadePolicy {
    /// A policy with each threshold checked to be a number from 0 to 1.
    pub fn new(
        deny_threshold: Option<f64>,
        allow_threshold: Option<f64>,
        redact_threshold: Option<f64>,
    ) -> Result<CascadePolicy, String> {
        for (name, t) in [
            ("deny", deny_threshold),
            ("allow", allow_threshold),
            ("redact", redact_threshold),
        ] {
            if let Some(t) = t
                && !(t.is_finite() && (0.0..=1.0).contains(&t))
            {
                return Err(format!("the {name} threshold must be a number from 0 to 1"));
            }
        }
        Ok(CascadePolicy {
            deny_threshold,
            allow_threshold,
            redact_threshold,
        })
    }

    /// The verdict to take from the first stage without asking the second, or `None` to escalate. A NaN anywhere
    /// escalates.
    pub fn accepts(&self, r: &Reading) -> Option<Verdict> {
        let reaches = |t: Option<f64>, x: f64| t.is_some_and(|t| x >= t);
        match r.lean {
            Lean::Deny if reaches(self.deny_threshold, r.probability) => {
                Some(Verdict::Deny("the first judge denied it".to_string()))
            }
            Lean::Allow
                if reaches(self.allow_threshold, r.probability)
                    && reaches(self.allow_threshold, r.confidence) =>
            {
                Some(Verdict::Allow)
            }
            Lean::Redact if reaches(self.redact_threshold, r.probability) => {
                Some(Verdict::Redact(FIRST_STAGE_REDACTION.to_string()))
            }
            _ => None,
        }
    }
}

/// Which stage decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The first judge's reading was taken.
    First,
    /// The item was escalated and the second judge's verdict stands (a defer included: then neither decided).
    Second,
}

impl Stage {
    pub fn word(self) -> &'static str {
        match self {
            Stage::First => "first",
            Stage::Second => "second",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decided {
    pub verdict: Verdict,
    pub stage: Stage,
}

pub struct CascadeJudge<F: Reader, S: Judge> {
    first: F,
    second: S,
    policy: CascadePolicy,
}

impl<F: Reader, S: Judge> CascadeJudge<F, S> {
    pub fn new(first: F, second: S, policy: CascadePolicy) -> Self {
        Self {
            first,
            second,
            policy,
        }
    }

    /// The verdict and the stage it came from.
    pub fn decide(&self, item: &ReviewItem) -> Decided {
        if let Some(reading) = self.first.read(item)
            && let Some(verdict) = self.policy.accepts(&reading)
        {
            return Decided {
                verdict,
                stage: Stage::First,
            };
        }
        Decided {
            verdict: self.second.judge(item),
            stage: Stage::Second,
        }
    }
}

impl<F: Reader, S: Judge> Judge for CascadeJudge<F, S> {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        self.decide(item).verdict
    }
}

/// How the text reaches a hosted first stage and what comes back.
pub trait ReadingTransport {
    fn read(&self, text: &str) -> Result<Reading, String>;
}

/// A hosted first stage behind the label gate every hosted reader shares (#53): off by default, text only when its
/// label is strictly below the ceiling, a failure is no reading.
pub struct GatedReader<T: ReadingTransport> {
    transport: T,
    chain: LabelChain,
    config: HostedConfig,
}

impl<T: ReadingTransport> GatedReader<T> {
    pub fn new(transport: T, chain: LabelChain, config: HostedConfig) -> Self {
        Self {
            transport,
            chain,
            config,
        }
    }
}

impl<T: ReadingTransport> Reader for GatedReader<T> {
    fn read(&self, item: &ReviewItem) -> Option<Reading> {
        let allowed = self.config.enabled
            && label_below_ceiling(
                &self.chain,
                self.config.max_label.as_deref(),
                &item.from_label,
            );
        if !allowed {
            return None;
        }
        self.transport.read(&item.text).ok()
    }
}

#[cfg(test)]
#[path = "cascade_judge_tests.rs"]
mod tests;
