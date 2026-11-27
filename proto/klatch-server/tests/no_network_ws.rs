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
    // the crate `ring`, not the end of another word (`string::`)
    let names_ring = |line: &str| {
        line.match_indices("ring::")
            .any(|(i, _)| !line[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
    };
    let users: Vec<String> = rust_sources()
        .iter()
        .filter(|(rel, text)| {
            !["src/tls.rs", "src/wire_ws.rs", "src/crl_sig.rs"].contains(&rel.as_str())
                && text.lines().any(|l| names_ring(code_of(l)))
        })
        .map(|(rel, _)| rel.clone())
        .collect();
    assert!(
        users.is_empty(),
        "ring is named outside tls.rs, wire_ws.rs and crl_sig.rs: {users:?}"
    );
    // and `crl_sig.rs` (#1648) names it only to verify a CRL's signature against a listed certificate's key (a sanity
    // check of the CRL file at start, no trust decision)
    let (_, sig) = rust_sources()
        .into_iter()
        .find(|(rel, _)| rel == "src/crl_sig.rs")
        .unwrap();
    let uses: Vec<&str> = sig.lines().map(code_of).filter(|l| names_ring(l)).collect();
    assert!(
        !uses.is_empty() && uses.iter().all(|l| l.contains("ring::signature::")),
        "crl_sig.rs uses ring for signature verification only: {uses:?}"
    );
    let (_, ws) = rust_sources()
        .into_iter()
        .find(|(rel, _)| rel == "src/wire_ws.rs")
        .unwrap();
    let digests: Vec<&str> = ws.lines().map(code_of).filter(|l| names_ring(l)).collect();
    assert!(
        digests.iter().all(|l| l.contains("digest")),
        "wire_ws.rs uses ring for a digest only: {digests:?}"
    );
}
