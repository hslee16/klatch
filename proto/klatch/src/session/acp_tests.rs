//! The librarian as an ACP agent (#115), driven by a scripted client over in-memory streams against a members' gateway.
//! The label, role and refusal checks are the gateway's; these tests look for their answers, and for the absence of
//! any way to write.

use super::acp::AcpServer;
use super::{Session, SessionConfig};
use crate::domain::Principal;
use crate::gateway::{AllowAll, FixedClock, ForbidFloors, Gateway};
use crate::store::SqliteStore;
use serde_json::{Value, json};
use std::io::Cursor;

fn who(name: &str) -> Principal {
    Principal::unattributed(name)
}

/// Room `r` (public) with one message from alice, room `vault` (private) with one, and `lib` the librarian at the lowest
/// clearance.
pub(super) fn gateway() -> Gateway<SqliteStore> {
    gateway_cleared("public")
}

fn gateway_cleared(lib_clearance: &str) -> Gateway<SqliteStore> {
    let g = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1_000),
        ForbidFloors::new(AllowAll),
    );
    let human = who("admin");
    g.join(&who("alice"), "r").unwrap();
    g.post(&who("alice"), "r", "hello from alice").unwrap();
    g.admin_set_room_label(&human, "vault", "private").unwrap();
    g.admin_set_clearance(&human, "alice", "private").unwrap();
    g.join(&who("alice"), "vault").unwrap();
    g.post(&who("alice"), "vault", "the private plan").unwrap();
    g.admin_set_librarian(&human, "lib", true).unwrap();
    g.admin_set_clearance(&human, "lib", lib_clearance).unwrap();
    g.for_members()
}

/// A scripted client: send each request line, return every message the agent printed.
fn drive(g: &Gateway<SqliteStore>, member: &str, lines: &[Value]) -> Vec<Value> {
    let p = who(member);
    let config = SessionConfig::default();
    let session = Session::new(g, &p, &config);
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut out = Vec::new();
    AcpServer::new(&session)
        .serve(Cursor::new(input), &mut out)
        .unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub(super) fn init() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}})
}
pub(super) fn new_session() -> Value {
    json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}})
}
pub(super) fn prompt(id: u64, text: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"session/prompt","params":{
        "sessionId":"klatch-librarian-1","prompt":[{"type":"text","text":text}]}})
}

/// The answer text and stop reason of one prompt.
fn ask(g: &Gateway<SqliteStore>, text: &str) -> (String, Value) {
    let out = drive(g, "lib", &[init(), new_session(), prompt(3, text)]);
    let update = out
        .iter()
        .find(|m| m["method"] == "session/update")
        .expect("an update");
    assert_eq!(
        update["params"]["update"]["sessionUpdate"],
        "agent_message_chunk"
    );
    let reply = out.iter().find(|m| m["id"] == 3).expect("the response");
    (
        update["params"]["update"]["content"]["text"]
            .as_str()
            .unwrap()
            .to_string(),
        reply["result"]["stopReason"].clone(),
    )
}

#[test]
fn initialize_answers_with_version_one_and_no_auth_and_no_load() {
    let out = drive(&gateway(), "lib", &[init()]);
    let r = &out[0]["result"];
    assert_eq!(r["protocolVersion"], 1, "{out:?}");
    assert_eq!(r["agentCapabilities"]["loadSession"], false);
    assert_eq!(r["authMethods"], json!([]));
    assert_eq!(r["agentInfo"]["name"], "klatch-librarian");
}

