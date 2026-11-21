//! The session loop over the shared gateway (#102): the protocol of `klatch serve --stdio` for several connections over
//! the one gateway that an owner thread holds.

use super::shared::SharedGateway;
use crate::domain::Principal;
use crate::gateway::{AllowAll, Gateway, HistoryRange, SystemClock, Transport};
use crate::session::{Backend, Session, SessionConfig};
use crate::store::SqliteStore;
use crate::store::test_dir::{TempDb, temp_db};

fn shared(tag: &str) -> (SharedGateway, TempDb) {
    let db = temp_db(tag);
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        Ok(Gateway::new(store, SystemClock, AllowAll))
    })
    .unwrap();
    (gw, db)
}

/// Two concurrent sessions, each a scripted stream, post through the one owner: the sequence numbers they are given are
/// dense and unique, and the history through a plain transport holds every message once.
#[test]
fn two_sessions_over_one_shared_backend_get_dense_sequence_numbers() {
    let (gw, _db) = shared("shared-session");
    let backend = gw.session_backend();
    let run = |name: &'static str| {
        let backend = backend.clone();
        std::thread::spawn(move || {
            let who = Principal::unattributed(name);
            let mut script = String::from(
                "{\"v\":1,\"type\":\"request\",\"id\":\"j\",\"method\":\"join\",\"params\":{\"room\":\"r\"}}\n",
            );
            for i in 0..10 {
                script.push_str(&format!(
                    "{{\"v\":1,\"type\":\"request\",\"id\":\"p{i}\",\"method\":\"post\",\"params\":{{\"room\":\"r\",\"body\":\"{name}-{i}\"}}}}\n"
                ));
            }
            let mut out = Vec::new();
            let config = SessionConfig::default();
            Session::over(&backend, &who, &config)
                .serve(std::io::Cursor::new(script.into_bytes()), &mut out)
                .unwrap();
            String::from_utf8(out).unwrap()
        })
    };
    let (a, b) = (run("alice"), run("bob"));
    let mut seqs: Vec<i64> = Vec::new();
    for out in [a.join().unwrap(), b.join().unwrap()] {
        let lines: Vec<serde_json::Value> = out
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["type"], "hello");
        for l in &lines {
            if l["type"] == "response" && l["id"].as_str().is_some_and(|i| i.starts_with('p')) {
                assert_eq!(l["ok"], true, "{l}");
                seqs.push(l["result"]["seq"].as_i64().unwrap());
            }
        }
    }
    seqs.sort();
    assert_eq!(seqs, (1..=20).collect::<Vec<i64>>());
    let all = gw
        .transport(Principal::verified("alice"))
        .history(
            "r",
            HistoryRange {
                since: None,
                last: Some(50),
            },
        )
        .unwrap()
        .messages;
    assert_eq!(all.len(), 20);
}

/// A session that skips its hello (the transport already sent one) starts with the first response.
#[test]
fn a_session_without_hello_sends_none() {
    let (gw, _db) = shared("shared-nohello");
    let backend = gw.session_backend();
    let who = Principal::unattributed("alice");
    let config = SessionConfig::default();
    let script = "{\"v\":1,\"type\":\"request\",\"id\":\"j\",\"method\":\"join\",\"params\":{\"room\":\"r\"}}\n";
    let mut out = Vec::new();
    Session::over(&backend, &who, &config)
        .without_hello()
        .serve(std::io::Cursor::new(script.as_bytes().to_vec()), &mut out)
        .unwrap();
    let first: serde_json::Value =
        serde_json::from_str(String::from_utf8(out).unwrap().lines().next().unwrap()).unwrap();
    assert_eq!(first["type"], "response");
}

/// A credential check runs on the owner: a token nobody issued is refused, and so is text that is no token.
#[test]
fn a_session_backend_refuses_a_credential_nobody_issued() {
    let (gw, _db) = shared("shared-cred");
    let backend = gw.session_backend();
    let token = crate::keys::Token::generate().unwrap().to_text();
    assert!(backend.verify_credential("w-1", &token).is_err());
    assert!(backend.verify_credential("w-1", "not a token").is_err());
}

/// One scripted connection: the response lines it got, as JSON.
fn run_script(
    backend: &super::shared::SharedBackend,
    who: &str,
    requests: &[&str],
) -> Vec<serde_json::Value> {
    let who = Principal::unattributed(who);
    let mut script = String::new();
    for (i, r) in requests.iter().enumerate() {
        script.push_str(&format!(
            "{{\"v\":1,\"type\":\"request\",\"id\":\"q{i}\",\"method\":{r}}}\n"
        ));
    }
    let mut out = Vec::new();
    Session::over(backend, &who, &SessionConfig::default())
        .without_hello()
        .serve(std::io::Cursor::new(script.into_bytes()), &mut out)
        .unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|l| l["type"] == "response")
        .collect()
}

const JOIN_VAULT: &str = "\"join\",\"params\":{\"room\":\"vault\"}";
const POST_VAULT: &str =
    "\"post\",\"params\":{\"room\":\"vault\",\"body\":\"the sealed launch plan\"}";
