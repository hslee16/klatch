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
