//! #1083: a session bound to something that can be taken away (an ssh key mapping) looks at the binding on every look
//! at the input and on every tick, and ends with a refusal once it is gone.

use super::*;

fn gone() -> Error {
    Error::Refused {
        rule: "ssh-key-unmapped".into(),
        reason: "the key is no longer mapped".into(),
    }
}

fn decode(out: Vec<u8>) -> Vec<ServerFrame> {
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| decode_server_line(l).unwrap())
        .collect()
}

#[test]
fn a_request_after_the_binding_went_is_refused_and_the_session_ends() {
    let g = gateway();
    let who = alice();
    let input = format!("{}\n", req("p", "post", json!({"room":"r","body":"after"})));
    let check = || Err(gone());
    let config = SessionConfig::default();
    let mut out = Vec::new();
    Session::new(&g, &who, &config)
        .fixed_member()
        .rechecked(&check)
        .serve(Cursor::new(input), &mut out)
        .unwrap();
    let frames = decode(out);
    assert!(!response(&frames, "p").0);
    assert!(frames.iter().any(|f| matches!(
        f,
        ServerFrame::Refusal { request: Some(i), rule, .. } if i == "p" && rule == "ssh-key-unmapped"
    )));
    // and a refusal that names no request says the session is over
    assert!(frames.iter().any(|f| matches!(
        f,
        ServerFrame::Refusal { request: None, rule, .. } if rule == "ssh-key-unmapped"
    )));
    // nothing was posted
    g.join(&bob(), "r").unwrap();
    assert!(g.read(&bob(), "r", true).unwrap().is_empty());
}

#[test]
fn a_binding_that_holds_changes_nothing() {
    let g = gateway();
    let who = alice();
    let input = format!("{}\n", req("p", "post", json!({"room":"r","body":"hello"})));
    let check = || Ok(());
    let config = SessionConfig::default();
    let mut out = Vec::new();
    Session::new(&g, &who, &config)
        .rechecked(&check)
        .serve(Cursor::new(input), &mut out)
        .unwrap();
    assert!(response(&decode(out), "p").0);
}

#[test]
fn an_idle_session_whose_binding_is_gone_ends_on_its_own() {
    let g = gateway();
    let who = alice();
    let config = SessionConfig {
        poll: std::time::Duration::from_millis(10),
        ..Default::default()
    };
    let check = || Err(gone());
    // the input never speaks and never ends: only the recheck can end this session
    let (reader, _writer) = std::io::pipe().unwrap();
    let mut out = Vec::new();
    Session::new(&g, &who, &config)
        .rechecked(&check)
        .serve(std::io::BufReader::new(reader), &mut out)
        .unwrap();
    let frames = decode(out);
    assert!(frames.iter().any(|f| matches!(
        f,
        ServerFrame::Refusal { request: None, rule, .. } if rule == "ssh-key-unmapped"
    )));
}