#[test]
fn a_session_is_made_and_a_prompt_to_an_unknown_one_is_invalid_params() {
    let out = drive(&gateway(), "lib", &[new_session(), prompt(3, "help")]);
    assert_eq!(out[0]["result"]["sessionId"], "klatch-librarian-1");
    let bad = json!({"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"nope","prompt":[]}});
    let out = drive(&gateway(), "lib", &[bad]);
    assert_eq!(out[0]["error"]["code"], -32602, "{out:?}");
}

#[test]
fn the_librarian_reads_a_room_and_the_answer_streams_before_the_stop_reason() {
    let out = drive(&gateway(), "lib", &[new_session(), prompt(3, "read r")]);
    let update_at = out
        .iter()
        .position(|m| m["method"] == "session/update")
        .unwrap();
    let reply_at = out.iter().position(|m| m["id"] == 3).unwrap();
    assert!(update_at < reply_at, "{out:?}");
    let text = out[update_at]["params"]["update"]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("hello from alice") && text.contains("data"),
        "{text}"
    );
    assert_eq!(out[reply_at]["result"]["stopReason"], "end_turn");
}

#[test]
fn reading_is_a_peek_so_asking_twice_gives_the_same_answer() {
    let g = gateway();
    let (a, _) = ask(&g, "read r");
    let (b, _) = ask(&g, "read r");
    assert!(a.contains("hello from alice") && a == b, "{a} / {b}");
}

#[test]
fn history_who_and_claims_answer() {
    let g = gateway();
    assert!(ask(&g, "history r 5").0.contains("hello from alice"));
    assert!(ask(&g, "who r").0.contains("alice"));
    assert!(ask(&g, "claims r").0.contains("claims"));
}

#[test]
fn a_query_above_the_callers_clearance_is_refused_and_leaks_nothing() {
    let g = gateway();
    for q in ["read vault", "history vault", "who vault", "claims vault"] {
        let (text, stop) = ask(&g, q);
        assert!(!text.contains("private plan"), "{q}: {text}");
        assert!(
            text.contains("refused") || text.contains("failed"),
            "{q}: {text}"
        );
        assert_eq!(stop, "end_turn");
    }
}

#[test]
fn a_cleared_librarian_reads_the_room() {
    let g = gateway_cleared("private");
    assert!(ask(&g, "read vault").0.contains("the private plan"));
}

#[test]
fn no_write_is_reachable_from_any_prompt() {
    let g = gateway();
    let before = g.read(&who("alice"), "r", false).unwrap().len();
    for q in [
        "post r a message",
        "claim r ticket:1",
        "release r ticket:1",
        "dm alice hi",
        "role_act close r",
        "admin close r",
        "read r\npost r smuggled",
        "read r; post r smuggled",
        "history r 1 2",
    ] {
        let (text, stop) = ask(&g, q);
        assert_eq!(stop, "end_turn");
        assert!(!text.contains("smuggled\""), "{q}: {text}");
    }
    let alice = who("alice");
    assert!(
        !g.read(&alice, "r", false)
            .unwrap()
            .iter()
            .any(|m| m.body.contains("smuggled"))
    );
    assert!(g.claims(&who("alice"), "r").unwrap().is_empty());
    assert_eq!(g.read(&who("alice"), "r", false).unwrap().len(), before);
}

#[test]
fn a_prompt_cannot_name_another_caller() {
    let g = gateway();
    let p = json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
        "sessionId":"klatch-librarian-1","member":"alice","prompt":[{"type":"text","text":"read vault"}]}});
    let out = drive(&g, "lib", &[new_session(), p]);
    let all = serde_json::to_string(&out).unwrap();
    assert!(!all.contains("private plan"), "{all}");
}

#[test]
fn an_ordinary_member_cannot_use_it_to_do_what_a_member_may_not_see() {
    let g = gateway();
    let out = drive(&g, "bob", &[new_session(), prompt(3, "read vault")]);
    let all = serde_json::to_string(&out).unwrap();
    assert!(
        !all.contains("private plan") && all.contains("refused"),
        "{all}"
    );
}

#[test]
fn unknown_methods_notifications_and_junk_are_handled() {
    let g = gateway();
    let cancel = json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"x"}});
    let load = json!({"jsonrpc":"2.0","id":9,"method":"session/load","params":{}});
    let out = drive(&g, "lib", &[cancel, load]);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0]["error"]["code"], -32601);
    let p = who("lib");
    let config = SessionConfig::default();
    let session = Session::new(&g, &p, &config);
    let mut out = Vec::new();
    AcpServer::new(&session)
        .serve(Cursor::new("not json\n"), &mut out)
        .unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["code"], -32700);
}

/// Spec check (#869): `authenticate` is a baseline agent method and its `methodId` must be one the agent advertised. This
/// agent advertises none, so any id is invalid params (not "method not found", which says the method is missing).
#[test]
fn authenticate_with_no_advertised_method_is_invalid_params_not_method_not_found() {
    let auth =
        json!({"jsonrpc":"2.0","id":5,"method":"authenticate","params":{"methodId":"oauth"}});
    let out = drive(&gateway(), "lib", &[auth]);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0]["error"]["code"], -32602, "{out:?}");
    assert_eq!(out[0]["id"], 5);
}

/// Spec check (#869): stdio messages must not contain embedded newlines, and nothing but messages goes to stdout. The
/// answer has several lines, so it must reach the client as JSON escapes on one physical line per message.
#[test]
fn every_message_is_one_physical_line_even_for_a_multiline_answer() {
    let g = gateway();
    let p = who("lib");
    let config = SessionConfig::default();
    let session = Session::new(&g, &p, &config);
    let input = format!("{}\n{}\n", new_session(), prompt(3, "history r 5"));
    let mut out = Vec::new();
    AcpServer::new(&session)
        .serve(Cursor::new(input), &mut out)
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.ends_with('\n'));
    assert_eq!(text.lines().count(), 3, "{text}");
    for line in text.lines() {
        serde_json::from_str::<Value>(line).unwrap();
    }
}

/// Spec check (#869): `session/cancel` is a notification (no reply, even an error) and does not end the session; a later
/// prompt on the same session is still answered with `end_turn`.
#[test]
fn a_cancel_notification_gets_no_reply_and_the_session_survives() {
    let cancel = json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"klatch-librarian-1"}});
    let out = drive(
        &gateway(),
        "lib",
        &[new_session(), cancel, prompt(3, "help")],
    );
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(out[2]["result"]["stopReason"], "end_turn");
}

