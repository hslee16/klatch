//! `klatch mcp --stdio` (#299): an MCP server over standard input and output, driven here by a small MCP client that
//! speaks JSON-RPC to the real binary against a real store. Every tool goes through the gateway checks the wire and the
//! command line meet, so the tests look for those checks' answers, not for a re-implementation of them.
mod common;

use common::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next: u64,
}

impl Mcp {
    fn start(h: &Home, who: &str) -> Mcp {
        let mut child = h.spawn(who, &["mcp", "--stdio"]);
        let stdin = child.stdin.take().unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Mcp {
            child,
            stdin: Some(stdin),
            lines,
            next: 0,
        }
    }
    fn send(&mut self, v: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }
    fn line(&self, secs: u64) -> Option<Value> {
        let line = self.lines.recv_timeout(Duration::from_secs(secs)).ok()?;
        Some(
            serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("stdout is not JSON: {line}: {e}")),
        )
    }
    /// A request and its response (every message on stdout must be JSON).
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let v = self.line(30).expect("a response");
        assert_eq!(v["id"], json!(id), "{v}");
        assert_eq!(v["jsonrpc"], "2.0");
        v
    }
    fn init(&mut self) -> Value {
        let r = self.rpc(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
        );
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        r
    }
    fn call(&mut self, tool: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name": tool, "arguments": args}))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn is_error(r: &Value) -> bool {
    r["result"]["isError"] == json!(true)
}

fn text(r: &Value) -> String {
    r["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn started(h: &Home, who: &str) -> Mcp {
    let mut m = Mcp::start(h, who);
    m.init();
    m
}

#[test]
fn initialize_answers_with_the_clients_version_when_known_the_newest_otherwise() {
    let h = Home::new();
    let mut m = Mcp::start(&h, "alice");
    let r = m.init();
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18", "{r}");
    assert_eq!(r["result"]["serverInfo"]["name"], "klatch");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert!(r["result"]["capabilities"]["resources"].is_null());
    let r = m.rpc(
        "initialize",
        json!({"protocolVersion": "1999-01-01", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
    );
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18", "{r}");
}

#[test]
fn initialize_carries_the_room_convention_as_instructions() {
    let h = Home::new();
    let r = Mcp::start(&h, "alice").init();
    let text = r["result"]["instructions"].as_str().unwrap_or("");
    assert!(text.contains("lobby") && text.contains("claim"), "{text}");
}

#[test]
fn tools_list_names_the_nine_tools_with_object_schemas() {
    let h = Home::new();
    let mut m = started(&h, "alice");
    let r = m.rpc("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().expect("tools");
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "klatch_asks",
            "klatch_claim",
            "klatch_invite",
            "klatch_join",
            "klatch_post",
            "klatch_read",
            "klatch_release",
            "klatch_wait",
            "klatch_who"
        ]
    );
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(!t["description"].as_str().unwrap_or("").is_empty());
    }
}

#[test]
fn a_post_is_read_by_another_member_as_data_with_the_senders_name() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    let r = a.call(
        "klatch_post",
        json!({"room": "lobby", "body": "hello over mcp"}),
    );
    assert!(!is_error(&r), "{r}");
    assert_eq!(r["result"]["structuredContent"]["seq"], 1);
    let r = b.call("klatch_read", json!({"room": "lobby"}));
    assert!(!is_error(&r), "{r}");
    let m = &r["result"]["structuredContent"]["messages"][0];
    assert_eq!(
        (m["sender"].as_str(), m["body"].as_str()),
        (Some("alice"), Some("hello over mcp"))
    );
    assert!(
        r["result"]["structuredContent"]["notice"]
            .as_str()
            .unwrap_or("")
            .contains("data"),
        "{r}"
    );
    let again = b.call("klatch_read", json!({"room": "lobby"}));
    assert_eq!(
        again["result"]["structuredContent"]["messages"],
        json!([]),
        "read advances the cursor"
    );
}

