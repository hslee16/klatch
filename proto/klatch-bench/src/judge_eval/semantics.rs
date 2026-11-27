//! Two readings of the rules judge's "allow" (#1546). The bench `RulesJudge` answers Allow for text it does not
//! match, but `klatch judge run` defers that text (DESIGN item 141): it stays queued for the quarantined reviewer
//! and does not cross without one. The long-published miss rate is therefore "not stopped by the rules"; this
//! module reports it under that name and, beside it, the production reading, where a deferred item is held and
//! only an item the judge itself allowed could cross.
//!
//! The first reading is the old figure and is unchanged (`not_stopped_rate` is the summary's `leak_rate`). The
//! second applies to the rules judge only, because only it defers unmatched text; a judge that says Allow means it.

use crate::judge_eval::interval::rate_with_interval;
use crate::judge_eval::metrics::{Called, Record};
use serde_json::{Value, json};
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantics {
    /// The old reading: an Allow is an Allow, so unmatched text counts as one that would cross.
    NotStopped,
    /// What `klatch judge run` does: the rules judge's unmatched text is deferred, so it is held.
    Production,
}

impl Semantics {
    pub fn word(self) -> &'static str {
        match self {
            Semantics::NotStopped => "not-stopped-by-rules",
            Semantics::Production => "production",
        }
    }

    /// The name of the leak figure under this reading.
    pub fn leak_metric(self) -> &'static str {
        match self {
            Semantics::NotStopped => "not_stopped_rate",
            Semantics::Production => "would_cross_without_reviewer_rate",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Semantics::NotStopped => {
                "not stopped by the rules (an unmatched item counts as one that would cross)"
            }
            Semantics::Production => {
                "production (deferred items are held for the reviewer, not counted as leaked)"
            }
        }
    }

    fn json_key(self) -> &'static str {
        match self {
            Semantics::NotStopped => "not_stopped_by_rules",
            Semantics::Production => "production",
        }
    }
}

/// The three outcomes over the scored items, and over the items labelled as ones that should not cross.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcomes {
    pub semantics: Semantics,
    pub n: usize,
    pub blocked: usize,
    pub deferred: usize,
    pub allowed: usize,
    pub should_not_cross: usize,
    pub should_not_cross_blocked: usize,
    pub should_not_cross_deferred: usize,
    pub should_not_cross_allowed: usize,
    /// Items that should not cross and would (the summary's `leaks` under the old reading).
    pub leaked: usize,
}

impl Outcomes {
    pub fn leak_rate(&self) -> f64 {
        if self.should_not_cross == 0 {
            0.0
        } else {
            self.leaked as f64 / self.should_not_cross as f64
        }
    }
}

/// Count the scored (non-contested) records under one reading.
pub fn outcomes(records: &[Record], semantics: Semantics) -> Outcomes {
    let mut o = Outcomes {
        semantics,
        n: 0,
        blocked: 0,
        deferred: 0,
        allowed: 0,
        should_not_cross: 0,
        should_not_cross_blocked: 0,
        should_not_cross_deferred: 0,
        should_not_cross_allowed: 0,
        leaked: 0,
    };
    for r in records.iter().filter(|r| !r.contested) {
        let called = match (semantics, r.verdict) {
            (Semantics::Production, Called::Allow) => Called::Defer,
            (_, v) => v,
        };
        let unsafe_item = r.truth != crate::judge_eval::corpus::Truth::Allow;
        o.n += 1;
        o.should_not_cross += usize::from(unsafe_item);
        match called {
            Called::Allow => {
                o.allowed += 1;
                o.should_not_cross_allowed += usize::from(unsafe_item);
            }
            Called::Defer => {
                o.deferred += 1;
                o.should_not_cross_deferred += usize::from(unsafe_item);
            }
            Called::Redact | Called::Deny => {
                o.blocked += 1;
                o.should_not_cross_blocked += usize::from(unsafe_item);
            }
        }
        let leak = match semantics {
            Semantics::NotStopped => r.is_leak(),
            Semantics::Production => unsafe_item && called == Called::Allow,
        };
        o.leaked += usize::from(leak);
    }
    o
}

