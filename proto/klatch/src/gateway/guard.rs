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
    /// Making a room that does not exist yet, by joining or posting to it (#188).
    CreateRoom {
        room: String,
    },
    /// Inviting `member` to `room` (#188). The pointer itself is a post in a room they share, which is judged as a
    /// post as well.
    Invite {
        member: String,
        room: String,
    },
    /// The room grantor granting `member` a room-scoped clearance in `room` (#893). The label is not in the operation:
    /// the gateway's own rules decide it from the policy table.
    GrantClearance {
        room: String,
        member: String,
    },
    /// A reviewer taking the next queued item to read (#51).
    ReviewNext,
    /// A reviewer recording its verdict on queued item `id` (#51).
    ReviewVerdict {
        id: i64,
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
            Operation::CreateRoom { .. } => "create_room",
            Operation::Invite { .. } => "invite",
            Operation::GrantClearance { .. } => "grant_clearance",
            Operation::ReviewNext => "review_next",
            Operation::ReviewVerdict { .. } => "review_verdict",
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
            | Operation::CreateRoom { room }
            | Operation::Invite { room, .. }
            | Operation::GrantClearance { room, .. }
            | Operation::Ack { room } => Some(room),
            Operation::Admin { .. } | Operation::ReviewNext | Operation::ReviewVerdict { .. } => {
                None
            }
        }
    }

    /// Whether the operation only looks and changes nothing the members could see: what a librarian may do. A read
    /// that moves the cursor is not one, and neither is anything administrative except the two records of what
    /// happened (the audit and the refusals).
    pub fn is_query(&self) -> bool {
        match self {
            Operation::Read { advance, .. } => !advance,
            Operation::Who { .. } | Operation::Claims { .. } | Operation::Subscribe { .. } => true,
            Operation::Admin { action, .. } => {
                matches!(
                    action.as_str(),
                    "audit" | "refusals" | "review-queue" | "label-order" | "scrub-check"
                )
            }
            _ => false,
        }
    }

    /// Whether it is one of the two things a reviewer may do: read the queue and record a verdict.
    pub fn is_review(&self) -> bool {
        matches!(
            self,
            Operation::ReviewNext | Operation::ReviewVerdict { .. }
        )
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

/// The admin acts that edit a floor (R29, D4): the agent cap, the label order, the scrub and deny list, and who may
/// grant room-scoped clearance (the room grantor and the grants themselves, #893), and a member's own post limit,
/// which an agent could otherwise lift for itself (#972), and which member an ssh key label is (#107).
/// Keep this list in step with the `administer` calls that write those policies; the floors tests fail when a
/// listed act stops being refused.
const FLOOR_ACTIONS: [&str; 8] = [
    "cap",
    "chain",
    "scrub-add",
    "scrub-remove",
    "grantor",
    "room-clear",
    "member-rate-limit",
    "ssh-key",
];

/// Whether the admin act edits a floor.
pub(crate) fn is_floor_act(action: &str) -> bool {
    FLOOR_ACTIONS.contains(&action)
}

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

/// The guard of the gateways the person's own commands use (`klatch admin`, `worker`, `anchor`, `run`, `judge`,
/// #1233): every operation needs the principal an `AdminProof` makes, the member `human` with `Verified`
/// attribution, and nothing else passes, a member named `admin`, an unattributed `human` or any other verified
/// member. The name is reserved (no member may take it) and only `proof::AdminProof::principal` makes one on a
/// command line. An agent role (the admin agent) never goes through this guard: it acts on a members' gateway,
/// where the role check and its allowlist decide. It composes with `ForbidFloors` (the launcher commands add it).
pub struct RequireHuman<G: Guard> {
    inner: G,
}

impl<G: Guard> RequireHuman<G> {
    pub fn new(inner: G) -> Self {
        Self { inner }
    }
}

impl<G: Guard> Guard for RequireHuman<G> {
    fn check(&self, who: &Principal, op: &Operation) -> Result<()> {
        if who.member != crate::proof::HUMAN
            || who.attribution != crate::domain::Attribution::Verified
        {
            return Err(Error::Refused {
                rule: "needs-human".into(),
                reason: "this act needs the admin passphrase, proved at a terminal (klatch admin)"
                    .into(),
            });
        }
        self.inner.check(who, op)
    }
}

#[cfg(test)]
mod require_human_tests {
    use super::*;

    fn ask(who: &Principal) -> Result<()> {
        RequireHuman::new(AllowAll).check(who, &Operation::admin("pause", "*"))
    }

    #[test]
    fn only_the_verified_human_passes() {
        assert!(ask(&Principal::verified("human")).is_ok());
        for bad in [
            Principal::unattributed("human"),
            Principal::unattributed("admin"),
            Principal::verified("admin"),
            Principal::verified("mallory"),
            Principal::verified("Human"),
            Principal::verified("human "),
            Principal::verified(""),
        ] {
            let e = ask(&bad).unwrap_err();
            assert!(
                matches!(&e, Error::Refused { rule, .. } if rule == "needs-human"),
                "{bad:?}: {e:?}"
            );
        }
    }

    #[test]
    fn it_refuses_every_kind_of_operation_not_only_admin_acts() {
        let g = RequireHuman::new(AllowAll);
        let nobody = Principal::unattributed("bob");
        for op in [
            Operation::admin("audit", ""),
            Operation::read("r", false),
            Operation::Join { room: "r".into() },
            Operation::ReviewNext,
        ] {
            assert!(g.check(&nobody, &op).is_err(), "{op:?}");
        }
    }

    #[test]
    fn the_name_is_one_no_member_can_take() {
        assert!(crate::domain::is_reserved_name(crate::proof::HUMAN));
    }

    #[test]
    fn it_still_asks_the_guard_it_wraps() {
        let g = RequireHuman::new(ForbidFloors::new(AllowAll));
        let human = Principal::verified("human");
        assert!(g.check(&human, &Operation::admin("pause", "*")).is_ok());
        assert!(g.check(&human, &Operation::admin("cap", "x")).is_err());
    }
}
