/// A message as delivered. `id` is globally unique and opaque; `seq` is dense and per room, and is
/// what people and cursors refer to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub seq: i64,
    pub sender: String,
    /// Whether the gateway proved who the sender was when the message was posted (R23).
    pub attribution: Attribution,
    /// The sensitivity label the message was posted under.
    pub label: String,
    pub body: String,
    pub ts: i64,
}

impl Message {
    /// The sender as a reader should see it: the name, and `(unverified)` after it when the gateway could not prove who
    /// sent it (R23, ticket #28). A proven sender is the bare name.
    pub fn sender_label(&self) -> String {
        match self.attribution {
            Attribution::Verified => self.sender.clone(),
            Attribution::Unattributed => format!("{} (unverified)", self.sender),
        }
    }
}

/// What a successful post returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub id: String,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInfo {
    pub name: String,
    pub last_seen: i64,
    /// The status the member set with the `status` operation (for example `done`), if any.
    pub status: Option<String>,
    /// The member whose session started this one (a headless worker started from an interactive session), if known.
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub path: String,
    pub owner: String,
    pub reason: String,
    /// Unix seconds at which the claim lapses.
    pub expires: i64,
    /// The scope the claim lives in: empty for a member with none, `*` for a shared or room-wide claim, otherwise
    /// the claimer's scope (DESIGN section 27).
    pub scope: String,
}

/// A claim a member was shown earlier that is no longer held (released, or lapsed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasedClaim {
    pub owner: String,
    pub path: String,
    /// What the holder said it changed when it released (#272), if anything.
    pub note: Option<String>,
}

/// One administrative act (`klatch admin ...`), kept so the human can see what was done and when (R11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub ts: i64,
    pub action: String,
    pub target: String,
    pub detail: String,
}

/// One refused call, kept so the refusal can be seen and counted (R27). It holds the rule that refused and never the
/// text that triggered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusalRecord {
    pub ts: i64,
    pub member: String,
    pub room: String,
    pub operation: String,
    pub rule: String,
}

/// The latest release of a path that carried a note: who released it, what they said, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseNote {
    pub owner: String,
    pub note: String,
    pub ts: i64,
}

/// What changed in other members' claims since a member was last shown them: claims it has not seen (or
/// that were renewed with a new expiry) and claims it saw that are gone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimChanges {
    pub added: Vec<Claim>,
    pub released: Vec<ReleasedClaim>,
}

/// Whether the gateway could prove who is calling. An unattributed caller must be shown as such (R23).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attribution {
    Verified,
    Unattributed,
}

/// The caller of a gateway operation. Produced at the transport edge from evidence about the peer; the
/// gateway never trusts a name carried inside a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub member: String,
    pub attribution: Attribution,
}

impl Principal {
    pub fn verified(member: &str) -> Self {
        Self {
            member: member.into(),
            attribution: Attribution::Verified,
        }
    }

    pub fn unattributed(member: &str) -> Self {
        Self {
            member: member.into(),
            attribution: Attribution::Unattributed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principals_carry_their_attribution() {
        assert_eq!(Principal::verified("a").attribution, Attribution::Verified);
        assert_eq!(
            Principal::unattributed("a").attribution,
            Attribution::Unattributed
        );
        assert_eq!(Principal::verified("a").member, "a");
    }
}

/// The latest run of one hook event for a member: when, how many times so far, and how much it printed.
/// Never the content, only its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRun {
    pub event: String,
    pub last_ts: i64,
    pub runs: i64,
    pub last_bytes: i64,
}
