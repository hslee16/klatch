//! The wire methods a role-holder uses (#506, #509): `role_act` for the admin agent and `review_next` and
//! `review_verdict` for the reviewer. They are the only admin-shaped methods on the wire. The table below is an
//! allowlist, so no floor act (cap, label order, scrub list) and no other human-only act can be named here, and the
//! gateway still refuses a member that does not hold the role, so a method name alone grants nothing.

use super::methods::Call;
use crate::domain::{AuditRecord, Error, Principal, RefusalRecord, ReviewItem, Verdict};
use crate::gateway::Gateway;
use crate::protocol::ServerFrame;
use crate::session::frames::{fail_with, failure, ok};
use crate::store::Store;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct ActParams {
    act: String,
    #[serde(default)]
    room: Option<String>,
    #[serde(default)]
    member: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct VerdictParams {
    id: i64,
    verdict: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// The acts of the admin agent that have a wire form. The rest of its list (runs, anchors) needs a file or an
/// adapter on the server side and is not here.
const WIRE_ACTS: [&str; 8] = [
    "close", "reopen", "remove", "allow", "clear", "label", "audit", "refusals",
];

fn invalid(id: String) -> Vec<ServerFrame> {
    vec![fail_with(
        id,
        "invalid_params",
        "the request parameters are missing or have the wrong shape".into(),
    )]
}

pub(crate) fn handle<S: Store>(g: &Gateway<S>, who: &Principal, call: Call) -> Vec<ServerFrame> {
    match call.method.as_str() {
        "role_act" => act(g, who, call),
        "review_next" => review_next(g, who, call),
        _ => review_verdict(g, who, call),
    }
}

fn act<S: Store>(g: &Gateway<S>, who: &Principal, call: Call) -> Vec<ServerFrame> {
    let Ok(p) = serde_json::from_value::<ActParams>(call.value) else {
        return invalid(call.id);
    };
    if !WIRE_ACTS.contains(&p.act.as_str()) {
        return vec![fail_with(
            call.id,
            "unknown_act",
            format!("{} is not an act a role can call over the wire", p.act),
        )];
    }
    match run_act(g, who, &p) {
        Some(Ok(v)) => vec![ok(call.id, v)],
        Some(Err(e)) => failure(call.id, &e),
        None => invalid(call.id),
    }
}

fn audit_json(rs: &[AuditRecord]) -> Value {
    let one = |a: &AuditRecord| json!({"ts": a.ts, "action": a.action, "target": a.target, "detail": a.detail});
    json!({ "records": rs.iter().map(one).collect::<Vec<_>>() })
}

fn refusals_json(rs: &[RefusalRecord]) -> Value {
    let one = |r: &RefusalRecord| {
        json!({"ts": r.ts, "member": r.member, "room": r.room,
               "operation": r.operation, "rule": r.rule})
    };
    json!({ "records": rs.iter().map(one).collect::<Vec<_>>() })
}

/// Run one wire act; `None` when a parameter it needs is missing.
fn run_act<S: Store>(
    g: &Gateway<S>,
    who: &Principal,
    p: &ActParams,
) -> Option<Result<Value, Error>> {
    let (room, member, label) = (p.room.as_deref(), p.member.as_deref(), p.label.as_deref());
    let limit = p.limit.unwrap_or(20).min(200);
    let done = |r: Result<(), Error>| r.map(|()| json!({}));
    Some(match p.act.as_str() {
        "close" => done(g.admin_close_room(who, room?, true)),
        "reopen" => done(g.admin_close_room(who, room?, false)),
        "remove" => done(g.admin_remove_member(who, room?, member?)),
        "allow" => done(g.admin_allow_member(who, room?, member?)),
        "clear" => done(g.admin_set_clearance(who, member?, label?)),
        "label" => done(g.admin_set_room_label(who, room?, label?)),
        "audit" => g.admin_audit(who, limit).map(|rs| audit_json(&rs)),
        _ => g.admin_refusals(who, limit).map(|rs| refusals_json(&rs)),
    })
}

fn item_json(i: &ReviewItem) -> Value {
    json!({"id": i.id, "room": i.room, "seq": i.seq, "to_label": i.to_label,
           "reason": i.reason, "text": i.text})
}

fn review_next<S: Store>(g: &Gateway<S>, who: &Principal, call: Call) -> Vec<ServerFrame> {
    match g.review_next(who) {
        Ok(item) => vec![ok(call.id, json!({ "item": item.as_ref().map(item_json) }))],
        Err(e) => failure(call.id, &e),
    }
}

/// A verdict from its wire words: redact carries its text, and a deferral is not something a reviewer can send.
fn verdict_of(p: &VerdictParams) -> Option<Verdict> {
    match p.verdict.as_str() {
        "allow" => Some(Verdict::Allow),
        "redact" => p.text.clone().map(Verdict::Redact),
        "deny" => Some(Verdict::Deny(p.reason.clone().unwrap_or_default())),
        _ => None,
    }
}

fn review_verdict<S: Store>(g: &Gateway<S>, who: &Principal, call: Call) -> Vec<ServerFrame> {
    let Ok(p) = serde_json::from_value::<VerdictParams>(call.value) else {
        return invalid(call.id);
    };
    let Some(verdict) = verdict_of(&p) else {
        return invalid(call.id);
    };
    match g.review_verdict(who, p.id, &verdict) {
        Ok(()) => vec![ok(call.id, json!({}))],
        Err(e) => failure(call.id, &e),
    }
}
