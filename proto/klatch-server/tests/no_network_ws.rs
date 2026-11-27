//! The WebSocket route's dependency (#112): `tungstenite` is in `klatch-server`'s tree as its frame layer and nothing more.

mod scan_common;

use scan_common::*;
use std::path::Path;

#[test]
fn websocket_is_the_frame_layer_of_tungstenite_alone_with_no_handshake_no_runtime_and_one_user() {
    // #112: `tungstenite` with `default-features = false` is the protocol's frame layer: masking, fragmentation, the size
    // limits, ping and pong and the close handshake, over any `Read + Write`. It does no I/O of its own and brings no HTTP
    // parser, no `sha1` (banned by deny.toml) and no runtime; the upgrade request is read by `loopback.rs`.
    let deps = dependency_names("normal");
    assert!(deps.iter().any(|d| d == "tungstenite"));
    for n in [
        "http",
        "httparse",
        "sha1",
        "tokio-tungstenite",
        "tokio",
        "futures",
        "futures-util",
        "async-std",
        "mio",
        "socket2",
        "hyper",
    ] {
        assert!(
            !deps.iter().any(|d| d == n),
            "{n} is in klatch-server's dependencies next to tungstenite"
        );
    }
    // only the WebSocket module names it, and it never turns the handshake feature on
    let problems: Vec<String> = rust_sources()
        .iter()
        .filter(|(rel, _)| rel != "src/wire_ws.rs")
        .flat_map(|(rel, text)| {
            text.lines()
                .enumerate()
                .filter(|(_, l)| code_of(l).contains("tungstenite"))
                .map(move |(n, _)| format!("{rel}:{}: names tungstenite outside wire_ws.rs", n + 1))
        })
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let line = manifest
        .lines()
        .find(|l| l.starts_with("tungstenite"))
        .expect("the dependency line");
    assert!(
        line.contains("default-features = false") && !line.contains("handshake"),
        "{line}"
    );
}

#[test]
fn ring_is_named_only_for_the_tls_provider_and_for_the_sha_1_of_the_upgrade_answer() {
    // `klatch-server` names ring in two places: the provider handed to rustls (`tls.rs`), and the SHA-1 of the WebSocket
    // upgrade answer (`wire_ws.rs`, #112: RFC 6455 mandates it; it is no security boundary)
    // read as tokens (#1704): a second `ring::rand` use on the line of the `ring::signature` import, a `ring::{..}` group
    // and `extern crate ring as rr` all get past a scan of lines, and none gets past this one
    let users: Vec<String> = rust_sources()
        .iter()
        .filter(|(rel, text)| {
            !["src/tls.rs", "src/wire_ws.rs", "src/crl_sig.rs"].contains(&rel.as_str())
                && !ring_paths(text).is_empty()
        })
        .map(|(rel, _)| rel.clone())
        .collect();
    assert!(
        users.is_empty(),
        "ring is named outside tls.rs, wire_ws.rs and crl_sig.rs: {users:?}"
    );
    // and `crl_sig.rs` (#1648) names it only to verify a CRL's signature against a listed certificate's key (a sanity
    // check of the CRL file at start, no trust decision): every naming is a path into `ring::signature`
    let only = |file: &str, module: &str| {
        let (_, text) = rust_sources()
            .into_iter()
            .find(|(rel, _)| rel == file)
            .unwrap();
        let uses = ring_paths(&text);
        assert!(
            !uses.is_empty() && uses.iter().all(|m| m == module),
            "{file} names ring other than as ring::{module}: {uses:?}"
        );
    };
    only("src/crl_sig.rs", "signature");
    // `wire_ws.rs` uses ring for a digest only (the SHA-1 of the upgrade answer)
    only("src/wire_ws.rs", "digest");
}

#[test]
fn the_ring_scan_sees_a_second_use_on_one_line_a_group_an_alias_and_ignores_comments_strings_and_other_words()
 {
    let paths = |t: &str| ring_paths(t);
    assert_eq!(paths("use ring::signature::{self, Key};"), ["signature"]);
    // a second use on the line of an allowed one
    assert_eq!(
        paths("use ring::signature::Key; use ring::rand::SystemRandom;"),
        ["signature", "rand"]
    );
    assert_eq!(
        paths("let a = ring::signature::X; let b = ring::rand::Y;"),
        ["signature", "rand"]
    );
    // a group names several modules at once, and an alias hides every later use
    assert_eq!(paths("use ring::{signature, rand};"), [""]);
    assert_eq!(paths("extern crate ring as rr;"), [""]);
    assert_eq!(paths("use ring as rr;"), [""]);
    assert_eq!(paths("use ring;"), [""]);
    // a path through another crate that ends in ring is the crate's own module
    assert_eq!(
        paths("rustls::crypto::ring::default_provider()"),
        ["default_provider"]
    );
    // a raw identifier is the crate's name too, and does not swallow what follows it
    assert_eq!(paths("let _ = r#ring::rand::X::new();"), ["rand"]);
    assert_eq!(
        paths("ring::signature::X; use r#ring::rand; use ring::rand::Y;"),
        ["signature", "rand", "rand"]
    );
    // a real raw string holding ring::rand is a string (with and without hashes)
    assert!(paths("let a = r#\"ring::rand \"quoted\" \"#; let b = r\"ring::rand\"; let c = br\"ring::rand\";").is_empty());
    assert_eq!(
        paths("let a = r#\"x\"#; use ring::signature::K;"),
        ["signature"]
    );
    // not the crate: another word, a comment, a string, a block comment
    assert!(paths("let string = 1; let ring_size = 2; fn during() {}").is_empty());
    assert!(
        paths("// use ring::rand::X;\nlet a = 1; /* ring::rand */ let b = \"ring::rand\";")
            .is_empty()
    );
}
