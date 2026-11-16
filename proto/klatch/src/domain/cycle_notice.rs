//! A cycle of waits that a claim being taken completed (DESIGN section 32 item 82, #648).

/// A member's own refused claim that, after another member took a claim, closes a cycle of waits: it waits for
/// `holder`, who waits, directly or through others, for something this member holds. Reported once, so the member
/// can yield as a refusal that closes a cycle tells it to (DESIGN 25.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleNotice {
    /// What the member was refused.
    pub path: String,
    /// Who holds it now.
    pub holder: String,
}