#[test]
fn peek_looks_without_moving_the_cursor() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    a.call("klatch_post", json!({"room": "lobby", "body": "x"}));
    let r = b.call("klatch_read", json!({"room": "lobby", "peek": true}));
    assert_eq!(
        r["result"]["structuredContent"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let r = b.call("klatch_read", json!({"room": "lobby"}));
    assert_eq!(
        r["result"]["structuredContent"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_claim_conflict_is_a_tool_error_naming_the_holder_and_a_release_frees_the_path() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    let r = a.call(
        "klatch_claim",
        json!({"room": "lobby", "path": "src/a.rs", "reason": "work"}),
    );
    assert!(!is_error(&r), "{r}");
    let r = b.call("klatch_claim", json!({"room": "lobby", "path": "src/a.rs"}));
    assert!(is_error(&r), "{r}");
    assert!(
        text(&r).contains("conflict") && text(&r).contains("alice"),
        "{}",
        text(&r)
    );
    let r = a.call(
        "klatch_release",
        json!({"room": "lobby", "path": "src/a.rs", "note": "done"}),
    );
    assert!(!is_error(&r), "{r}");
    let r = b.call("klatch_claim", json!({"room": "lobby", "path": "src/a.rs"}));
    assert!(!is_error(&r), "{r}");
    let r = a.call(
        "klatch_release",
        json!({"room": "lobby", "path": "never/claimed"}),
    );
    assert!(is_error(&r) && text(&r).contains("not_found"), "{r}");
}

#[test]
fn a_refused_post_is_a_tool_error_that_names_the_rule_and_never_repeats_the_text() {
    let h = Home::new();
    let mut a = started(&h, "alice");
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let r = a.call(
        "klatch_post",
        json!({"room": "lobby", "body": format!("token {secret}")}),
    );
    assert!(is_error(&r), "{r}");
    assert!(
        text(&r).contains("refused") && text(&r).contains("credentials"),
        "{}",
        text(&r)
    );
    assert!(
        !r.to_string().contains(secret),
        "the refusal repeated the secret"
    );
}

#[test]
fn wait_returns_the_messages_that_arrive_and_the_timeout_marker_when_none_do() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    a.call("klatch_post", json!({"room": "lobby", "body": "first"}));
    let r = b.call("klatch_wait", json!({"rooms": ["lobby"], "timeout": 5}));
    let s = &r["result"]["structuredContent"];
    assert_eq!(s["timeout"], false, "{r}");
    assert_eq!(
        (
            s["messages"][0]["room"].as_str(),
            s["messages"][0]["body"].as_str()
        ),
        (Some("lobby"), Some("first"))
    );
    let started_at = std::time::Instant::now();
    let r = b.call("klatch_wait", json!({"rooms": ["lobby"], "timeout": 1}));
    let s = &r["result"]["structuredContent"];
    assert_eq!(
        (
            s["timeout"].as_bool(),
            s["messages"].as_array().map(Vec::len)
        ),
        (Some(true), Some(0)),
        "{r}"
    );
    assert!(
        started_at.elapsed() < Duration::from_secs(10),
        "the wait overran its bound"
    );
}

#[test]
fn a_wait_is_woken_by_a_post_made_while_it_waits() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    b.next += 1;
    let id = b.next;
    b.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                   "params": {"name": "klatch_wait", "arguments": {"rooms": ["lobby"], "timeout": 20}}}));
    std::thread::sleep(Duration::from_millis(500));
    a.call(
        "klatch_post",
        json!({"room": "lobby", "body": "while you wait"}),
    );
    let r = b.line(15).expect("the wait to return");
    assert_eq!(
        r["result"]["structuredContent"]["messages"][0]["body"], "while you wait",
        "{r}"
    );
}

