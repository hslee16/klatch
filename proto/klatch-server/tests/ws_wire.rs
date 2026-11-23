//! The WebSocket route (#112, epic #13): `GET /v1/ws` upgrades a connection to a duplex session that speaks the wire of
//! `GET /v1/stream` plus `POST /v1/frames`, one frame line per text message. Covered here: the conformance scenario
//! (requests, events, acknowledgements) gives the same frames over WebSocket on loopback, WebSocket over mutual TLS and
//! server-sent events; a stream resumes by `seq`; a client without a valid credential is refused at the upgrade (the one
//! `401`, nothing about the member), and forwarded identity headers give none; `Origin` is refused unless the operator
//! named it; a malformed upgrade is refused; over TLS the refusals of #108 hold (no certificate, another CA, an expired
//! one, plaintext to the TLS port). The limits of the frame layer are in `ws_limits.rs`.

mod tls_common;

use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tls_common::scenario::*;
use tls_common::wire::*;
use tls_common::ws::*;
use tls_common::*;

fn upgrade_status(net: &Net, extra: &[(&str, &str)]) -> (u16, Vec<(String, String)>) {
    let (status, headers, _) = net.ws_upgrade(extra);
    (status, headers)
}

// ---------------------------------------------------------------------------------------------------------------
// conformance

#[test]
fn the_same_scenario_over_sse_websocket_and_websocket_over_tls_gives_the_same_frames() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    // one store each: the scenario posts the same bodies, and a store refuses a repeat of a member's message
    let (sse, plain) = (plain_server(&[]), plain_server(&[]));
    let t = tls_server();
    for (i, name) in ["alice", "bob"].into_iter().enumerate() {
        let over_sse = run_scenario(&sse.net, name, &sse.tokens[i]);
        let over_ws = run_scenario_ws(&plain.net, name, &plain.tokens[i]);
        let over_wss = run_scenario_ws(&t.net, name, &t.tokens[i]);
        let (ra, sa) = split(&over_sse);
        let (rb, sb) = split(&over_ws);
        let (rc, sc) = split(&over_wss);
        assert!(ra.len() >= 10 && !sa.is_empty(), "{name}: {over_sse:?}");
        assert_eq!(ra, rb, "{name}: responses over ws");
        assert_eq!(sa, sb, "{name}: stream frames over ws");
        assert_eq!(ra, rc, "{name}: responses over wss");
        assert_eq!(sa, sc, "{name}: stream frames over wss");
        // the events carry the store's own sequence numbers, in the same order
        let seqs: Vec<i64> = rc
            .iter()
            .filter(|f| f["id"].as_str().is_some_and(|i| i.starts_with('p')))
            .map(|f| f["result"]["seq"].as_i64().unwrap())
            .collect();
        assert_eq!(seqs.len(), 4, "{name}: {seqs:?}");
        // acknowledgements: the scenario acknowledges seq 2 of the subscription it opened, and the members' rooms are the
        // same afterwards over every transport (read gives what is unread after the acknowledgement)
        assert!(
            rc.iter().any(|f| f["id"] == "read" && f["ok"] == true),
            "{name}: no response to read"
        );
    }
}

fn resume_over(net: &Net, token: &str, room: &str) {
    let mut a = net.ws_open("alice", token);
    assert_eq!(a.recv().unwrap()["attribution"], "verified");
    assert_eq!(a.request("join", json!({"room": room}))["ok"], true);
    let seqs: Vec<i64> = (0..4).map(|i| a.post(room, &format!("m{i}"))).collect();
    assert_eq!(seqs, [1, 2, 3, 4]);
    drop(a);
    // a new connection asks for everything after the second message and gets the same messages with the same numbers
    let mut b = net.ws_open("alice", token);
    b.recv().unwrap();
    assert_eq!(
        b.request("subscribe", json!({"room": room, "from_seq": 2}))["ok"],
        true
    );
    let mut got = Vec::new();
    while got.len() < 2 {
        let f = b.recv().expect("the resumed messages");
        if f["type"] == "message" {
            got.push((
                f["seq"].as_i64().unwrap(),
                f["body"].as_str().unwrap().to_string(),
            ));
        }
    }
    assert_eq!(got, [(3, "m2".to_string()), (4, "m3".to_string())]);
    // an acknowledgement is a frame like any other: it is taken and the connection goes on
    b.text(&json!({"v":1,"type":"ack","room":room,"seq":4}).to_string());
    assert_eq!(b.post(room, "after-ack"), 5);
}

