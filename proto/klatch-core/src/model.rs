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
