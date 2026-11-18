//! The `Judge` seam (DESIGN D4, R31). A judge screens one queued message for channel safety and nothing else: label
//! leaks, scrubber and deny-list matches, injection-shaped text. It never rates the quality or correctness of any
//! agent's work (the non-goal in section 3). Klatch works with no judge: `NullJudge` decides nothing, so an item
//! stays queued, and label enforcement, which is inline and fails closed, never waits on a judge.

/// A queued message as a judge sees it: the message text loaded when the item is judged (the queue keeps no copy)
/// and why it was queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewItem {
    pub id: i64,
    pub room: String,
    pub seq: i64,
    /// The label the message would cross to.
    pub to_label: String,
    /// The label the message was posted under. Empty when unknown, which a label-gated judge treats as above its
    /// gate.
    pub from_label: String,
    pub reason: String,
    pub text: String,
}

/// What a judge returns. `Defer` means "no decision": the item stays queued, which is how an unreachable or absent
/// judge shows up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Allow with this text in place of the original.
    Redact(String),
    Deny(String),
    Defer,
}

impl Verdict {
    /// The word recorded in the queue and the audit.
    pub fn word(&self) -> &'static str {
        match self {
            Verdict::Allow => "allow",
            Verdict::Redact(_) => "redact",
            Verdict::Deny(_) => "deny",
            Verdict::Defer => "defer",
        }
    }
}

/// One function: take a queued item, return a verdict. Channel safety only.
pub trait Judge {
    fn judge(&self, item: &ReviewItem) -> Verdict;
}

/// The default judge: decides nothing.
pub struct NullJudge;

impl Judge for NullJudge {
    fn judge(&self, _item: &ReviewItem) -> Verdict {
        Verdict::Defer
    }
}
