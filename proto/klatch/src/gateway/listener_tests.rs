//! Tests of the transport seam: a fake transport (no sockets) hands the real session loop a byte stream and
//! `PeerEvidence`, and an `EvidenceTable` identity decides who the peer is (DESIGN section 20 rule 11, R23).

use super::listener::{Connection, Connector, EvidenceTable, Listener};
use crate::domain::Principal;
use crate::gateway::{Identity, PeerEvidence};
use crate::protocol::{Attribution as WireAttribution, ServerFrame, decode_server_line};
use crate::session::{SessionConfig, serve_connection};
use crate::store::SqliteStore;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

/// A transport with no sockets: peers are queued with their evidence and input, and each connection's
/// output is captured so a test can read what the session wrote.
#[derive(Default)]
struct FakeTransport {
    pending: VecDeque<Connection>,
}

type Capture = Arc<Mutex<Vec<u8>>>;

struct CaptureWriter(Capture);

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl FakeTransport {
    fn peer(&mut self, evidence: PeerEvidence, lines: &[Value]) -> Capture {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let capture = Capture::default();
        self.pending.push_back(Connection {
            input: Box::new(Cursor::new(input)),
            output: Box::new(CaptureWriter(capture.clone())),
            evidence,
            closer: None,
        });
        capture
    }
}

impl Listener for FakeTransport {
    fn accept(&mut self) -> std::io::Result<Option<Connection>> {
        Ok(self.pending.pop_front())
    }
}

impl Connector for FakeTransport {
    fn connect(&mut self) -> std::io::Result<Connection> {
        self.pending
            .pop_front()
            .ok_or_else(|| std::io::Error::other("no peer queued"))
    }
}

fn gateway() -> crate::gateway::Gateway<SqliteStore> {
    crate::gateway::Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        crate::gateway::FixedClock(1_000),
        crate::gateway::AllowAll,
    )
}

fn req(id: &str, method: &str, params: Value) -> Value {
    json!({"v":1,"type":"request","id":id,"method":method,"params":params})
}

fn frames(capture: &Capture) -> Vec<ServerFrame> {
    String::from_utf8(capture.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap())
        .collect()
}

fn hello(frames: &[ServerFrame]) -> (String, WireAttribution) {
    match &frames[0] {
        ServerFrame::Hello {
            member,
            attribution,
            ..
        } => (member.clone(), attribution.clone()),
        other => panic!("first frame is not hello: {other:?}"),
    }
}

fn table() -> EvidenceTable {
    EvidenceTable::new()
        .allow(
            PeerEvidence::SshKey {
                fingerprint: "SHA256:alicekey".into(),
                forced_member: Some("alice".into()),
            },
            "alice",
        )
        .allow(
            PeerEvidence::Certificate {
                subject: "CN=carol".into(),
            },
            "carol",
        )
        .allow(PeerEvidence::bearer("tok-dave"), "dave")
        .allow(PeerEvidence::Process { pid: 77, uid: 501 }, "eve")
}

/// Accept the one queued peer and run the real session loop for it.
fn serve_next(t: &mut FakeTransport, g: &crate::gateway::Gateway<SqliteStore>) {
    let conn = t.accept().unwrap().expect("a queued peer");
    serve_connection(g, &table(), conn, &SessionConfig::default()).unwrap();
}

fn ssh_alice() -> PeerEvidence {
    PeerEvidence::SshKey {
        fingerprint: "SHA256:alicekey".into(),
        forced_member: Some("alice".into()),
    }
}

#[test]
fn a_peer_with_a_mapped_ssh_key_acts_as_its_member_and_is_verified() {
    let g = gateway();
    let mut t = FakeTransport::default();
    let out = t.peer(
        ssh_alice(),
        &[req("p", "post", json!({"room":"r","body":"hello"}))],
    );
    serve_next(&mut t, &g);
    let f = frames(&out);
    assert_eq!(hello(&f), ("alice".into(), WireAttribution::Verified));
    let posted = g
        .read(&Principal::unattributed("watcher"), "r", false)
        .unwrap();
    assert_eq!(posted[0].sender, "alice");
    assert_eq!(posted[0].attribution, crate::domain::Attribution::Verified);
}

#[test]
fn certificate_and_bearer_evidence_map_to_their_members() {
    let g = gateway();
    let mut t = FakeTransport::default();
    let cert = t.peer(
        PeerEvidence::Certificate {
            subject: "CN=carol".into(),
        },
        &[],
    );
    let bearer = t.peer(PeerEvidence::bearer("tok-dave"), &[]);
    serve_next(&mut t, &g);
    serve_next(&mut t, &g);
    assert_eq!(
        hello(&frames(&cert)),
        ("carol".into(), WireAttribution::Verified)
    );
    assert_eq!(
        hello(&frames(&bearer)),
        ("dave".into(), WireAttribution::Verified)
    );
}

