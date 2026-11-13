#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub sender: String,
    pub body: String,
    pub ts: i64,
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
