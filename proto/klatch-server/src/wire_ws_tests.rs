//! Tests of the WebSocket route's upgrade checks (`wire_ws.rs`).

use super::*;
use crate::loopback::parse_head;

fn head(extra: &str) -> Head {
    let text = format!(
        "GET /v1/ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13{extra}"
    );
    parse_head(text.as_bytes()).unwrap()
}

#[test]
fn the_accept_value_is_the_one_of_rfc_6455_and_base64_pads() {
    assert_eq!(
        accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
    for (plain, coded) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(plain.as_bytes()), coded);
    }
}

#[test]
fn a_key_is_sixteen_bytes_of_base64() {
    assert!(is_key("dGhlIHNhbXBsZSBub25jZQ=="));
    for bad in [
        "",
        "dGhlIHNhbXBsZQ==",
        "dGhlIHNhbXBsZSBub25jZQ=A",
        "!!!!!!!!!!!!!!!!!!!!!!==",
        "dGhlIHNhbXBsZSBub25jZQ=====",
    ] {
        assert!(!is_key(bad), "{bad:?}");
    }
}

#[test]
fn an_upgrade_is_taken_only_whole_and_alone() {
    assert_eq!(
        check_upgrade(&head(""), b"").unwrap(),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
    // a list in Connection and any case are fine
    let listed = head("")
        .headers
        .iter()
        .map(|(n, v)| {
            (
                n.clone(),
                if n == "connection" {
                    "keep-alive, UPGRADE".to_string()
                } else {
                    v.clone()
                },
            )
        })
        .collect();
    let h = Head {
        headers: listed,
        ..head("")
    };
    assert!(check_upgrade(&h, b"").is_ok());
    let status = |r: Vec<u8>| {
        String::from_utf8(r)
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .to_string()
    };
    let mut dup = head("");
    dup.headers.push((
        "sec-websocket-key".into(),
        "dGhlIHNhbXBsZSBub25jZQ==".into(),
    ));
    assert_eq!(status(check_upgrade(&dup, b"").unwrap_err()), "400");
    assert_eq!(status(check_upgrade(&head(""), b"x").unwrap_err()), "400");
    let mut old = head("");
    old.headers.retain(|(n, _)| n != "sec-websocket-version");
    old.headers
        .push(("sec-websocket-version".into(), "8".into()));
    let r = String::from_utf8(check_upgrade(&old, b"").unwrap_err()).unwrap();
    assert!(
        r.starts_with("HTTP/1.1 426") && r.contains("Sec-WebSocket-Version: 13"),
        "{r}"
    );
}

#[test]
fn an_origin_header_is_refused_unless_named_exactly_and_once() {
    let allowed = vec!["https://app.example".to_string()];
    assert!(check_origin(&head(""), &allowed).is_ok());
    assert!(check_origin(&head("\r\nOrigin: https://app.example"), &allowed).is_ok());
    for origin in [
        "https://evil.example",
        "",
        "null",
        "https://app.example:1",
        "HTTPS://APP.EXAMPLE",
    ] {
        assert!(
            check_origin(&head(&format!("\r\nOrigin: {origin}")), &allowed).is_err(),
            "{origin:?}"
        );
        assert!(
            check_origin(&head(&format!("\r\nOrigin: {origin}")), &[]).is_err(),
            "{origin:?} with none allowed"
        );
    }
    let twice = head("\r\nOrigin: https://app.example\r\nOrigin: https://app.example");
    assert!(check_origin(&twice, &allowed).is_err());
}

#[test]
fn an_http_1_0_request_is_not_a_websocket_upgrade() {
    let text = "GET /v1/ws HTTP/1.0\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13";
    let old = parse_head(text.as_bytes()).unwrap();
    assert!(old.http10);
    let answer = String::from_utf8(check_upgrade(&old, b"").unwrap_err()).unwrap();
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    assert!(!head("").http10);
}

#[test]
fn only_the_credential_refusal_frame_ends_the_socket() {
    let revoked =
        br#"{"type":"refusal","v":1,"rule":"credential","reason":"the credential was revoked"}"#;
    let expired =
        br#"{"v":1,"type":"refusal","rule":"credential","reason":"the credential expired"}"#;
    assert!(ends_credential(revoked) && ends_credential(expired));
    // another rule, a message that quotes the words, a response and a frame too long to be one are not
    for other in [
        &br#"{"type":"refusal","v":1,"rule":"clearance","reason":"x"}"#[..],
        br#"{"type":"message","v":1,"room":"r","seq":1,"body":"\"credential\" refusal"}"#,
        br#"{"type":"response","v":1,"id":"a","ok":false,"error":{"code":"refused","message":"credential"}}"#,
        b"not json \"credential\"",
    ] {
        assert!(!ends_credential(other), "{}", String::from_utf8_lossy(other));
    }
    let mut long = revoked.to_vec();
    long.extend(std::iter::repeat_n(b' ', 2000));
    assert!(!ends_credential(&long));
}