#[test]
fn a_wait_outside_its_bounds_is_an_invalid_params_error() {
    let h = Home::new();
    let mut m = started(&h, "alice");
    for timeout in [0, 56, -1] {
        let r = m.call(
            "klatch_wait",
            json!({"rooms": ["lobby"], "timeout": timeout}),
        );
        assert_eq!(r["error"]["code"], -32602, "{r}");
    }
    let r = m.call("klatch_wait", json!({"rooms": [], "timeout": 1}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

#[test]
fn who_lists_the_members_of_a_room() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    a.call("klatch_post", json!({"room": "lobby", "body": "hi"}));
    b.call("klatch_post", json!({"room": "lobby", "body": "hi"}));
    let r = a.call("klatch_who", json!({"room": "lobby"}));
    let names: Vec<String> = r["result"]["structuredContent"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"alice".to_string()) && names.contains(&"bob".to_string()),
        "{names:?}"
    );
}

#[test]
fn a_non_coding_agent_over_mcp_answers_a_coding_members_ask_in_the_lobby() {
    let h = Home::new();
    h.ok("alice", &["join", "lobby"]);
    h.ok(
        "alice",
        &["post", "lobby", "seeking: a reviewer for the parser"],
    );
    let mut bot = started(&h, "bot-grok");
    let r = bot.call("klatch_wait", json!({"rooms": ["lobby"], "timeout": 5}));
    assert_eq!(
        r["result"]["structuredContent"]["messages"][0]["sender"], "alice",
        "{r}"
    );
    let r = bot.call("klatch_claim", json!({"room": "lobby", "path": "ask:1"}));
    assert!(!is_error(&r), "{r}");
    let r = bot.call(
        "klatch_post",
        json!({"room": "lobby", "body": "answer to #1: I will review it"}),
    );
    assert!(!is_error(&r), "{r}");
    let out = h.ok("alice", &["read", "lobby"]);
    assert!(
        out.contains("bot-grok") && out.contains("I will review it"),
        "{out}"
    );
    assert!(
        h.ok("alice", &["who", "lobby"]).contains("bot-grok"),
        "the bot is a member like any other"
    );
}

#[test]
fn a_member_argument_has_no_effect_the_process_is_the_member() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    let r = a.call(
        "klatch_post",
        json!({"room": "lobby", "body": "x", "member": "mallory", "sender": "mallory"}),
    );
    assert!(!is_error(&r), "{r}");
    let r = b.call("klatch_read", json!({"room": "lobby"}));
    assert_eq!(
        r["result"]["structuredContent"]["messages"][0]["sender"],
        "alice"
    );
}

