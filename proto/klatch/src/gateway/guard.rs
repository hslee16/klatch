use crate::domain::{Error, Principal, Result};

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

    /// Whether the operation only looks and changes nothing the members could see: what a librarian may do. A read
    /// that moves the cursor is not one, and neither is anything administrative except the two records of what
    /// happened (the audit and the refusals).
    pub fn is_query(&self) -> bool {
        match self {
            Operation::Read { advance, .. } => !advance,
            Operation::Who { .. } | Operation::Claims { .. } | Operation::Subscribe { .. } => true,
            Operation::Admin { action, .. } => matches!(action.as_str(), "audit" | "refusals"),
            _ => false,
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

/// The admin acts that edit a floor (R29, D4): the agent cap, the label order, and the scrub and deny list.
/// Keep this list in step with the `administer` calls that write those policies; the floors tests fail when a
/// listed act stops being refused.
const FLOOR_ACTIONS: [&str; 4] = ["cap", "chain", "scrub-add", "scrub-remove"];

/// Refuses every act that edits a floor, whoever asks, and leaves every other decision to the guard it wraps.
/// Every gateway an agent can reach uses it; only the terminal-gated `klatch admin` gateway does not, because a
/// person typing at a terminal is the one party allowed to set the floors. Defence in depth: the wire protocol
/// has no admin method at all, so this holds even if one is added or a caller embeds the gateway.
pub struct ForbidFloors<G: Guard> {
    inner: G,
}

impl<G: Guard> ForbidFloors<G> {
    pub fn new(inner: G) -> Self {
        Self { inner }
    }
}

impl<G: Guard> Guard for ForbidFloors<G> {
    fn check(&self, who: &Principal, op: &Operation) -> Result<()> {
        if let Operation::Admin { action, .. } = op
            && FLOOR_ACTIONS.contains(&action.as_str())
        {
            return Err(Error::Refused {
                rule: "floors".into(),
                reason: format!(
                    "{action} edits a floor; floors are set by the person at a terminal (klatch admin), never by an agent"
                ),
            });
        }
        self.inner.check(who, op)
    }
}
