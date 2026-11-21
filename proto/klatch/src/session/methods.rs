//! The plain request methods: each reads its parameters, calls the gateway as the session's principal and shapes
//! the answer.

use crate::domain::{Claim, Error, MemberInfo, MemberKind, Message, Posted, Principal};
use crate::gateway::{ClaimSpec, Gateway};
use crate::protocol::ServerFrame;
use crate::session::frames::{fail_with, failure, message_json, ok};
use crate::store::Store;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// One request being answered: its id, method and parameters.
pub struct Call {
    pub id: String,
    pub method: String,
    pub value: Value,
}

#[derive(Deserialize)]
pub(crate) struct RoomParams {
    pub room: String,
}
#[derive(Deserialize)]
struct CouldAnswerParams {
    room: String,
    member: String,
    ask: i64,
}
#[derive(Deserialize)]
struct ReceiptParams {
    room: String,
    seq: i64,
}
#[derive(Deserialize)]
struct CrossingsParams {
    #[serde(default)]
    limit: Option<usize>,
}
#[derive(Deserialize)]
struct PostParams {
    room: String,
    body: String,
    #[serde(default)]
    label: Option<String>,
    /// A handoff: a kind, and optionally an opaque payload (which needs a kind).
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    payload: Option<String>,
}
#[derive(Deserialize)]
struct ReadParams {
    room: String,
    #[serde(default)]
    peek: bool,
}
#[derive(Deserialize)]
struct HistoryParams {
    room: String,
    #[serde(default)]
    since: Option<i64>,
    #[serde(default)]
    last: Option<usize>,
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
struct DmParams {
    to: String,
    body: String,
    #[serde(default)]
    label: Option<String>,
}
#[derive(Deserialize)]
struct NoParams {}
#[derive(Deserialize)]
struct ScrubCheckParams {
    text: String,
}
#[derive(Deserialize)]
struct MuteParams {
    room: String,
    muted: bool,
}
#[derive(Deserialize)]
struct ScopeParams {
    scope: String,
}
#[derive(Deserialize)]
struct KindParams {
    /// `bot`, `service` or `human`; null or absent withdraws the declaration.
    kind: Option<String>,
}
#[derive(Deserialize)]
struct StatusParams {
    room: String,
    status: String,
}
#[derive(Deserialize)]
struct InviteParams {
    member: String,
    room: String,
    via: String,
}
#[derive(Deserialize)]
struct GrantParams {
    room: String,
    member: String,
    label: String,
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
            if m.status_ts > 0 {
                v["status_ts"] = json!(m.status_ts);
            }
        }
        if let Some(kind) = m.kind {
            v["kind"] = json!(kind.word());
        }
        if let Some(mode) = &m.mode {
            v["mode"] = json!(mode);
        }
        if let Some(place) = &m.place {
            if let Some(r) = &place.repository {
                v["repository"] = json!(r);
            }
            if let Some(d) = &place.directory {
                v["directory"] = json!(d);
            }
        }
        v
    };
    json!({ "members": ms.iter().map(one).collect::<Vec<_>>() })
}

/// The wire `kind` method: a word of the closed vocabulary, or none to withdraw the declaration.
fn declare_kind<S: Store>(
    g: &Gateway<S>,
    who: &Principal,
    word: Option<&str>,
) -> Result<(), Error> {
    let kind = match word {
        None => None,
        Some(w) => Some(
            MemberKind::parse(w)
                .ok_or_else(|| Error::Invalid("kind is bot, service or human, or null".into()))?,
        ),
    };
    g.set_kind(who, kind)
}

/// The open asks of a room as the wire shows them (#901): number, asker, text (other members' words, so data), age
/// anchor, whether a live claim exists, whether the ask waits on its asker, the working room's name if any, and the member whose dm a moved ask went to (a name only, #1188).
fn asks_json(asks: &[crate::domain::OpenAsk]) -> Value {
    let one = |a: &crate::domain::OpenAsk| {
        json!({
            "seq": a.seq, "sender": a.sender, "text": a.text, "ts": a.ts, "claimed": a.claimed,
            "waiting_on_asker": a.waiting_on_asker.is_some(),
            "room": a.room.as_ref().map(|r| r.name.clone()),
            "moved_to": a.moved.as_ref().filter(|m| m.room.is_some()).map(|m| m.counterpart(&a.sender)),
        })
    };
    json!({ "asks": asks.iter().map(one).collect::<Vec<_>>() })
}

