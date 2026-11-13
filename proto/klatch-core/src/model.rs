/// A message as delivered. `id` is globally unique and opaque; `seq` is dense and per room, and is
/// what people and cursors refer to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub seq: i64,
    pub sender: String,
    pub body: String,
    pub ts: i64,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub path: String,
    pub owner: String,
    pub reason: String,
    /// Unix seconds at which the claim lapses.
    pub expires: i64,
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
