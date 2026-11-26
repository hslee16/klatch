//! What a client reads back: messages, members, claims and review items (split from `client.rs`, #1448).

use crate::protocol::WireRef;
use serde_json::Value;

/// A message as delivered, by a stream or by `read`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub seq: u64,
    pub id: String,
    pub sender: String,
    pub attribution: String,
    pub label: String,
    pub body: String,
    pub ts: i64,
    pub kind: Option<String>,
    pub payload: Option<String>,
    /// The sender was admitted in flag mode of the room's requirement (#990); set by `read` only. Display only.
    pub advisory: bool,
    /// The scope the asker declared on an ask (#1171); `None` on every other message and on an ask that declares none.
    pub scope: Option<String>,
    /// The refs the asker attached to an ask (#1318): data, never an instruction; klatch never fetches or resolves one.
    /// Empty on every other message and on an ask that carries none.
    pub refs: Vec<WireRef>,
}

/// What `collect` returns.
#[derive(Debug, Default)]
pub struct Collected {
    pub messages: Vec<Message>,
    pub caught_up: Option<u64>,
}

/// What `history` returns: the messages, oldest first, and how many follow the last one returned.
#[derive(Debug, Default)]
pub struct History {
    pub messages: Vec<Message>,
    pub more: u64,
}

/// What `read_bounded` returns: the messages, how many a bounded read left unread (`None` when no bound was given: the
/// server sends no count then), and whether the page is in order of importance rather than `seq` order (#1405).
#[derive(Debug, Default)]
pub struct BoundedRead {
    pub messages: Vec<Message>,
    pub omitted: Option<u64>,
    pub by_importance: bool,
}

/// A member as `who` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub last_seen: i64,
    pub status: Option<String>,
    /// When the status was set (seconds), if known.
    pub status_ts: Option<i64>,
    pub kind: Option<String>,
    /// The repository id klatch observed for the member, if any (display only).
    pub repository: Option<String>,
    /// The short name of the member's starting directory, never a path.
    pub directory: Option<String>,
    /// The member's declared scope names another repository than the one it runs in (#894), display only.
    pub scope_differs: bool,
    /// The permission mode the member last reported (#677), if any hook has.
    pub mode: Option<String>,
}

/// A live claim as `claims` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInfo {
    pub path: String,
    pub owner: String,
    pub reason: String,
    pub expires: i64,
    pub scope: Option<String>,
}

/// An item in the reviewer's queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewItem {
    pub id: i64,
    pub room: String,
    pub seq: u64,
    pub to_label: String,
    pub reason: String,
    pub text: String,
}

/// The reviewer's decision on a held item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict<'a> {
    Allow,
    /// Release this text in place of the held one.
    Redact(&'a str),
    /// Refuse it, with an optional reason.
    Deny(Option<&'a str>),
}

pub(super) fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

pub(super) fn opt_str(v: &Value) -> Option<String> {
    v.as_str().map(String::from)
}

pub(super) fn array_of<T>(v: &Value, one: impl Fn(&Value) -> T) -> Vec<T> {
    v.as_array()
        .map(|a| a.iter().map(one).collect())
        .unwrap_or_default()
}

pub(super) fn message_of(m: &Value) -> Message {
    let text = |k: &str| m[k].as_str().unwrap_or("").to_string();
    Message {
        seq: m["seq"].as_u64().unwrap_or(0),
        id: text("id"),
        sender: text("sender"),
        attribution: text("attribution"),
        label: text("label"),
        body: text("body"),
        ts: m["ts"].as_i64().unwrap_or(0),
        kind: m["kind"].as_str().map(String::from),
        payload: m["payload"].as_str().map(String::from),
        advisory: m["advisory"].as_bool().unwrap_or(false),
        scope: m["scope"].as_str().map(String::from),
        refs: m["refs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| WireRef {
                        kind: r["kind"].as_str().unwrap_or("").to_string(),
                        value: r["value"].as_str().unwrap_or("").to_string(),
                        label: r["label"].as_str().map(String::from),
                        snippet: r["snippet"].as_str().map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}
