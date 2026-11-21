//! The answer to "could member X answer ask N, and if not, why" (#671, part 1 of the decision on #627). It carries
//! member names, an ask number and stable rule words with one-line reasons; never any message text.

/// One thing that stops a member from answering an ask: a stable rule word and a one-line reason that names the next
/// step where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub rule: String,
    pub reason: String,
}

/// Whether `member` could answer ask `ask` now, with every reason it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CouldAnswer {
    pub member: String,
    pub ask: i64,
    /// Empty when the member could answer.
    pub blockers: Vec<Blocker>,
}

impl CouldAnswer {
    /// Nothing stops the member.
    pub fn could(&self) -> bool {
        self.blockers.is_empty()
    }

    /// The reasons on one line, for display: `clearance: ...; busy: ...`, or `yes`.
    pub fn summary(&self) -> String {
        if self.could() {
            return "yes".into();
        }
        self.blockers
            .iter()
            .map(|b| format!("{}: {}", b.rule, b.reason))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_is_yes_or_each_reason() {
        let mut c = CouldAnswer {
            member: "m".into(),
            ask: 3,
            blockers: vec![],
        };
        assert_eq!((c.could(), c.summary().as_str()), (true, "yes"));
        c.blockers = vec![
            Blocker {
                rule: "busy".into(),
                reason: "m holds a claim".into(),
            },
            Blocker {
                rule: "credential".into(),
                reason: "expired".into(),
            },
        ];
        assert!(!c.could());
        assert_eq!(c.summary(), "busy: m holds a claim; credential: expired");
    }
}
