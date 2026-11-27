//! The judges the runner grades, behind one small trait. A production [`Judge`] (the null judge, the rules) goes
//! through [`Seam`] unchanged; a judge that reports more (a confidence, tokens, cost) implements [`EvalJudge`]
//! directly and is also a production `Judge`, so what is graded is what would run.

use crate::judge_eval::corpus::Truth;
use klatch::domain::{Judge, LabelChain, ReviewItem, Verdict, credential_kind, screen, valid_name};

/// What a judge said about one item, with what it reports about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Judged {
    /// The verdict production would act on.
    pub verdict: Verdict,
    /// The judge's own pick and its confidence in it, before any threshold (for calibration).
    pub raw: Option<(Truth, f64)>,
    /// The probability of the judge's own pick, when it reports one apart from the confidence (Jev does; the
    /// confidence of a choice is never above it).
    pub probability: Option<f64>,
    /// For a judge made of stages: which stage decided (`first` or `second`).
    pub stage: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cost_usd: Option<f64>,
    /// Why there is no decision, when the call failed. Never holds the message text.
    pub error: Option<String>,
}

impl Judged {
    pub fn of(verdict: Verdict) -> Judged {
        Judged {
            verdict,
            raw: None,
            probability: None,
            stage: None,
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            error: None,
        }
    }

    pub fn failed(error: String) -> Judged {
        Judged {
            error: Some(error),
            ..Judged::of(Verdict::Defer)
        }
    }
}

/// A judge as the runner sees it.
pub trait EvalJudge {
    fn name(&self) -> String;
    fn evaluate(&self, item: &ReviewItem) -> Judged;
}

/// A production judge, graded through its own `judge` and nothing else.
pub struct Seam<J: Judge> {
    name: String,
    judge: J,
}

impl<J: Judge> Seam<J> {
    pub fn new(name: &str, judge: J) -> Self {
        Seam {
            name: name.to_string(),
            judge,
        }
    }
}

impl<J: Judge> EvalJudge for Seam<J> {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn evaluate(&self, item: &ReviewItem) -> Judged {
        Judged::of(self.judge.judge(item))
    }
}

/// Read a scrub list: `name = term` per line, `#` comments and blank lines skipped. Names follow the engine's rule
/// names.
pub fn parse_scrub(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, term) = line
            .split_once('=')
            .ok_or_else(|| format!("scrub line {}: expected name = term", n + 1))?;
        let (name, term) = (name.trim(), term.trim());
        if !valid_name(name) {
            return Err(format!("scrub line {}: bad rule name", n + 1));
        }
        if term.is_empty() {
            return Err(format!("scrub line {}: empty term", n + 1));
        }
        out.push((name.to_string(), term.to_string()));
    }
    Ok(out)
}

/// What klatch does today with no model: the credential scanner the gateway refuses posts with (R20,
/// `credential_kind`), then the scrub list (R9), matched as the gateway matches it (case-insensitive substring, only
/// when the destination is below the agent cap), then the screen of #1413 (`klatch::domain::screen`). A match is a deny naming the rule, never the text; anything else
/// is allowed. It never redacts.
///
/// The scrub match is a copy of `Gateway::check_scrub`, which needs a store; the credential match is the engine's
/// own function.
pub struct RulesJudge {
    terms: Vec<(String, String)>,
    chain: LabelChain,
}

impl RulesJudge {
    pub fn new(terms: Vec<(String, String)>) -> Self {
        RulesJudge {
            terms,
            chain: LabelChain::default(),
        }
    }
}

impl Judge for RulesJudge {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        if let Some(kind) = credential_kind(&item.text) {
            return Verdict::Deny(format!(
                "credentials: the text looks like it contains {kind}"
            ));
        }
        if !self
            .chain
            .dominates(&item.to_label, self.chain.default_cap())
        {
            let lower = item.text.to_lowercase();
            if let Some((name, _)) = self
                .terms
                .iter()
                .find(|(_, term)| lower.contains(&term.to_lowercase()))
            {
                return Verdict::Deny(format!("scrub:{name}"));
            }
        }
        match screen(
            &item.text,
            &item.from_label,
            &item.to_label,
            &self.chain,
            &self.terms,
        ) {
            Some(reason) => Verdict::Deny(reason),
            None => Verdict::Allow,
        }
    }
}

#[cfg(test)]
#[path = "judges_tests.rs"]
mod tests;