const HISTORY_VAULT: &str = "\"history\",\"params\":{\"room\":\"vault\",\"last\":10}";

type Asked = std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>;

/// A shared gateway whose `vault` room is `private`, the label sealed, alice cleared to it. `keyed` is the key source
/// handed to the owner (none: the daemon as it was, with no keys).
fn sealed_shared(tag: &str, keyed: Option<Asked>) -> (SharedGateway, TempDb) {
    let db = temp_db(tag);
    let path = db.to_path_buf();
    let build = move || {
        let store = SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        let g = Gateway::new(store, SystemClock, AllowAll);
        let admin = Principal::unattributed("admin");
        g.admin_set_room_label(&admin, "vault", "private")?;
        g.admin_set_clearance(&admin, "alice", "private")?;
        g.admin_seal_label(&admin, "private")?;
        Ok(g)
    };
    let gw = match keyed {
        None => SharedGateway::spawn(build).unwrap(),
        Some(asked) => SharedGateway::spawn_keyed(build, move |labels: &[String]| {
            asked.lock().unwrap().push(labels.to_vec());
            let mut ring = crate::keys::LabelKeys::new();
            for l in labels {
                ring = ring.with(l, crate::keys::Key::from_bytes([9; 32]));
            }
            Ok(ring)
        })
        .unwrap(),
    };
    (gw, db)
}

/// With a key source a connection of a member cleared to a sealed label reads and writes it through the owner.
#[test]
fn a_cleared_connection_reads_and_writes_a_sealed_label_through_the_owner() {
    let asked: Asked = Default::default();
    let (gw, _db) = sealed_shared("shared-sealed-keyed", Some(asked.clone()));
    let out = run_script(
        &gw.session_backend(),
        "alice",
        &[JOIN_VAULT, POST_VAULT, HISTORY_VAULT],
    );
    assert!(out.iter().all(|l| l["ok"] == true), "{out:?}");
    assert!(
        out[2].to_string().contains("the sealed launch plan"),
        "{out:?}"
    );
    // the keys asked for are those alice's clearance reaches
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == &["private".to_string()])
    );
}

/// Without a key source (the daemon as it was) the same connection holds no key: the write fails closed.
#[test]
fn without_a_key_source_a_sealed_label_stays_sealed() {
    let (gw, _db) = sealed_shared("shared-sealed-nokeys", None);
    let out = run_script(&gw.session_backend(), "alice", &[JOIN_VAULT, POST_VAULT]);
    assert_eq!(out[1]["ok"], false, "{out:?}");
}

/// A member whose clearance does not reach the label is never asked a key for.
#[test]
fn a_connection_is_given_only_the_keys_its_clearance_reaches() {
    let asked: Asked = Default::default();
    let (gw, _db) = sealed_shared("shared-sealed-scope", Some(asked.clone()));
    let backend = gw.session_backend();
    let ok = run_script(&backend, "alice", &[JOIN_VAULT, POST_VAULT]);
    assert!(ok.iter().all(|l| l["ok"] == true), "{ok:?}");
    asked.lock().unwrap().clear();
    let bob = run_script(&backend, "bob", &[JOIN_VAULT, HISTORY_VAULT]);
    assert!(bob.iter().all(|l| l["ok"] == false), "{bob:?}");
    assert!(
        asked
            .lock()
            .unwrap()
            .iter()
            .all(|l| !l.contains(&"private".to_string())),
        "{:?}",
        asked.lock().unwrap()
    );
}

/// A room that requires a harness capability refuses a connection that is the generic floor, and admits one whose edge
/// verified a Claude Code session; the harness is per connection, not per gateway.
#[test]
fn a_harness_reaches_the_owner_for_one_connection_only() {
    let db = temp_db("shared-harness");
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        let g = Gateway::new(store, SystemClock, AllowAll);
        g.admin_set_room_require(
            &Principal::unattributed("admin"),
            "gated",
            Some("refuse:session-identity"),
        )?;
        Ok(g)
    })
    .unwrap();
    const JOIN: &str = "\"join\",\"params\":{\"room\":\"gated\"}";
    let plain = gw.session_backend();
    let floor = run_script(&plain, "alice", &[JOIN]);
    assert_eq!(floor[0]["ok"], false, "{floor:?}");
    assert!(
        floor[0]
            .to_string()
            .contains("generic lacks session-identity"),
        "{floor:?}"
    );
    let claude = gw.session_backend().with_harness(|| {
        Box::new(crate::harness::ClaudeCode::new(
            crate::harness::Support::Supported(crate::harness::Version(2, 1, 325)),
        ))
    });
    let verified = run_script(&claude, "alice", &[JOIN]);
    assert_eq!(verified[0]["ok"], true, "{verified:?}");
    let again = run_script(&plain, "bob", &[JOIN]);
    assert_eq!(again[0]["ok"], false, "{again:?}");
}