/// The caller's own crossings (#961): ids, the source pointer, the target room, a state and one fixed sentence. Never
/// the text, a redaction, a reviewer's reason or the name of a rule or term.
fn crossings_json(list: &[crate::domain::CrossingStatus]) -> Value {
    let one = |c: &crate::domain::CrossingStatus| {
        json!({
            "id": c.id, "room": c.room, "seq": c.seq, "to_room": c.to_room, "state": c.state, "why": c.why,
        })
    };
    json!({ "crossings": list.iter().map(one).collect::<Vec<_>>() })
}

/// The receipts of the caller's own message (#848): one row per recipient with a liveness state word, the seconds
/// that state names, and the shown and read flags. Never the message text.
fn receipts_json(r: &crate::domain::Receipts, now: i64) -> Value {
    use crate::domain::Receipt;
    let one = |(name, state): &(String, Receipt)| {
        let (word, secs) = match state {
            Receipt::Parked { back_by } => ("parked", (back_by - now).max(0)),
            Receipt::SeenAfter { secs } => ("seen_after", *secs),
            Receipt::NotSeen { secs_before } => ("not_seen", *secs_before),
        };
        let taken = r.taken.iter().find(|(n, _)| n == name).map(|(_, t)| *t);
        json!({
            "name": name, "state": word, "secs": secs,
            "shown": taken.is_some_and(|t| t.shown), "read": taken.is_some_and(|t| t.read),
        })
    };
    json!({ "seq": r.seq, "posted": r.posted, "recipients": r.rows.iter().map(one).collect::<Vec<_>>() })
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

/// The stuck-case events as the wire shows them (#271): a kind, a stable key, and ids, names and numbers; no text.
fn stuck_json(events: &[crate::domain::StuckEvent]) -> Value {
    use crate::domain::StuckEvent;
    let one = |e: &StuckEvent| {
        let mut v = json!({ "kind": e.kind(), "key": e.key() });
        match e {
            StuckEvent::StaleAsk {
                ask,
                asker,
                age_secs,
            } => {
                v["ask"] = json!(ask);
                v["asker"] = json!(asker);
                v["age_secs"] = json!(age_secs);
            }
            StuckEvent::NobodyCapable { ask, asker, cannot } => {
                v["ask"] = json!(ask);
                v["asker"] = json!(asker);
                v["cannot"] = json!(cannot);
            }
            StuckEvent::AskCycle { members, asks } => {
                v["members"] = json!(members);
                v["asks"] = json!(asks);
            }
            StuckEvent::ClaimDeadlock { members } => {
                v["members"] = json!(members);
            }
        }
        v
    };
    json!({ "events": events.iter().map(one).collect::<Vec<_>>() })
}

/// Session-start housekeeping, every member's: remove the members not seen for the fixed retention. It takes no
/// parameters, so a caller cannot name a shorter age and prune members that are still in use.
fn prune<S: Store>(g: &Gateway<S>, who: &Principal, id: String) -> Vec<ServerFrame> {
    match g.prune_members(who, crate::domain::MEMBER_RETENTION_SECS) {
        Ok(removed) => vec![ok(id, json!({ "removed": removed }))],
        Err(e) => failure(id, &e),
    }
}

/// Answer a request that is a plain call through the gateway.
pub(crate) fn handle<S: Store>(
    g: &Gateway<S>,
    who: &Principal,
    call: Call,
    harness: Option<&dyn crate::harness::Harness>,
) -> Vec<ServerFrame> {
    let empty = |()| json!({});
    match call.method.clone().as_str() {
        "post" => call.run(
            |p: PostParams| match (p.kind.as_deref(), p.payload.as_deref()) {
                (None, None) => g.post_labeled(who, (&p.room, &p.body, p.label.as_deref())),
                (None, Some(_)) => Err(crate::domain::Error::Invalid(
                    "a payload needs a kind".into(),
                )),
                (Some(kind), payload) => g.post_handoff(who, (&p.room, &p.body, p.label.as_deref()), &crate::gateway::Handoff { kind, payload }),
            },
            |x: Posted| json!({ "seq": x.seq, "id": x.id }),
        ),
        "dm" => call.run(
            |p: DmParams| g.dm(who, &p.to, &p.body, p.label.as_deref()),
            |(room, x): (String, Posted)| json!({ "room": room, "seq": x.seq, "id": x.id }),
        ),
        "read" => call.run(
            |p: ReadParams| g.read(who, &p.room, !p.peek),
            |ms: Vec<Message>| json!({ "messages": ms.iter().map(message_json).collect::<Vec<_>>() }),
        ),
        "history" => call.run(
            |p: HistoryParams| {
                let range = crate::gateway::HistoryRange {
                    since: p.since,
                    last: p.last,
                };
                g.history(who, &p.room, range)
            },
            |h: crate::gateway::History| {
                json!({ "messages": h.messages.iter().map(message_json).collect::<Vec<_>>(), "more": h.more })
            },
        ),
        "join" => call.run(
            |p: RoomParams| match harness {
                Some(h) => g.join_with_harness(who, &p.room, h).map(|_| ()),
                None => g.join(who, &p.room),
            },
            empty,
        ),
        "asks" => call.run(|p: RoomParams| g.open_asks(who, &p.room), |a| asks_json(&a)),
        "my_crossings" => call.run(
            |p: CrossingsParams| g.my_crossings(who, p.limit.unwrap_or(20).min(200)),
            |l| crossings_json(&l),
        ),
        "my_rooms" => call.run(|_: NoParams| g.rooms(who), |r| json!({ "rooms": r })),
        "board" => call.run(
            |p: RoomParams| g.board(who, &p.room),
            |b: crate::domain::Board| {
                json!({ "room": b.room, "board": crate::domain::render_board(&b, g.now()) })
            },
        ),
        "receipt" => call.run(
            |p: ReceiptParams| g.receipt(who, &p.room, p.seq),
            |r| receipts_json(&r, g.now()),
        ),
        "my_role" => call.run(|_: NoParams| g.my_role(who), |r| json!({ "role": r })),
        "invite" => call.run(
            |p: InviteParams| g.invite(who, &p.member, &p.room, &p.via),
            empty,
        ),
        "grant_room_clearance" => call.run(
            |p: GrantParams| g.grant_room_clearance(who, &p.room, &p.member, &p.label),
            empty,
        ),
        "who" => call.run(|p: RoomParams| g.who(who, &p.room), |ms| members_json(&ms)),
        "status" => call.run(|p: StatusParams| g.set_status(who, &p.room, &p.status), empty),
        "claim" => call.run(
            |p: ClaimParams| {
                let ttl = g.claim_ttl_for(who, (&p.room, &p.path), p.ttl)?;
                g.claim(who, &p.room, &ClaimSpec::new(&p.path, ttl, &p.reason).shared(p.shared))
            },
            empty,
        ),
        "mute" => call.run(|p: MuteParams| g.set_muted(who, &p.room, p.muted), empty),
        "scope" => call.run(|p: ScopeParams| g.set_scope(who, &p.scope), empty),
        "kind" => call.run(|p: KindParams| declare_kind(g, who, p.kind.as_deref()), empty),
        "release" => call.run(
            |p: ReleaseParams| match p.note.as_deref() {
                Some(n) => g.release_with_note(who, (&p.room, &p.path), n),
                None => g.release(who, &p.room, &p.path),
            },
            empty,
        ),
        "claims" => call.run(|p: RoomParams| g.claims(who, &p.room), |cs| claims_json(&cs)),
        "stuck" => call.run(|p: RoomParams| g.stuck(who, &p.room), |ev| stuck_json(&ev)),
        "could_answer" => call.run(
            |p: CouldAnswerParams| g.could_answer(who, &p.room, &p.member, p.ask),
            |c: crate::domain::CouldAnswer| {
                json!({
                    "member": c.member,
                    "ask": c.ask,
                    "could": c.could(),
                    "blockers": c.blockers.iter().map(|b| json!({ "rule": b.rule, "reason": b.reason })).collect::<Vec<_>>(),
                })
            },
        ),
        "label_order" => call.run(
            |_: NoParams| g.label_order(who),
            |(chain, cap): (Vec<String>, String)| json!({ "chain": chain, "cap": cap }),
        ),
        "scrub_check" => call.run(
            |p: ScrubCheckParams| g.scrub_check(who, &p.text),
            |scrubbed: bool| json!({ "scrubbed": scrubbed }),
        ),
        "prune" => prune(g, who, call.id),
        "role_act" | "review_next" | "review_verdict" => {
            crate::session::role_methods::handle(g, who, call)
        }
        other => vec![fail_with(
            call.id,
            "unknown_method",
            format!("unknown method {other}"),
        )],
    }
}