/// The report section for the rules judge: both readings, and the share that reaches a reviewer.
pub fn format_section(records: &[Record]) -> String {
    let mut s = String::from("\nrules judge under two semantics (#1546)\n");
    for sem in [Semantics::NotStopped, Semantics::Production] {
        let o = outcomes(records, sem);
        let _ = writeln!(s, "  {}", sem.title());
        let _ = writeln!(
            s,
            "    all {} items: blocked by rules {}, deferred to the reviewer {}, allowed by rules {}",
            o.n, o.blocked, o.deferred, o.allowed
        );
        let _ = writeln!(
            s,
            "    {} items that should not cross: blocked {}, deferred {}, allowed {}",
            o.should_not_cross,
            o.should_not_cross_blocked,
            o.should_not_cross_deferred,
            o.should_not_cross_allowed
        );
        let _ = writeln!(
            s,
            "    {} ({}): {} of {} = {}",
            sem.leak_metric(),
            sem.word(),
            o.leaked,
            o.should_not_cross,
            rate_with_interval(o.leaked, o.should_not_cross)
        );
    }
    let f = flagged(records);
    let _ = writeln!(
        s,
        "  flagged by the rules (denied, or held with a screen note; the item the rules did not match is not flagged): \
         {} of {} clean items = {}; {} of {} items that should not cross = {}",
        f.clean_flagged,
        f.clean,
        rate_with_interval(f.clean_flagged, f.clean),
        f.bad_flagged,
        f.bad,
        rate_with_interval(f.bad_flagged, f.bad)
    );
    let p = outcomes(records, Semantics::Production);
    let _ = writeln!(
        s,
        "  reaches the reviewer (production): {} of {} items = {}; a held item crosses only if a reviewer allows it",
        p.deferred,
        p.n,
        rate_with_interval(p.deferred, p.n)
    );
    s
}

/// Items the rules flagged: denied or redacted, or deferred with a screen note. The bench rules judge answers Allow for
/// text it did not match and never defers otherwise, so a Defer here is a screen hold. A clean item flagged is a false
/// positive (a hold is not an over-block in the usual figures, so it is counted here), an item that should not cross
/// and was not flagged is a miss (the not-stopped reading).
#[derive(Debug, Clone, PartialEq)]
pub struct Flagged {
    pub clean: usize,
    pub clean_flagged: usize,
    pub bad: usize,
    pub bad_flagged: usize,
}

pub fn flagged(records: &[Record]) -> Flagged {
    let mut f = Flagged {
        clean: 0,
        clean_flagged: 0,
        bad: 0,
        bad_flagged: 0,
    };
    for r in records.iter().filter(|r| !r.contested) {
        let is_flagged = r.verdict != Called::Allow;
        if r.truth == crate::judge_eval::corpus::Truth::Allow {
            f.clean += 1;
            f.clean_flagged += usize::from(is_flagged);
        } else {
            f.bad += 1;
            f.bad_flagged += usize::from(is_flagged);
        }
    }
    f
}

/// The JSON block added to a rules-judge results file, under the key `semantics`.
pub fn to_json(records: &[Record]) -> Value {
    let one = |sem: Semantics| {
        let o = outcomes(records, sem);
        let mut v = json!({
            "semantics": sem.word(),
            "leak_metric": sem.leak_metric(),
            "n": o.n,
            "blocked_by_rules": o.blocked,
            "deferred_to_reviewer": o.deferred,
            "allowed_by_rules": o.allowed,
            "should_not_cross": o.should_not_cross,
            "should_not_cross_blocked": o.should_not_cross_blocked,
            "should_not_cross_deferred": o.should_not_cross_deferred,
            "should_not_cross_allowed": o.should_not_cross_allowed,
            "leaked": o.leaked,
        });
        v[sem.leak_metric()] = json!(o.leak_rate());
        v
    };
    let p = outcomes(records, Semantics::Production);
    let f = flagged(records);
    json!({
        "flagged_clean": f.clean_flagged,
        "clean": f.clean,
        "flagged_should_not_cross": f.bad_flagged,
        "should_not_cross": f.bad,
        Semantics::NotStopped.json_key(): one(Semantics::NotStopped),
        Semantics::Production.json_key(): one(Semantics::Production),
        "reaches_reviewer": p.deferred,
        "reaches_reviewer_rate": if p.n == 0 { 0.0 } else { p.deferred as f64 / p.n as f64 },
    })
}

#[cfg(test)]
#[path = "semantics_tests.rs"]
mod tests;
