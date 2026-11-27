//! The pieces behind the resident judge loop (#517, D4). Verdicts are data and the floors decide first: the gateway
//! applies a recorded verdict only after checking labels again, and a judge here can only narrow what the floors
//! allow, never widen it.
//!
//! - `RulesJudge`: the local first gate. The credential scanner and the scrub list deny on a match (final). The screen of
//!   #1413 (personal data, injection phrasings, internal hosts, confidentiality phrases) defers with a note that
//!   names the rule, so the item stays for the reviewer or the model judge; it otherwise defers; it never vouches for text, so clean text waits for a reviewer or a configured judge.
//! - `Narrowed`: the rules run first, and a deny from them stands whatever the inner judge would say. The inner
//!   judge is not shown text the rules denied.
//! - `Breaker`: a judge that keeps failing (it defers or panics) cannot allow by default. After `limit` failures in
//!   a row the breaker opens and answers `Defer` without calling the judge again, so items stay queued.
//!
//! Nothing here touches the network. A hosted judge is off unless an operator builds one (see `HostedJudge`).

use super::{Judge, LabelChain, ReviewItem, Screened, Verdict, credential_kind, screen};
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

impl<J: Judge + ?Sized> Judge for &J {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        (**self).judge(item)
    }
    fn judge_noted(&self, item: &ReviewItem) -> (Verdict, Option<String>) {
        (**self).judge_noted(item)
    }
}

/// The credential scanner and the scrub list as a judge. A match is a deny that names the rule, never the text.
pub struct RulesJudge {
    terms: Vec<(String, String)>,
    chain: LabelChain,
}

impl RulesJudge {
    /// `terms` are (rule name, term) pairs, matched case-insensitively as the gateway matches them, and only when
    /// the destination is below the agent cap.
    pub fn new(terms: Vec<(String, String)>, chain: LabelChain) -> Self {
        RulesJudge { terms, chain }
    }
}

impl Judge for RulesJudge {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        self.judge_noted(item).0
    }

    /// The floors (credential scanner, scrub list) deny. A hit of the screen (#1413) is a heuristic, so it is a
    /// deferral with a note naming the rule (`screen:injection`, never the text): the item stays queued for the
    /// reviewer, or reaches the model judge when one is configured, and the audit records the rule.
    fn judge_noted(&self, item: &ReviewItem) -> (Verdict, Option<String>) {
        if let Some(kind) = credential_kind(&item.text) {
            return (
                Verdict::Deny(format!(
                    "credentials: the text looks like it contains {kind}"
                )),
                None,
            );
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
                return (Verdict::Deny(format!("scrub:{name}")), None);
            }
        }
        match screen(
            &item.text,
            &item.from_label,
            &item.to_label,
            &self.chain,
            &self.terms,
        ) {
            Some(Screened::Deny(reason)) => (Verdict::Deny(reason), None),
            Some(Screened::Hold(reason)) => (Verdict::Defer, Some(reason)),
            None => (Verdict::Defer, None),
        }
    }
}

/// Rules first, then `inner`. A rules deny stands and the inner judge never sees that text.
pub struct Narrowed<R: Judge, J: Judge> {
    rules: R,
    inner: J,
}

impl<R: Judge, J: Judge> Narrowed<R, J> {
    pub fn new(rules: R, inner: J) -> Self {
        Narrowed { rules, inner }
    }
}

impl<R: Judge, J: Judge> Judge for Narrowed<R, J> {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        match self.rules.judge(item) {
            denied @ Verdict::Deny(_) => denied,
            _ => self.inner.judge(item),
        }
    }
    fn judge_noted(&self, item: &ReviewItem) -> (Verdict, Option<String>) {
        let (verdict, flagged) = self.rules.judge_noted(item);
        match verdict {
            denied @ Verdict::Deny(_) => (denied, None),
            _ => match self.inner.judge_noted(item) {
                // the inner judge had no view: keep the rules' flag so the audit still names the rule
                (Verdict::Defer, None) => (Verdict::Defer, flagged),
                other => other,
            },
        }
    }
}

/// Stops calling a failing judge. A `Defer` or a panic counts as a failure, any decision resets the count, and
/// once `limit` failures happen in a row it answers `Defer` without calling the judge. It is shared across passes
/// of one resident loop, so a down service is not hammered and a failure never turns into an allow.
pub struct Breaker<J: Judge> {
    inner: J,
    limit: usize,
    failures: Cell<usize>,
}

impl<J: Judge> Breaker<J> {
    pub fn new(inner: J, limit: usize) -> Self {
        Breaker {
            inner,
            limit: limit.max(1),
            failures: Cell::new(0),
        }
    }

    pub fn is_open(&self) -> bool {
        self.failures.get() >= self.limit
    }
}

impl<J: Judge> Judge for Breaker<J> {
    fn judge(&self, item: &ReviewItem) -> Verdict {
        self.judge_noted(item).0
    }

    /// A deferral that carries a note was a deliberate answer (the judge was reached and unsure), so it resets the
    /// count like a decision does.
    fn judge_noted(&self, item: &ReviewItem) -> (Verdict, Option<String>) {
        if self.is_open() {
            return (Verdict::Defer, None);
        }
        let (verdict, note) = catch_unwind(AssertUnwindSafe(|| self.inner.judge_noted(item)))
            .unwrap_or((Verdict::Defer, None));
        if verdict == Verdict::Defer && note.is_none() {
            self.failures.set(self.failures.get() + 1);
        } else {
            self.failures.set(0);
        }
        (verdict, note)
    }
}

#[cfg(test)]
#[path = "judge_loop_tests.rs"]
mod tests;