/// A prompt made of the given content blocks; the answer text and stop reason.
fn ask_blocks(g: &Gateway<SqliteStore>, blocks: Value) -> (String, Value) {
    let msg = json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
        "sessionId":"klatch-librarian-1","prompt":blocks}});
    let out = drive(g, "lib", &[init(), new_session(), msg]);
    let update = out
        .iter()
        .find(|m| m["method"] == "session/update")
        .expect("an update");
    let reply = out.iter().find(|m| m["id"] == 3).expect("the response");
    (
        update["params"]["update"]["content"]["text"]
            .as_str()
            .unwrap()
            .to_string(),
        reply["result"]["stopReason"].clone(),
    )
}

#[test]
fn a_resource_link_is_read_as_its_name_and_uri_text() {
    // The link text is "read r": the same look as the typed words (#1042).
    let link = json!([{"type":"resource_link","name":"read","uri":"r"}]);
    let (text, stop) = ask_blocks(&gateway(), link);
    assert!(text.contains("hello from alice"), "{text}");
    assert_eq!(stop, "end_turn");
}

#[test]
fn a_resource_link_after_text_is_joined_in_order() {
    let blocks = json!([
        {"type":"text","text":"read"},
        {"type":"resource_link","name":"r","uri":"ignored"}
    ]);
    let (text, _) = ask_blocks(&gateway(), blocks);
    // "read" then "r ignored" on the next line: only the first line is the query.
    assert!(text.contains("needs a room name"), "{text}");
}

#[test]
fn link_text_is_data_control_characters_are_stripped_and_length_is_bounded() {
    let link = json!([{"type":"resource_link","name":"re\u{0}ad\u{7}","uri":"r\u{1b}"}]);
    let (text, _) = ask_blocks(&gateway(), link);
    assert!(text.contains("hello from alice"), "{text}");
    let long = "a".repeat(100_000);
    let link = json!([{"type":"resource_link","name":long,"uri":"b".repeat(100_000)}]);
    let (text, stop) = ask_blocks(&gateway(), link);
    assert_eq!(stop, "end_turn");
    assert!(
        text.len() < 4_000,
        "answer echoed unbounded link text: {}",
        text.len()
    );
    // A newline in a name cannot start a second query line.
    let link = json!([{"type":"resource_link","name":"help\nread","uri":"r"}]);
    let (text, _) = ask_blocks(&gateway(), link);
    assert!(!text.contains("hello from alice"), "{text}");
}

#[test]
fn a_link_without_a_name_or_uri_is_skipped_and_the_prompt_still_answers() {
    let (text, stop) = ask_blocks(&gateway(), json!([{"type":"resource_link"}]));
    assert!(text.contains("librarian"), "{text}");
    assert_eq!(stop, "end_turn");
}

fn hostile() -> String {
    format!(
        "\u{1b}[31m\u{202e}\u{e0041}\n{}\u{1b}]0;x\u{7}\u{2066}\r",
        "n".repeat(100 * 1024)
    )
}

fn assert_bounded(text: &str, site: &str) {
    assert!(
        text.is_ascii() && !text.chars().any(char::is_control),
        "{site}: {text:?}"
    );
    assert!(text.len() < 400, "{site}: {} bytes", text.len());
}

#[test]
fn an_unknown_acp_method_is_named_capped_and_ascii() {
    let h = hostile();
    let call = json!({"jsonrpc":"2.0","id":1,"method":h,"params":{}});
    let out = drive(
        &gateway(),
        "lib",
        &[call, json!({"jsonrpc":"2.0","id":2,"method":"tele port"})],
    );
    assert_bounded(
        out[0]["error"]["message"].as_str().unwrap(),
        "acp no method",
    );
    assert_eq!(out[1]["error"]["message"], "no method tele port");
}

fn mcp(lines: &[Value]) -> Vec<Value> {
    let g = gateway();
    let p = who("lib");
    let config = SessionConfig::default();
    let session = Session::new(&g, &p, &config);
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let mut out = Vec::new();
    super::mcp::McpServer::new(&session, String::new())
        .serve(Cursor::new(input), &mut out)
        .unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn the_mcp_errors_that_repeat_the_caller_are_named_capped_and_ascii() {
    let h = hostile();
    let call = |id: u64, method: &str, params: Value| json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
    let out = mcp(&[
        call(1, &h, json!({})),
        call(2, "tools/call", json!({"name": h, "arguments": {}})),
        call(
            3,
            "tools/call",
            json!({"name":"klatch_read","arguments":{"room":"r","peek":h}}),
        ),
        call(4, "tele port", json!({})),
        call(5, "tools/call", json!({"name":"nope","arguments":{}})),
    ]);
    for (i, site) in ["mcp no method", "mcp no tool", "mcp bad arguments"]
        .iter()
        .enumerate()
    {
        assert_bounded(out[i]["error"]["message"].as_str().unwrap(), site);
    }
    assert_eq!(out[3]["error"]["message"], "no method tele port");
    assert_eq!(out[4]["error"]["message"], "no tool nope");
}