#[test]
fn a_malformed_call_is_a_json_rpc_error_and_the_session_carries_on() {
    let h = Home::new();
    let mut m = started(&h, "alice");
    let r = m.call("klatch_nothing", json!({}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = m.call("klatch_post", json!({"room": "lobby"}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = m.rpc("no/such/method", json!({}));
    assert_eq!(r["error"]["code"], -32601, "{r}");
    let r = m.call("klatch_post", json!({"room": "bad room", "body": "x"}));
    assert!(is_error(&r) && text(&r).contains("invalid"), "{r}");
    let r = m.call(
        "klatch_post",
        json!({"room": "lobby", "body": "still working"}),
    );
    assert!(!is_error(&r), "{r}");
}

#[test]
fn a_notification_gets_no_answer_and_ping_is_answered() {
    let h = Home::new();
    let mut m = started(&h, "alice");
    m.send(&json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {}}));
    let r = m.rpc("ping", json!({}));
    assert_eq!(r["result"], json!({}), "{r}");
}

#[test]
fn closing_standard_input_ends_the_server_cleanly() {
    let h = Home::new();
    let mut m = started(&h, "alice");
    m.call("klatch_post", json!({"room": "lobby", "body": "x"}));
    drop(m.stdin.take());
    let status = m.child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn join_asks_and_invite_let_an_mcp_only_member_route() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    a.call("klatch_post", json!({"room": "lobby", "body": "hi"}));
    b.call("klatch_post", json!({"room": "lobby", "body": "hi"}));
    a.call(
        "klatch_post",
        json!({"room": "lobby", "body": "seeking: a parser review"}),
    );
    let r = b.call("klatch_asks", json!({"room": "lobby"}));
    assert!(!is_error(&r), "{r}");
    let asks = &r["result"]["structuredContent"]["asks"];
    assert_eq!(asks[0]["sender"], "alice", "{r}");
    assert!(
        r["result"]["structuredContent"]["notice"]
            .as_str()
            .unwrap_or("")
            .contains("data"),
        "{r}"
    );
    let r = b.call("klatch_join", json!({"room": "ask-2"}));
    assert!(!is_error(&r), "{r}");
    let r = b.call(
        "klatch_invite",
        json!({"member": "alice", "room": "ask-2", "via": "lobby"}),
    );
    assert!(!is_error(&r), "{r}");
    let r = a.call("klatch_read", json!({"room": "lobby"}));
    assert!(
        r["result"]["structuredContent"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"]
                .as_str()
                .unwrap_or("")
                .contains("klatch join ask-2")),
        "{r}"
    );
    let r = a.call(
        "klatch_invite",
        json!({"member": "alice", "room": "ask-2", "via": "lobby"}),
    );
    assert!(is_error(&r) && text(&r).contains("invalid"), "{r}");
    let r = a.call("klatch_join", json!({}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

/// 90 posts of about 400 bytes from alice in lobby (under the 100 a minute a member may post); bob has joined first.
fn flooded(h: &Home) -> (Mcp, Mcp) {
    let (mut a, mut b) = (started(h, "alice"), started(h, "bob"));
    b.call("klatch_join", json!({"room": "lobby"}));
    for i in 0..90 {
        let body = format!("flood {i:03} {}", "z".repeat(390));
        let r = a.call("klatch_post", json!({"room": "lobby", "body": body}));
        assert!(!is_error(&r), "{r}");
    }
    (a, b)
}

fn flood_seqs(r: &Value) -> Vec<u64> {
    r["result"]["structuredContent"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["seq"].as_u64().unwrap())
        .collect()
}

#[test]
fn a_read_and_a_wait_result_are_bounded_say_what_was_left_and_continue_in_order() {
    let h = Home::new();
    let (_a, mut b) = flooded(&h);
    let r = b.call("klatch_read", json!({"room": "lobby"}));
    assert!(
        r.to_string().len() <= 40_000,
        "{} bytes",
        r.to_string().len()
    );
    let s = &r["result"]["structuredContent"];
    let first = flood_seqs(&r);
    assert!(!first.is_empty() && first.len() <= 40, "{}", first.len());
    assert_eq!(s["omitted"], json!(90 - first.len()), "{s}");
    assert!(s["notice"].as_str().unwrap().contains("data"), "{s}");
    assert!(s["next"].as_str().unwrap().contains("klatch_read"), "{s}");
    let w = b.call("klatch_wait", json!({"rooms": ["lobby"], "timeout": 5}));
    assert!(
        w.to_string().len() <= 40_000,
        "{} bytes",
        w.to_string().len()
    );
    let second = flood_seqs(&w);
    assert!(!second.is_empty() && second.len() <= 20, "{}", second.len());
    assert_eq!(
        second[0],
        first[first.len() - 1] + 1,
        "wait continues where read stopped"
    );
    assert!(
        w["result"]["structuredContent"]["omitted"]
            .as_u64()
            .unwrap()
            > 0,
        "{w}"
    );
    let rest = flood_seqs(&b.call("klatch_read", json!({"room": "lobby"})));
    assert_eq!(rest[0], second[second.len() - 1] + 1);
}

#[test]
fn one_huge_message_in_a_read_result_is_cut_with_a_marker() {
    let h = Home::new();
    let (mut a, mut b) = (started(&h, "alice"), started(&h, "bob"));
    b.call("klatch_join", json!({"room": "lobby"}));
    a.call(
        "klatch_post",
        json!({"room": "lobby", "body": "y".repeat(30_000)}),
    );
    a.call("klatch_post", json!({"room": "lobby", "body": "after"}));
    let r = b.call("klatch_read", json!({"room": "lobby"}));
    assert!(
        r.to_string().len() <= 40_000,
        "{} bytes",
        r.to_string().len()
    );
    let m = &r["result"]["structuredContent"]["messages"][0];
    assert!(m["body"].as_str().unwrap().contains("[truncated"), "{m}");
    assert_eq!(m["truncated"], json!(true));
    let again = b.call("klatch_read", json!({"room": "lobby"}));
    assert_eq!(
        again["result"]["structuredContent"]["messages"][0]["body"],
        "after"
    );
}
