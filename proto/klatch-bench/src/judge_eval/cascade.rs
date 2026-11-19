//! The cascade as a graded judge (#936): the production [`CascadeJudge`] over two graded judges, plus what the report
//! needs that the `Judge` seam does not carry: which stage decided, and the tokens and cost of the stages actually
//! called. Policy, gate and fallbacks are the domain's; nothing here decides anything.

use crate::judge_eval::corpus::Truth;
use crate::judge_eval::judges::{EvalJudge, Judged};
use klatch::domain::{
    CascadeJudge, CascadePolicy, Judge, Lean, Reader, Reading, ReviewItem, Stage, Verdict,
};
use std::cell::RefCell;
use std::rc::Rc;

type Seen = Rc<RefCell<Option<Judged>>>;

/// The first stage: a graded judge read as a [`Reader`]. The call's `Judged` is kept for the costs.
struct FirstStage {
    judge: Box<dyn EvalJudge>,
    seen: Seen,
}

impl Reader for FirstStage {
    fn read(&self, item: &ReviewItem) -> Option<Reading> {
        let judged = self.judge.evaluate(item);
        let reading = judged.raw.map(|(pick, confidence)| Reading {
            lean: match pick {
                Truth::Allow => Lean::Allow,
                Truth::Redact => Lean::Redact,
                Truth::Deny => Lean::Deny,
            },
            // a judge that reports no probability apart from its confidence is read on the confidence
            probability: judged.probability.unwrap_or(confidence),
            confidence,
        });
        *self.seen.borrow_mut() = Some(judged);
        reading
    }
}

/// The second stage: a graded judge as a plain [`Judge`].
struct SecondStage {
    judge: Box<dyn EvalJudge>,
    seen: Seen,
}

impl Judge for SecondStage {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        let judged = self.judge.evaluate(item);
        let verdict = judged.verdict.clone();
        *self.seen.borrow_mut() = Some(judged);
        verdict
    }
}

pub struct CascadeEval {
    name: String,
    judge: CascadeJudge<FirstStage, SecondStage>,
    first: Seen,
    second: Seen,
}

impl CascadeEval {
    pub fn new(
        first: Box<dyn EvalJudge>,
        second: Box<dyn EvalJudge>,
        policy: CascadePolicy,
    ) -> Self {
        let name = format!("cascade:{},{}", first.name(), second.name());
        let (seen_first, seen_second) = (Seen::default(), Seen::default());
        CascadeEval {
            name,
            judge: CascadeJudge::new(
                FirstStage {
                    judge: first,
                    seen: seen_first.clone(),
                },
                SecondStage {
                    judge: second,
                    seen: seen_second.clone(),
                },
                policy,
            ),
            first: seen_first,
            second: seen_second,
        }
    }
}

fn add_u64(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

fn add_f64(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    }
}

impl EvalJudge for CascadeEval {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn evaluate(&self, item: &ReviewItem) -> Judged {
        self.first.borrow_mut().take();
        self.second.borrow_mut().take();
        let decided = self.judge.decide(item);
        let first = self.first.borrow_mut().take();
        let second = self.second.borrow_mut().take();
        let mut out = Judged::of(decided.verdict);
        out.stage = Some(decided.stage.word().to_string());
        // what was spent: every stage that was called
        for j in [&first, &second].into_iter().flatten() {
            out.tokens_in = add_u64(out.tokens_in, j.tokens_in);
            out.tokens_out = add_u64(out.tokens_out, j.tokens_out);
            out.cost_usd = add_f64(out.cost_usd, j.cost_usd);
        }
        // a failed call is reported for the stage that decided; a first stage that failed and was escalated is not
        // a failed call of the cascade
        if decided.stage == Stage::Second {
            out.error = second.and_then(|j| j.error);
        }
        out
    }
}

#[cfg(test)]
#[path = "cascade_tests.rs"]
mod tests;