#[test]
fn a_connection_resumes_by_sequence_number_on_loopback_and_over_tls() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain = plain_server(&[]);
    resume_over(&plain.net, &plain.tokens[0], "res");
    let t = tls_server();
    resume_over(&t.net, &t.tokens[0], "res");
}

#[test]
fn the_upgrade_answer_is_a_101_with_the_accept_key_and_offers_no_extension_or_subprotocol() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain = plain_server(&[]);
    let creds = credential("alice", &plain.tokens[0]);
    let mut extra: Vec<(&str, &str)> = creds
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    // a client may ask for compression and a subprotocol; the server takes neither (it has no state for them)
    extra.push(("Sec-WebSocket-Extensions", "permessage-deflate"));
    extra.push(("Sec-WebSocket-Protocol", "chat"));
    let (status, headers) = upgrade_status(&plain.net, &extra);
    assert_eq!(status, 101, "{headers:?}");
    let has = |n: &str| headers.iter().any(|(h, _)| h == n);
    assert!(
        has("sec-websocket-accept")
            && !has("sec-websocket-extensions")
            && !has("sec-websocket-protocol")
    );
    assert!(
        headers
            .iter()
            .any(|(h, v)| h == "upgrade" && v.eq_ignore_ascii_case("websocket"))
    );
}

// ---------------------------------------------------------------------------------------------------------------
// who is calling

fn refused_without_a_credential(net: &Net, token: &str) {
    let ok = format!("Bearer {token}");
    let basic = format!("Basic {token}");
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("no credential at all", vec![]),
        (
            "a token and no member",
            vec![("Authorization", ok.as_str())],
        ),
        ("a member and no token", vec![("X-Klatch-Member", "alice")]),
        (
            "another member's token",
            vec![("X-Klatch-Member", "bob"), ("Authorization", ok.as_str())],
        ),
        (
            "a wrong token",
            vec![
                ("X-Klatch-Member", "alice"),
                ("Authorization", "Bearer klatch-not-a-token"),
            ],
        ),
        (
            "the wrong scheme",
            vec![
                ("X-Klatch-Member", "alice"),
                ("Authorization", basic.as_str()),
            ],
        ),
        (
            "forwarded identity only",
            vec![
                ("X-Forwarded-User", "alice"),
                ("X-Client-Cert", "CN=alice"),
                ("X-Remote-User", "alice"),
            ],
        ),
        (
            "a typed name",
            vec![("X-Klatch-Member", "alice"), ("X-Klatch-Name", "alice")],
        ),
    ];
    for (what, headers) in cases {
        let (status, h, mut r) = net.ws_upgrade(&headers);
        assert_eq!(status, 401, "{what}");
        assert!(
            !h.iter().any(|(n, _)| n.starts_with("sec-websocket")),
            "{what}: {h:?}"
        );
        // the answer is the one text and then the end: no frame, no stream
        let mut rest = String::new();
        let _ = r.read_to_string(&mut rest);
        assert_eq!(rest, "unauthorized", "{what}");
    }
}

#[test]
fn a_client_without_a_valid_credential_is_refused_at_the_upgrade_on_loopback_and_over_tls() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain = plain_server(&[]);
    refused_without_a_credential(&plain.net, &plain.tokens[0]);
    let t = tls_server();
    refused_without_a_credential(&t.net, &t.tokens[0]);
}

#[test]
fn a_credential_that_is_revoked_cannot_open_a_connection() {
    // the credential is checked live at the upgrade, as for a stream: an expired or revoked one is the same 401. Here a
    // member the store never issued a credential to stands for it (revocation itself is tested with the gateway).
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain = plain_server(&[]);
    let bearer = format!("Bearer {}", plain.tokens[0]);
    let (status, _) = upgrade_status(
        &plain.net,
        &[("X-Klatch-Member", "carol"), ("Authorization", &bearer)],
    );
    assert_eq!(status, 401);
}

// ---------------------------------------------------------------------------------------------------------------
// Origin

