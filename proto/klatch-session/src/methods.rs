//! The plain request methods: each reads its parameters, calls the gateway as the session's principal and shapes
//! the answer.

use crate::frames::{fail_with, failure, message_json, ok};
use klatch_core::{Claim, Error, MemberInfo, Message, Posted, Principal};
use klatch_gateway::Gateway;
use klatch_protocol::ServerFrame;
use klatch_store::Store;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// One request being answered: its id, method and parameters.
pub(crate) struct Call {
    pub id: String,
    pub method: String,
    pub value: Value,
}

#[derive(Deserialize)]
pub(crate) struct RoomParams {
    pub room: String,
}
#[derive(Deserialize)]
struct PostParams {
    room: String,
    body: String,
}
#[derive(Deserialize)]
struct ReadParams {
    room: String,
    #[serde(default)]
    peek: bool,
}
#[derive(Deserialize)]
struct ClaimParams {
    room: String,
    path: String,
    ttl: Option<i64>,
    #[serde(default)]
    reason: String,
    /// A shared claim conflicts with every scope.
    #[serde(default)]
    shared: bool,
}
#[derive(Deserialize)]
struct ScopeParams {
    scope: String,
}
#[derive(Deserialize)]
struct StatusParams {
    room: String,
    status: String,
}
#[derive(Deserialize)]
struct ReleaseParams {
    room: String,
    path: String,
    #[serde(default)]
    note: Option<String>,
}

/// The parameters of a request, or the frames that refuse it.
pub(crate) fn params<T: DeserializeOwned>(id: &str, value: Value) -> Result<T, Vec<ServerFrame>> {
    serde_json::from_value(value).map_err(|_| {
        vec![fail_with(
            id.to_string(),
            "invalid_params",
            "the request parameters are missing or have the wrong shape".into(),
        )]
    })
}

impl Call {
    /// Read the parameters, run `act`, and answer with `shape` of what it returned (or with the failure).
    fn run<P: DeserializeOwned, T>(
        self,
        act: impl FnOnce(P) -> Result<T, Error>,
        shape: impl FnOnce(T) -> Value,
    ) -> Vec<ServerFrame> {
        let p = match params::<P>(&self.id, self.value) {
            Ok(p) => p,
            Err(frames) => return frames,
        };
        match act(p) {
            Ok(v) => vec![ok(self.id, shape(v))],
            Err(e) => failure(self.id, &e),
        }
    }
}

fn members_json(ms: &[MemberInfo]) -> Value {
    let one = |m: &MemberInfo| {
        let mut v = json!({"name": m.name, "last_seen": m.last_seen});
        if let Some(status) = &m.status {
            v["status"] = json!(status);
        }
        v
    };
    json!({ "members": ms.iter().map(one).collect::<Vec<_>>() })
}

fn claims_json(cs: &[Claim]) -> Value {
    let one = |c: &Claim| {
        let mut v = json!({
            "path": c.path, "owner": c.owner, "reason": c.reason, "expires": c.expires
        });
        if !c.scope.is_empty() {
            v["scope"] = json!(c.scope);
        }
        v
    };
    json!({ "claims": cs.iter().map(one).collect::<Vec<_>>() })
}

/// Answer a request that is a plain call through the gateway.
pub(crate) fn handle<S: Store>(g: &Gateway<S>, who: &Principal, call: Call) -> Vec<ServerFrame> {
    let empty = |()| json!({});
    match call.method.clone().as_str() {
        "post" => call.run(
            |p: PostParams| g.post(who, &p.room, &p.body),
            |x: Posted| json!({ "seq": x.seq, "id": x.id }),
        ),
        "read" => call.run(
            |p: ReadParams| g.read(who, &p.room, !p.peek),
            |ms: Vec<Message>| json!({ "messages": ms.iter().map(message_json).collect::<Vec<_>>() }),
        ),
        "who" => call.run(|p: RoomParams| g.who(who, &p.room), |ms| members_json(&ms)),
        "status" => call.run(|p: StatusParams| g.set_status(who, &p.room, &p.status), empty),
        "claim" => call.run(
            |p: ClaimParams| {
                let ttl = klatch_core::claim_ttl(&p.path, p.ttl);
                g.claim_with(who, &p.room, &p.path, ttl, &p.reason, p.shared)
            },
            empty,
        ),
        "scope" => call.run(|p: ScopeParams| g.set_scope(who, &p.scope), empty),
        "release" => call.run(
            |p: ReleaseParams| match p.note.as_deref() {
                Some(n) => g.release_with_note(who, &p.room, &p.path, n),
                None => g.release(who, &p.room, &p.path),
            },
            empty,
        ),
        "claims" => call.run(|p: RoomParams| g.claims(who, &p.room), |cs| claims_json(&cs)),
        other => vec![fail_with(
            call.id,
            "unknown_method",
            format!("unknown method {other}"),
        )],
    }
}
