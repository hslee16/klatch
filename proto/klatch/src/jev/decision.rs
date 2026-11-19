//! Turning a Jev answer into a decision, the documented way (confidence.md, patterns__confidence-routing.md,
//! patterns__noul-screen.md). Jev is advice to a deterministic guard, never authority: nothing here can widen what the
//! rules and the guard already allow. It can only defer, hold, narrow, or route among candidates the guard allowed.
//!
//! Confidence is how concentrated the probabilities are; the probability of the option your code acts on is the chance
//! that option is right, so that is the number a threshold belongs on (confidence.md). What changed from the first
//! adapter of #912, which required both numbers to reach the threshold for every outcome: an outcome that only narrows
//! (a deny, a redact, a route to `none`) is now taken on the probability alone, at the threshold the operator chose,
//! while an outcome that lets something through (an allow, a route to a candidate) still needs both. For a fixed
//! number of options confidence is a function of the top probability and is never above it, so requiring it as well
//! only ever makes the widening outcomes stricter.

use super::answer::{ChoiceAnswer, ScoreAnswer};
use crate::domain::Verdict;
use std::ops::RangeInclusive;

/// The extra choice every routing question carries: send nothing anywhere.
pub const NONE_CHOICE: &str = "none";

/// The text a redaction carries: Jev returns no text, so it cannot write a replacement, and the original is never used.
pub const REDACTED_PLACEHOLDER: &str = "[redacted by the judge]";

/// The documented choice confidence from the probabilities: `(max - 1/n) / (1 - 1/n)`. A lone option is certain.
pub fn choice_confidence(probabilities: &[f64]) -> f64 {
    let n = probabilities.len();
    if n < 2 {
        return 1.0;
    }
    let max = probabilities.iter().copied().fold(0.0, f64::max);
    (max - 1.0 / n as f64) / (1.0 - 1.0 / n as f64)
}

/// A noul answer carries no confidence; the documented stand in is the distance from one half, `|2p - 1|`.
pub fn noul_confidence(p: f64) -> f64 {
    (2.0 * p - 1.0).abs()
}

/// The documented score confidence from the level probabilities (lowest level first): one minus the probability
/// weighted distance from the most likely level over the same distance for an even spread, floored at zero.
pub fn score_confidence(probabilities: &[f64]) -> f64 {
    let n = probabilities.len();
    if n < 2 {
        return 1.0;
    }
    let mut m = 0;
    for (i, p) in probabilities.iter().enumerate() {
        if *p > probabilities[m] {
            m = i;
        }
    }
    let spread: f64 = probabilities
        .iter()
        .enumerate()
        .map(|(i, p)| p * (i as f64 - m as f64).abs())
        .sum();
    let mid = (n as f64 - 1.0) / 2.0;
    let even: f64 = (0..n).map(|i| (i as f64 - mid).abs()).sum::<f64>() / n as f64;
    (1.0 - spread / even).max(0.0)
}

/// How clearly the chosen option beats the runner-up (`p_max / p_second`); infinite when the runner-up has none and
/// `None` for a lone option.
pub fn top_to_second_ratio(a: &ChoiceAnswer) -> Option<f64> {
    let mut p: Vec<f64> = a.probabilities.values().copied().collect();
    p.sort_by(|x, y| y.total_cmp(x));
    let (top, second) = (*p.first()?, *p.get(1)?);
    Some(if second == 0.0 {
        f64::INFINITY
    } else {
        top / second
    })
}

impl ScoreAnswer {
    /// True when the answer is settled at all: its confidence reaches `floor` (the documented pattern checks confidence
    /// first, with 0.5 as the example floor, then routes on the score).
    pub fn is_settled(&self, floor: f64) -> bool {
        self.confidence >= floor
    }