#[test]
fn a_peer_found_only_by_pid_and_uid_is_mapped_but_never_verified() {
    let g = gateway();
    let mut t = FakeTransport::default();
    let out = t.peer(PeerEvidence::Process { pid: 77, uid: 501 }, &[]);
    serve_next(&mut t, &g);
    assert_eq!(
        hello(&frames(&out)),
        ("eve".into(), WireAttribution::Unattributed)
    );
}

#[test]
fn an_unmapped_peer_is_unattributed_and_its_messages_are_marked_unverified() {
    let g = gateway();
    let mut t = FakeTransport::default();
    let out = t.peer(
        PeerEvidence::bearer("unknown-token"),
        &[req("p", "post", json!({"room":"r","body":"hi"}))],
    );
    serve_next(&mut t, &g);
    let (member, attribution) = hello(&frames(&out));
    assert_eq!(attribution, WireAttribution::Unattributed);
    let posted = g
        .read(&Principal::unattributed("watcher"), "r", false)
        .unwrap();
    assert_eq!(posted[0].sender, member);
    assert!(posted[0].sender_label().ends_with("(unverified)"));
}

#[test]
fn a_peer_with_no_evidence_is_unattributed_and_its_messages_are_marked_unverified() {
    let g = gateway();
    let mut t = FakeTransport::default();
    let out = t.peer(
        PeerEvidence::None,
        &[req("p", "post", json!({"room":"r","body":"hi"}))],
    );
    serve_next(&mut t, &g);
    assert_eq!(hello(&frames(&out)).1, WireAttribution::Unattributed);
    let posted = g
        .read(&Principal::unattributed("watcher"), "r", false)
        .unwrap();
    assert!(posted[0].sender_label().ends_with("(unverified)"));
}

#[test]
fn a_forced_command_member_that_disagrees_with_the_table_is_unmapped() {
    // The key is known, but its forced command names someone else: the pair is not what was registered.
    let g = gateway();
    let mut t = FakeTransport::default();
    let out = t.peer(
        PeerEvidence::SshKey {
            fingerprint: "SHA256:alicekey".into(),
            forced_member: Some("mallory".into()),
        },
        &[],
    );
    serve_next(&mut t, &g);
    let (member, attribution) = hello(&frames(&out));
    assert_eq!(attribution, WireAttribution::Unattributed);
    assert_ne!(member, "alice");
}

#[test]
fn a_peer_cannot_use_a_request_to_become_someone_else() {
    let g = gateway();
    let mut t = FakeTransport::default();
    t.peer(
        PeerEvidence::None,
        &[req(
            "p",
            "post",
            json!({"room":"r","body":"x","sender":"alice","member":"alice"}),
        )],
    );
    serve_next(&mut t, &g);
    let posted = g
        .read(&Principal::unattributed("watcher"), "r", false)
        .unwrap();
    assert_ne!(posted[0].sender, "alice");
}

#[test]
fn the_unmapped_name_never_carries_a_secret_from_the_evidence() {
    let id = table();
    for ev in [
        PeerEvidence::bearer("super-secret-token"),
        PeerEvidence::SshKey {
            fingerprint: "SHA256:privatefp".into(),
            forced_member: None,
        },
        PeerEvidence::Certificate {
            subject: "CN=stranger".into(),
        },
    ] {
        let p = id.resolve(&ev).unwrap();
        assert!(!p.member.contains("secret") && !p.member.contains("privatefp"));
        assert!(crate::domain::valid_name(&p.member), "{}", p.member);
    }
}

#[test]
fn a_bearer_token_is_redacted_from_debug_output() {
    let text = format!("{:?}", PeerEvidence::bearer("super-secret-token"));
    assert!(!text.contains("super-secret-token"), "{text}");
}

#[test]
fn a_listener_that_runs_dry_says_so_and_a_connector_can_fail() {
    let mut t = FakeTransport::default();
    assert!(Listener::accept(&mut t).unwrap().is_none());
    assert!(Connector::connect(&mut t).is_err());
    t.peer(PeerEvidence::None, &[]);
    let conn = Connector::connect(&mut t).unwrap();
    assert_eq!(conn.evidence, PeerEvidence::None);
}

#[test]
fn the_traits_work_through_trait_objects() {
    let mut t = FakeTransport::default();
    t.peer(PeerEvidence::None, &[]);
    let l: &mut dyn Listener = &mut t;
    assert!(l.accept().unwrap().is_some());
}
