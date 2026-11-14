use klatch_core::{Principal, Result};

/// What a caller is trying to do, as seen by the guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Join {
        room: String,
    },
    Post {
        room: String,
        body: String,
    },
    Read {
        room: String,
        advance: bool,
    },
    Who {
        room: String,
    },
    Claim {
        room: String,
        path: String,
    },
    Release {
        room: String,
        path: String,
    },
    Claims {
        room: String,
    },
    /// Reading a room as a stream: `tail` and `head`.
    Subscribe {
        room: String,
    },
    /// Setting the caller's own status in a room.
    Status {
        room: String,
        status: String,
    },
    /// Acknowledging messages, which moves the member's read cursor.
    Ack {
        room: String,
    },
    /// Muting or unmuting a room for the caller.
    Mute {
        room: String,
        muted: bool,
    },
    /// An administrative act (`klatch admin`): close or reopen a room, pause or resume all, remove or allow a
    /// member. `action` names it; `target` is the room or member it is about.
    Admin {
        action: String,
        target: String,
    },
}

impl Operation {
    /// An administrative or cross-member act the guard decides: `action` on `target`.
    pub fn admin(action: &str, target: &str) -> Self {
        Operation::Admin {
            action: action.into(),
            target: target.into(),
        }
    }
    pub fn read(room: &str, advance: bool) -> Self {
        Operation::Read {
            room: room.into(),
            advance,
        }
    }

    pub fn claims(room: &str) -> Self {
        Operation::Claims { room: room.into() }
    }

    pub fn release(room: &str, path: &str) -> Self {
        Operation::Release {
            room: room.into(),
            path: path.into(),
        }
    }

    /// What the operation is called in the record of refusals.
    pub fn name(&self) -> &'static str {
        match self {
            Operation::Join { .. } => "join",
            Operation::Post { .. } => "post",
            Operation::Read { .. } => "read",
            Operation::Who { .. } => "who",
            Operation::Claim { .. } => "claim",
            Operation::Release { .. } => "release",
            Operation::Claims { .. } => "claims",
            Operation::Subscribe { .. } => "subscribe",
            Operation::Status { .. } => "status",
            Operation::Ack { .. } => "ack",
            Operation::Mute { .. } => "mute",
            Operation::Admin { .. } => "admin",
        }
    }

    /// The room the operation is about, if it is about one.
    pub fn room(&self) -> Option<&str> {
        match self {
            Operation::Join { room }
            | Operation::Post { room, .. }
            | Operation::Read { room, .. }
            | Operation::Who { room }
            | Operation::Claim { room, .. }
            | Operation::Release { room, .. }
            | Operation::Claims { room }
            | Operation::Subscribe { room }
            | Operation::Status { room, .. }
            | Operation::Mute { room, .. }
            | Operation::Ack { room } => Some(room),
            Operation::Admin { .. } => None,
        }
    }

    /// Whether a closed room refuses it: posting, claiming and setting a status. A closed room stays readable.
    pub fn changes_a_closed_room(&self) -> bool {
        matches!(
            self,
            Operation::Post { .. } | Operation::Claim { .. } | Operation::Status { .. }
        )
    }
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