    /// The probability that the true level is within `levels` (levels past the scale add nothing). Use it, not the
    /// score, when a decision depends on one side of a boundary (two split levels can look settled by confidence).
    pub fn level_mass(&self, levels: RangeInclusive<usize>) -> f64 {
        self.probabilities
            .iter()
            .enumerate()
            .filter(|(i, _)| levels.contains(i))
            .map(|(_, p)| p)
            .sum()
    }
}

/// The two thresholds of the noul screen: above `yes_above` is a clear yes, below `no_below` a clear no, and between is
/// its own outcome, never rounded to either side. The documented starting points are 0.7 and 0.3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoulThresholds {
    yes_above: f64,
    no_below: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Yes,
    No,
    Unsure,
}

impl Default for NoulThresholds {
    fn default() -> Self {
        NoulThresholds {
            yes_above: 0.7,
            no_below: 0.3,
        }
    }
}

impl NoulThresholds {
    pub fn new(yes_above: f64, no_below: f64) -> Result<NoulThresholds, String> {
        let unit = |x: f64| x.is_finite() && (0.0..=1.0).contains(&x);
        if !unit(yes_above) || !unit(no_below) || no_below >= yes_above {
            return Err(
                "the noul thresholds must be within 0 and 1 with no_below under yes_above".into(),
            );
        }
        Ok(NoulThresholds {
            yes_above,
            no_below,
        })
    }

    pub fn yes_above(&self) -> f64 {
        self.yes_above
    }

    pub fn no_below(&self) -> f64 {
        self.no_below
    }

    pub fn band(&self, p: f64) -> Band {
        if p > self.yes_above {
            Band::Yes
        } else if p < self.no_below {
            Band::No
        } else {
            Band::Unsure
        }
    }
}

/// The hosted judge's verdict for an `allow`, `redact` or `deny` choice. A deny or a redact only holds a message back,
/// so it is taken when the probability of the pick reaches `threshold`; an allow lets a message cross, so it also needs
/// the confidence to reach it. Anything else, an unknown choice included, is `Defer`.
pub fn judge_verdict(a: &ChoiceAnswer, threshold: f64) -> Verdict {
    let p = a.chosen_probability();
    match a.choice.as_str() {
        "allow" if p >= threshold && a.confidence >= threshold => Verdict::Allow,
        "redact" if p >= threshold => Verdict::Redact(REDACTED_PLACEHOLDER.to_string()),
        "deny" if p >= threshold => Verdict::Deny("the Jev judge denied it".to_string()),
        _ => Verdict::Defer,
    }
}

/// The facilitator and router decision. `candidates` are the names (rooms or members) the rules and the guard already
/// allow; `rules` is what the rules router decided (`None` for nothing). A `none` answer narrows to nothing when the
/// probability of it reaches `threshold`; a candidate is taken only when both the probability and the confidence reach
/// it; a name outside the candidates is ignored. In every other case the rules decision stands.
pub fn pick(
    candidates: &[String],
    rules: Option<&str>,
    answer: &ChoiceAnswer,
    threshold: f64,
) -> Option<String> {
    let rules = rules.map(String::from);
    let p = answer.chosen_probability();
    if answer.choice == NONE_CHOICE {
        return if p >= threshold { None } else { rules };
    }
    if p >= threshold && answer.confidence >= threshold && candidates.contains(&answer.choice) {
        return Some(answer.choice.clone());
    }
    rules
}

/// [`pick`] behind a noul screen, asked in the same request (patterns__noul-screen.md and fan-out): a clear no
/// ("does this ask need a routed recipient at all?") narrows to nothing, an unsure answer leaves the rules decision,
/// and only a clear yes lets the pick run. It can only narrow or route among the candidates, never widen.
pub fn pick_screened(
    candidates: &[String],
    rules: Option<&str>,
    screen: Band,
    answer: &ChoiceAnswer,
    threshold: f64,
) -> Option<String> {
    match screen {
        Band::No => None,
        Band::Unsure => rules.map(String::from),
        Band::Yes => pick(candidates, rules, answer, threshold),
    }
}

#[cfg(test)]
mod tests;
