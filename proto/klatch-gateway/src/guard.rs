use klatch_core::{Principal, Result};

/// What a caller is trying to do, as seen by the guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Join { room: String },
    Post { room: String, body: String },
    Read { room: String, advance: bool },
    Who { room: String },
    Claim { room: String, path: String },
    Release { room: String, path: String },
    Claims { room: String },
}

/// The one hook point for policy. The gateway calls it once per operation, before the store is touched.
/// Returning an error refuses the operation; the label and clearance check (DESIGN D3) will be a guard.
pub trait Guard {
    fn check(&self, who: &Principal, op: &Operation) -> Result<()>;
}

/// The guard used until a policy exists: allows everything.
pub struct AllowAll;

impl Guard for AllowAll {
    fn check(&self, _who: &Principal, _op: &Operation) -> Result<()> {
        Ok(())
    }
}