#[test]
fn a_request_with_an_origin_is_refused_unless_the_operator_named_it() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain = plain_server(&[]);
    let creds = credential("alice", &plain.tokens[0]);
    let with_origin = |origin: &str| -> Vec<(String, String)> {
        let mut h: Vec<(String, String)> = creds.to_vec();
        h.push(("Origin".into(), origin.into()));
        h
    };
    let status = |net: &Net, headers: &[(String, String)]| {
        let h: Vec<(&str, &str)> = headers
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        upgrade_status(net, &h).0
    };
    // a page on another site, a page on loopback, `null`, and an empty value: none is an identity and none is let in
    for origin in [
        "https://evil.example",
        "http://127.0.0.1:1",
        "http://localhost",
        "null",
        "",
    ] {
        assert_eq!(
            status(&plain.net, &with_origin(origin)),
            403,
            "Origin {origin:?} with a valid credential"
        );
    }
    // without a credential the answer is still the one 401: the Origin rule does not tell a stranger anything
    let stranger = [("Origin".to_string(), "https://evil.example".to_string())];
    assert_eq!(status(&plain.net, &stranger), 401);
    // a client that sends no Origin (a worker, a script) is let in
    assert_eq!(status(&plain.net, &creds.to_vec()), 101);
}

#[test]
fn an_origin_the_operator_named_is_let_in_exactly_and_no_other() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain = plain_server(&["--ws-allow-origin", "https://app.example"]);
    let creds = credential("alice", &plain.tokens[0]);
    let status = |origin: &str| {
        let mut h: Vec<(&str, &str)> = creds
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        h.push(("Origin", origin));
        upgrade_status(&plain.net, &h).0
    };
    assert_eq!(status("https://app.example"), 101);
    for other in [
        "http://app.example",
        "https://app.example:8443",
        "https://app.example.evil.example",
        "https://evil.example",
        "HTTPS://APP.EXAMPLE ",
        "null",
    ] {
        assert_eq!(status(other), 403, "{other:?}");
    }
}

