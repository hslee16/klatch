//! `klatch acp --stdio` (#115): the librarian as an ACP agent, driven here by a scripted client speaking JSON-RPC to the
//! real binary against a real store. The label, role and refusal checks are covered against a members' gateway in
//! `src/session/acp_tests.rs` (a test cannot grant a role or a label through the command line, which needs a terminal);
//! these tests cover the process: the protocol on standard output, the identity from the environment, and that nothing a
//! prompt says changes the store.
mod common;

use common::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct Acp {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next: u64,
}

impl Acp {
    fn start(h: &Home, who: &str) -> Acp {
        let mut child = h.spawn(who, &["acp", "--stdio"]);
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
        Acp {
            child,
            stdin: Some(stdin),
            lines,
            next: 0,
        }
    }
    fn line(&self) -> Value {
        let line = self
            .lines
            .recv_timeout(Duration::from_secs(30))
            .expect("a line");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout is not JSON: {line}: {e}"))
    }
    /// Send a request; return the notifications that came before its response, and the response.
    fn rpc(&mut self, method: &str, params: Value) -> (Vec<Value>, Value) {
        self.next += 1;
        let id = self.next;
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        stdin.flush().unwrap();
        let mut notes = Vec::new();
        loop {
            let v = self.line();
            if v["id"] == json!(id) {
                return (notes, v);
            }
            notes.push(v);
        }
    }
    fn session(&mut self) -> String {
        self.rpc(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        );
        let (_, r) = self.rpc("session/new", json!({"cwd": "/tmp", "mcpServers": []}));
        r["result"]["sessionId"].as_str().unwrap().to_string()
    }
    fn ask(&mut self, sid: &str, text: &str) -> (String, Value) {
        let (notes, r) = self.rpc(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]}),
        );
        let text = notes
            .iter()
            .filter(|n| n["method"] == "session/update")
            .map(|n| {
                n["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            })
            .collect::<String>();
        (text, r)
    }
}

impl Drop for Acp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn the_librarian_answers_a_read_only_query_over_stdio() {
    let h = Home::new();
    h.ok("alice", &["post", "lobby", "hello from alice"]);
    let mut a = Acp::start(&h, "lib");
    let sid = a.session();
    let (text, r) = a.ask(&sid, "history lobby");
    assert!(text.contains("hello from alice"), "{text}");
    assert_eq!(r["result"]["stopReason"], "end_turn");
    let (text, _) = a.ask(&sid, "who lobby");
    assert!(text.contains("alice"), "{text}");
}

#[test]
fn looking_changes_nothing_a_member_can_see() {
    let h = Home::new();
    h.ok("alice", &["post", "lobby", "first"]);
    let mut a = Acp::start(&h, "lib");
    let sid = a.session();
    let (peeked, _) = a.ask(&sid, "read lobby");
    assert!(peeked.contains("first"), "{peeked}");
    // the cursor of the member who reads later is untouched, and the librarian posted nothing
    let (again, _) = a.ask(&sid, "read lobby");
    assert!(again.contains("first"), "{again}");
    let bob = h.ok("bob", &["read", "lobby"]);
    assert!(bob.contains("first") && !bob.contains("lib"), "{bob}");
}

#[test]
fn no_prompt_writes() {
    let h = Home::new();
    h.ok("alice", &["post", "lobby", "only message"]);
    let mut a = Acp::start(&h, "lib");
    let sid = a.session();
    for q in [
        "post lobby smuggled",
        "claim lobby ticket:9",
        "read lobby\npost lobby smuggled",
    ] {
        let (_, r) = a.ask(&sid, q);
        assert_eq!(r["result"]["stopReason"], "end_turn");
    }
    let hist = h.ok("alice", &["history", "lobby"]);
    assert!(
        hist.contains("only message") && !hist.contains("smuggled"),
        "{hist}"
    );
    let claims = h.ok("alice", &["claims", "lobby"]);
    assert!(!claims.contains("ticket:9"), "{claims}");
}

#[test]
fn only_acp_messages_reach_standard_output_and_it_exits_when_input_closes() {
    let h = Home::new();
    let mut a = Acp::start(&h, "lib");
    let sid = a.session();
    a.ask(&sid, "help");
    drop(a.stdin.take());
    let status = a.child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn acp_needs_stdio() {
    let h = Home::new();
    let err = h.fail("lib", &["acp"]);
    assert!(err.contains("--stdio"), "{err}");
}