#[test]
fn the_allowed_origin_flag_takes_only_a_plain_origin_and_has_no_wildcard() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    for bad in [
        "*",
        "app.example",
        "https://app.example/path",
        "ftp://x",
        "null",
        "",
        "https://*.example",
    ] {
        let dir = Dir::new();
        let (code, log) = refused_start(
            &dir,
            "http",
            &[
                "--listen".into(),
                "127.0.0.1:0".into(),
                "--ws-allow-origin".into(),
                bad.into(),
            ],
        );
        assert_eq!(code, Some(1), "{bad:?}: {log}");
        assert!(
            log.contains("--ws-allow-origin") && log.contains("such as"),
            "{bad:?}: {log}"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// a malformed upgrade

#[test]
fn a_request_that_is_not_a_valid_upgrade_is_refused_after_the_credential() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let plain = plain_server(&[]);
    let creds = credential("alice", &plain.tokens[0]);
    let auth: String = creds.iter().map(|(n, v)| format!("{n}: {v}\r\n")).collect();
    let raw = |head: &str, extra_bytes: &[u8]| -> u16 {
        let mut c = plain.net.connect();
        c.write_all(head.as_bytes()).unwrap();
        c.write_all(extra_bytes).unwrap();
        c.flush().unwrap();
        read_head(&mut std::io::BufReader::new(c)).0
    };
    let good = format!(
        "GET /v1/ws HTTP/1.1\r\nHost: x\r\n{auth}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    assert_eq!(raw(&good, b""), 101);
    let no_upgrade = format!("GET /v1/ws HTTP/1.1\r\nHost: x\r\n{auth}\r\n");
    let wrong_upgrade = good.replace("Upgrade: websocket", "Upgrade: h2c");
    let no_connection = good.replace("Connection: Upgrade\r\n", "");
    let connection_close = good.replace("Connection: Upgrade", "Connection: close");
    let no_key = good.replace(&format!("Sec-WebSocket-Key: {KEY}\r\n"), "");
    let short_key = good.replace(KEY, "dGhlIHNhbXBsZQ==");
    let not_base64 = good.replace(KEY, "!!!!!!!!!!!!!!!!!!!!!!==");
    let two_keys = good.replace(
        "Sec-WebSocket-Version: 13\r\n",
        &format!("Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {KEY}\r\n"),
    );
    let version_8 = good.replace("Version: 13", "Version: 8");
    let no_version = good.replace("Sec-WebSocket-Version: 13\r\n", "");
    for (what, head) in [
        ("no upgrade headers", &no_upgrade),
        ("Upgrade: h2c", &wrong_upgrade),
        ("no Connection header", &no_connection),
        ("Connection: close", &connection_close),
        ("no key", &no_key),
        ("a key of the wrong length", &short_key),
        ("a key that is not base64", &not_base64),
        ("two keys", &two_keys),
        ("version 8", &version_8),
        ("no version", &no_version),
    ] {
        let status = raw(head, b"");
        assert!(status == 400 || status == 426, "{what}: {status}");
    }
    // bytes that arrive with the request, before the 101, are not a frame of a session that does not exist yet
    assert_eq!(raw(&good, &encode(true, TEXT, b"{}", true)), 400);
    // the route takes a GET and nothing else; a POST is a 405 that names the method
    let post = good.replace("GET /v1/ws", "POST /v1/ws");
    assert_eq!(raw(&post, b""), 405);
    // and without the credential none of this is told: the 401 comes first
    let unauth = no_key.replace(&auth, "");
    assert_eq!(raw(&unauth, b""), 401);
}

// ---------------------------------------------------------------------------------------------------------------
// the refusals of the TLS listener hold for the WebSocket route

/// What a client sends through TLS `config` as an upgrade and gets back: an error, or the text that came.
fn upgrade_over_tls(
    addr: std::net::SocketAddr,
    config: &std::sync::Arc<rustls::ClientConfig>,
) -> std::io::Result<String> {
    let mut tls = tls_connect(addr, config)?;
    let creds = credential("alice", "x");
    let extra: Vec<(&str, &str)> = creds
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    tls.write_all(upgrade_request(&extra).as_bytes())?;
    let mut buf = [0u8; 12];
    tls.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

#[test]
fn over_tls_no_certificate_another_ca_an_expired_one_and_plaintext_never_reach_the_upgrade() {
    let dir = Dir::new();
    let (server_ca, client_ca) = (Ca::new("server ca"), Ca::new("client ca"));
    let files = server_files(
        &dir,
        &server_ca.issue("klatch", Use::Server),
        &client_ca,
        0o600,
    );
    let server = Server::started(&dir, "https", &tls_args(&files));
    let stranger = Ca::new("stranger").issue("worker-1", Use::Client);
    let old = client_ca.issue("worker-1", Use::ExpiredClient);
    let server_use = server_ca.issue("klatch", Use::Server);
    for (what, config) in [
        ("no certificate", client_config(&server_ca, None, None)),
        (
            "another CA",
            client_config(&server_ca, Some(&stranger), None),
        ),
        (
            "an expired certificate",
            client_config(&server_ca, Some(&old), None),
        ),
        (
            "a server certificate",
            client_config(&server_ca, Some(&server_use), None),
        ),
    ] {
        let got = upgrade_over_tls(server.addr, &config);
        assert!(got.is_err(), "{what}: {got:?}");
    }
    // a valid certificate gets as far as the credential, and a wrong one is the 401
    let leaf = client_ca.issue("worker-1", Use::Client);
    let ok = client_config(&server_ca, Some(&leaf), None);
    assert!(
        upgrade_over_tls(server.addr, &ok)
            .unwrap()
            .starts_with("HTTP/1.1 401")
    );
    // plaintext to the TLS port gets no HTTP answer and no upgrade
    let mut sock = TcpStream::connect(server.addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ = sock.write_all(upgrade_request(&[("Authorization", "Bearer x")]).as_bytes());
    let mut got = Vec::new();
    let _ = sock.read_to_end(&mut got);
    let text = String::from_utf8_lossy(&got);
    assert!(
        !text.contains("HTTP/1") && !text.contains("101"),
        "{text:?}"
    );
}

#[test]
fn the_route_is_on_both_listeners_and_a_session_over_tls_is_the_members_own() {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = tls_server();
    let mut a = t.net.ws_open("alice", &t.tokens[0]);
    let mut b = t.net.ws_open("bob", &t.tokens[1]);
    let (ha, hb) = (a.recv().unwrap(), b.recv().unwrap());
    assert_eq!(
        (ha["member"].as_str(), hb["member"].as_str()),
        (Some("alice"), Some("bob"))
    );
    assert_eq!(a.request("join", json!({"room": "own"}))["ok"], true);
    assert_eq!(b.request("join", json!({"room": "own"}))["ok"], true);
    // the sender of a post is the member of the connection: a frame cannot name another one
    let r = a.request(
        "post",
        json!({"room": "own", "body": "from a", "sender": "bob"}),
    );
    let seq = r["result"]["seq"].as_i64().unwrap();
    let h = b.request("history", json!({"room": "own", "last": 5}));
    let senders: Vec<&str> = h["result"]["messages"]
        .as_array()
        .map(|m| m.iter().filter_map(|x| x["sender"].as_str()).collect())
        .unwrap_or_default();
    assert!(seq >= 1 && !senders.is_empty(), "no messages in {h}");
    assert!(senders.iter().all(|s| *s == "alice"), "{h}");
}
