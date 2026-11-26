//! `klatch-server` starts no program and makes no outbound connection. Since phase 2 (#1058) it may listen on TCP, but
//! only in `src/loopback.rs`, which refuses every address but 127.0.0.1 and ::1, with one narrow exception (#108): the
//! mutual TLS listener (`klatch-server https`) may bind another address when the operator allows it, through
//! `bind_mutual_tls`, which asks `klatch::session::policy::check_mutual_tls_bind` first (a client CA is required and an
//! address off loopback needs `--allow-off-loopback`). TLS is rustls (with ring), named only in `src/tls.rs` and `src/tls_bridge.rs`, and no
//! call that skips a check is named anywhere. Since #102 it may also listen on a Unix
//! domain socket and ask the kernel who is on the other end, but only in `src/daemon.rs`. This is an audit by inspection, like the one for
//! the `klatch` binary (klatch/tests/no_telemetry.rs): it checks what the crate could do (its dependencies and its
//! source), not what one run did. The rules: socket types and `std::net` only in the loopback module (and its tests, for
//! a client connecting to their own listener); no `TcpStream::connect` anywhere else; exactly one `TcpListener::bind`,
//! after the loopback check; the Unix socket types and the peer-credential calls only in the daemon module, which has
//! one `UnixListener::bind` and one `UnixStream::connect` (the probe of its own socket path that tells a live daemon from
//! a dead one's file); no UDP, datagram socket, name resolution or HTTP client anywhere; no `Command`.

mod scan_common;

use scan_common::*;
use std::path::Path;

/// The one source file that may name the socket types and open a listener (#1058): the loopback listener, which refuses
/// every address but 127.0.0.1 and ::1, and (#108) the policy-checked bind of the mutual TLS listener. Phases 3 and 4 and
/// the TLS listener reuse it; no other file gets this allowance. `tls.rs` and `wire_https.rs` name only the `Stream` and
/// `Listener` aliases and `loopback::connected_pair`, never a socket type.
const LOOPBACK_ONLY: &[&str] = &["src/loopback.rs"];

/// The one source file that may name the Unix socket types and the kernel's peer-credential calls (#102): the daemon,
/// which listens on one Unix socket in a directory only its user can write to and closes a peer of another uid before it
/// reads a byte. TCP stays out of it (the `SOCKETS` rule applies to it).
const DAEMON_ONLY: &[&str] = &["src/daemon.rs"];

/// What only the daemon module may name.
const DAEMON_NAMES: &[&str] = &[
    "UnixListener",
    "UnixStream",
    "std::os::unix::net",
    "getsockopt",
    "getpeereid",
    "SO_PEERCRED",
    "LOCAL_PEERPID",
];

/// The one source file that may connect a client socket: the tests of the loopback listener, which connect to a listener
/// they started themselves on a loopback port the system chose. Nothing else may connect, in any file.
const TEST_CLIENT: &[&str] = &["src/loopback_tests.rs"];

/// Socket types and the network module: only the loopback module (and, for a client, its tests) may name them.
const SOCKETS: &[&str] = &["TcpStream", "TcpListener", "std::net", "SocketAddr"];

/// Never anywhere, not even in the loopback module: datagram and unix sockets, name resolution, an HTTP client, an
/// all-interfaces address.
const NEVER: &[&str] = &[
    "UdpSocket",
    "UnixDatagram",
    "ToSocketAddrs",
    "ureq",
    "reqwest",
    "hyper",
    "UNSPECIFIED",
];

/// Programs this crate may start by literal name: none. (The `klatch` library starts git, ps and the keystores for the
/// member setup; that is the library's own audit.)
const PROGRAMS: &[&str] = &[];

/// The names of `set` that `code` contains.
fn found<'a>(code: &'a str, set: &'a [&'a str]) -> impl Iterator<Item = &'a str> + 'a {
    set.iter().copied().filter(move |name| code.contains(name))
}

fn is_in(rel: &str, files: &[&str]) -> bool {
    files.contains(&rel)
}

/// Socket types and `std::net` outside the loopback module and its tests.
fn socket_problems(rel: &str, code: &str, at: &str) -> Vec<String> {
    if is_in(rel, LOOPBACK_ONLY) || is_in(rel, TEST_CLIENT) {
        return Vec::new();
    }
    // `std::net::Shutdown` is what `UnixStream::shutdown` takes: not a network type
    let net_code = code.replace("std::net::Shutdown", "");
    found(&net_code, SOCKETS)
        .map(|s| format!("{at}: uses {s}"))
        .collect()
}

/// The Unix socket names and peer-credential calls outside the daemon module, and the names never allowed anywhere.
fn name_problems(rel: &str, code: &str, at: &str) -> Vec<String> {
    let outside_daemon = if is_in(rel, DAEMON_ONLY) {
        Vec::new()
    } else {
        found(code, DAEMON_NAMES)
            .map(|s| format!("{at}: uses {s} outside the daemon module"))
            .collect()
    };
    let never = found(code, NEVER).map(|s| format!("{at}: uses {s}"));
    outside_daemon.into_iter().chain(never).collect()
}

/// Whether a line opens an outbound connection: a connect is allowed only in the loopback listener's own tests.
fn connects_outward(rel: &str, code: &str) -> bool {
    let unix_connect = code.contains("UnixStream::connect(") && !is_in(rel, DAEMON_ONLY);
    let connects =
        code.contains("TcpStream::connect") || code.contains("connect_timeout(") || unix_connect;
    connects && !is_in(rel, TEST_CLIENT)
}

/// A program started: none, by literal name or chosen at run time, and the process `Command` type is not named.
fn program_problems(code: &str, at: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(i) = code.find("Command::new(") {
        let arg = &code[i + "Command::new(".len()..];
        match arg.strip_prefix('"').and_then(|a| a.split('"').next()) {
            Some(p) if PROGRAMS.contains(&p) => {}
            Some(p) => problems.push(format!("{at}: runs {p}")),
            None => problems.push(format!("{at}: runs a program chosen at run time")),
        }
    }
    if code.contains("process::Command") || code.contains("std::process::Command") {
        problems.push(format!("{at}: names the process Command type"));
    }
    problems
}

/// Everything one line of source may not do.
fn line_problems(rel: &str, code: &str, at: &str) -> Vec<String> {
    let mut problems = socket_problems(rel, code, at);
    problems.extend(name_problems(rel, code, at));
    if connects_outward(rel, code) {
        problems.push(format!("{at}: opens an outbound connection"));
    }
    // No listener but the one in `bind_loopback`.
    if code.contains("TcpListener::bind") && !is_in(rel, LOOPBACK_ONLY) {
        problems.push(format!(
            "{at}: binds a listener outside the loopback module"
        ));
    }
    problems.extend(program_problems(code, at));
    problems
}

#[test]
fn the_source_opens_no_socket_outside_the_loopback_module_and_starts_no_program() {
    let sources = rust_sources();
    assert!(!sources.is_empty());
    let problems: Vec<String> = sources
        .iter()
        .flat_map(|(rel, text)| {
            text.lines().enumerate().flat_map(move |(n, line)| {
                line_problems(rel, code_of(line), &format!("{rel}:{}", n + 1))
            })
        })
        .collect();
    assert!(
        problems.is_empty(),
        "outbound capability in the source:\n{}",
        problems.join("\n")
    );
}

/// The text of the function that starts at `signature` in `text`, up to the next top-level closing brace.
fn function<'a>(text: &'a str, signature: &str) -> &'a str {
    let start = text
        .find(signature)
        .unwrap_or_else(|| panic!("{signature}"));
    text[start..].split("\n}\n").next().unwrap()
}

#[test]
fn there_is_exactly_one_bind_and_each_way_to_it_checks_the_address_first() {
    let sources = rust_sources();
    let binds: Vec<&str> = sources
        .iter()
        .flat_map(|(rel, text)| {
            text.lines()
                .filter(|l| code_of(l).contains("TcpListener::bind"))
                .map(move |_| rel.as_str())
        })
        .collect();
    assert_eq!(
        binds,
        ["src/loopback.rs"],
        "one bind, in the loopback module"
    );
    let (_, text) = sources
        .iter()
        .find(|(rel, _)| rel == "src/loopback.rs")
        .unwrap();
    // the loopback way: the check, then the one bind
    let body = function(text, "pub fn bind_loopback");
    let (check, bind) = (
        body.find("check_loopback(")
            .expect("the address is checked"),
        body.find("bind_checked(").expect("the one bind"),
    );
    assert!(check < bind, "the loopback check comes before the bind");
    // the TLS way (#108): the policy check, then the one bind; nothing else binds
    let body = function(text, "pub fn bind_mutual_tls");
    let (check, bind) = (
        body.find("check_mutual_tls_addr(")
            .expect("the policy check"),
        body.find("bind_checked(").expect("the one bind"),
    );
    assert!(check < bind, "the policy check comes before the bind");
    let check = function(text, "pub fn check_mutual_tls_addr");
    assert!(
        check.contains("check_mutual_tls_bind("),
        "the policy module decides"
    );
    let callers = |name: &str| -> Vec<&str> {
        sources
            .iter()
            .filter(|(rel, t)| {
                rel != "src/loopback.rs" && t.lines().map(code_of).any(|l| l.contains(name))
            })
            .map(|(rel, _)| rel.as_str())
            .collect()
    };
    assert_eq!(callers("bind_checked("), Vec::<&str>::new());
    assert_eq!(callers("bind_mutual_tls("), ["src/wire_https.rs"]);
    // the only addresses the plaintext listener accepts are spelled out in one function, and it does not use `is_loopback`
    let allowed = function(text, "fn is_allowed_ip");
    assert!(allowed.contains("Ipv4Addr::LOCALHOST") && allowed.contains("Ipv6Addr::LOCALHOST"));
    assert!(
        !allowed.contains("is_loopback"),
        "127.0.0.2 and the like stay refused"
    );
}

#[test]
fn the_daemon_module_has_one_unix_listener_one_probe_and_checks_the_peer_before_it_serves() {
    let sources = rust_sources();
    let (_, text) = sources
        .iter()
        .find(|(rel, _)| rel == "src/daemon.rs")
        .expect("the daemon module");
    let code: Vec<&str> = text
        .lines()
        .map(code_of)
        .take_while(|l| !l.contains("#[cfg(test)]"))
        .collect();
    let count = |needle: &str| code.iter().filter(|l| l.contains(needle)).count();
    assert_eq!(count("UnixListener::bind("), 1, "one listener");
    assert_eq!(
        count("UnixStream::connect("),
        1,
        "one probe, of the daemon's own path"
    );
    for forbidden in [
        "TcpListener",
        "TcpStream",
        "TcpSocket",
        "0.0.0.0",
        "SocketAddr",
    ] {
        assert_eq!(count(forbidden), 0, "{forbidden}");
    }
    // the order of admitting a connection (`admit`): who the peer is, then the uid check; and in the accept loop
    // (`accept_loop`) a connection is admitted before any thread is spawned for it, so nothing is served before the uid check
    let admit = &text[text.find("fn admit<").expect("admit")..];
    let (cred, uid) = (
        admit.find("peer_cred(&stream)").expect("peer credentials"),
        admit.find("local_evidence(peer").expect("the uid check"),
    );
    assert!(cred < uid, "who the peer is comes before the uid check");
    let looped = &text[text.find("fn accept_loop(").expect("accept_loop")..];
    let (admitted, spawn) = (
        looped.find("admit(").expect("a connection is admitted"),
        looped.find("scope.spawn(").expect("the connection thread"),
    );
    let body = &text[text.find("fn run(").expect("run")..];
    assert!(
        admitted < spawn,
        "the uid check comes before anything is served"
    );
    // the socket is born 0600 (umask) and made 0600, and the directory is checked before the path is cleared
    assert!(text.contains("umask(0o177)") && text.contains("from_mode(0o600)"));
    let dir_check = body.find("checked_directory(").unwrap();
    let clear = body.find("clear_path(").unwrap();
    assert!(dir_check < clear);
}

#[test]
fn no_http_or_networking_crate_is_in_the_dependency_tree() {
    let deps = dependency_names("normal");
    const NETWORK: &[&str] = &[
        "reqwest",
        "hyper",
        "hyper-util",
        "h2",
        "ureq",
        "curl",
        "curl-sys",
        "isahc",
        "attohttpc",
        "surf",
        "minreq",
        "tiny_http",
        "axum",
        "warp",
        "actix-web",
        // tungstenite itself is allowed since #112, as its frame layer only: see the test below that holds it to that
        "tokio-tungstenite",
        "tokio",
        "async-std",
        "smol",
        "mio",
        "socket2",
        "native-tls",
        "openssl",
        "quinn",
        "hickory-resolver",
        "trust-dns-resolver",
    ];
    for n in NETWORK {
        assert!(
            !deps.iter().any(|d| d == n),
            "a networking crate is in klatch-server's dependencies: {n}"
        );
    }
    assert!(deps.len() > 10, "the tree looks empty: {deps:?}");
}

#[test]
fn the_crate_is_not_published_and_ships_through_archives_wheels_and_the_formula() {
    // Decision D1 (#1236): klatch-server is the second binary of every release archive, the platform wheels and the
    // Homebrew formula. It stays off crates.io on purpose: `cargo install klatch` keeps installing only the network-free
    // binary, and a crate that opens sockets is not a thing a user gets by accident. Publishing it later is a decision
    // that removes this line and adds `cargo publish --dry-run --locked -p klatch-server` to CI and the gate.
    let toml =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(
        toml.lines().any(|l| l.trim() == "publish = false"),
        "klatch-server is not published to crates.io (#1236)"
    );
}

#[test]
fn the_optional_hosted_judge_transport_is_not_linked_into_the_server() {
    // `jev-http` (ureq, rustls, ring, webpki-roots) is an optional feature of the klatch library that no shipped binary
    // may link (#871). Since #108 the server links rustls, ring and rustls-webpki ITSELF, for its own TLS listener, so
    // the old check (those three names absent from the server's tree) cannot stand. It is replaced by three that together
    // say more: (1) `ureq` and `webpki-roots`, the parts only the judge transport uses, are still absent from the
    // server's tree; (2) the library's own build, with its default features, links none of the five names, so the judge
    // transport does not come in with the library; (3) in the server's tree the only crate that depends on rustls is
    // klatch-server, so no other path (ureq, the library) brings it in, and ring and rustls-webpki are reached only
    // through rustls.
    let deps = dependency_names("normal,build");
    for n in ["ureq", "webpki-roots"] {
        assert!(
            !deps.iter().any(|d| d == n),
            "{n} (the jev-http transport) is linked into klatch-server"
        );
    }
    let library = tree(&["-p", "klatch", "--edges", "normal,build"]);
    for n in ["ureq", "rustls", "ring", "webpki-roots", "rustls-webpki"] {
        assert!(
            !library.iter().any(|d| d == n),
            "{n} is in the klatch library's default build (the jev-http transport is optional)"
        );
    }
    let dependents = |krate: &str| -> Vec<String> {
        let lines = tree(&[
            "-p",
            "klatch-server",
            "--edges",
            "normal,build",
            "--invert",
            krate,
            "--depth",
            "1",
        ]);
        let mut names: Vec<String> = lines.into_iter().skip(1).collect();
        names.sort();
        names.dedup();
        names
    };
    assert_eq!(
        dependents("rustls"),
        ["klatch-server"],
        "who depends on rustls"
    );
    let webpki = dependents("rustls-webpki");
    assert!(webpki.iter().all(|d| d == "rustls"), "{webpki:?}");
    let ring = dependents("ring");
    // `klatch-server` itself depends on ring for the SHA-1 of the WebSocket upgrade answer (#112; `no_network_ws.rs` checks
    // where it is named)
    assert!(
        ring.iter()
            .all(|d| d == "rustls" || d == "rustls-webpki" || d == "klatch-server"),
        "{ring:?}"
    );
}

#[test]
fn the_only_tls_library_is_rustls_with_ring_and_no_other_crypto_or_tls_crate_is_linked() {
    // #108 (decision of #157): rustls, which was already in the tree for the optional `jev-http` feature of the library.
    // OpenSSL is not linked for TLS (SQLCipher's vendored copy is the store's, behind `openssl-src`), and no second TLS
    // stack or a certificate generator is in the build: certificates are the operator's.
    let deps = dependency_names("normal,build");
    assert!(deps.iter().any(|d| d == "rustls") && deps.iter().any(|d| d == "ring"));
    for n in [
        "native-tls",
        "openssl",
        "aws-lc-rs",
        "aws-lc-sys",
        "boring",
        "rcgen",
        "x509-parser",
        "rustls-native-certs",
        "webpki-roots",
    ] {
        assert!(
            !deps.iter().any(|d| d == n),
            "{n} is linked into klatch-server"
        );
    }
    // the certificate generator the tests use is a dev-dependency only
    let normal = dependency_names("normal");
    assert!(
        !normal.iter().any(|d| d == "rcgen"),
        "rcgen is a dev-dependency only"
    );
}

/// The only files that may name rustls: the handshake and configuration, and the pump of an established connection.
const TLS_MODULES: &[&str] = &["src/tls.rs", "src/tls_bridge.rs", "src/tls_crl.rs"];

/// Names that skip or weaken a check: none may appear in the source, in any file. (A test file may build a client that
/// trusts its own throwaway CA; that is `tests/`, not `src/`.)
const SKIPS: &[&str] = &[
    "dangerous()",
    "danger::",
    "with_no_client_auth",
    "allow_unauthenticated",
    "NoClientAuth",
    "ServerCertVerifier",
    "ClientCertVerifier",
    "AllowAnyAnonymousClient",
    "with_cert_resolver",
    "verify_hostname",
    "SKIP_VERIFY",
];

#[test]
fn rustls_is_named_only_in_the_tls_module_and_nothing_names_a_way_to_skip_a_check() {
    let sources = rust_sources();
    let mut problems = Vec::new();
    for (rel, text) in &sources {
        // what the tests of a module name is not what the server could do
        let text = text
            .split("#[cfg(test)]\nmod tests {")
            .next()
            .unwrap_or(text);
        for (n, line) in text.lines().enumerate() {
            let code = code_of(line);
            let at = format!("{rel}:{}", n + 1);
            if code.contains("rustls") && !TLS_MODULES.contains(&rel.as_str()) {
                problems.push(format!("{at}: names rustls outside the TLS modules"));
            }
            for skip in SKIPS {
                // the usage text and the help may say there is no such option; code may not name one
                if code.contains(skip) && !rel.ends_with("_tests.rs") {
                    problems.push(format!("{at}: names {skip}"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // the one verifier is rustls's own, built on the operator's CA file, and client authentication is its default (a
    // required certificate); the module never builds one that makes it optional
    let (_, tls) = sources.iter().find(|(rel, _)| rel == "src/tls.rs").unwrap();
    assert!(tls.contains("WebPkiClientVerifier::builder_with_provider("));
    assert!(tls.contains("with_client_cert_verifier("));
    assert!(tls.contains("check_peer(Security::MutualTls"));
    // the CRL option (#1491) goes through that same verifier, with expiry enforced; no second verifier is built anywhere
    assert!(tls.contains(".with_crls("));
    assert!(tls.contains(".enforce_revocation_expiration()"));
    // counted by occurrence in every source file (tls.rs and tls_crl.rs included, test modules excluded): one builder call,
    // one place that installs a client verifier, and no verifier of the crate's own (the SKIPS list above refuses the type
    // names; this refuses an `impl` of the trait in any spelling)
    let code_of_all = || {
        sources.iter().map(|(rel, text)| {
            let code = text
                .split("#[cfg(test)]\nmod tests {")
                .next()
                .unwrap_or(text);
            (rel.clone(), code.to_string())
        })
    };
    let count = |needle: &str| -> usize {
        code_of_all()
            .map(|(_, code)| {
                code.lines()
                    .map(code_of)
                    .filter(|l| l.contains(needle))
                    .count()
            })
            .sum()
    };
    assert_eq!(
        count("WebPkiClientVerifier::builder"),
        1,
        "exactly one call builds the client verifier"
    );
    assert_eq!(
        count("with_client_cert_verifier("),
        1,
        "one place installs it"
    );
    assert_eq!(count(".with_crls("), 1, "the CRLs go to that one verifier");
    for needle in [
        "impl ClientCertVerifier",
        "ClientCertVerifier for",
        "dyn ClientCertVerifier",
    ] {
        assert_eq!(count(needle), 0, "a verifier of the crate's own: {needle}");
    }
}

#[test]
fn the_key_file_checks_are_in_the_module_that_reads_the_key() {
    let sources = rust_sources();
    let (_, tls) = sources.iter().find(|(rel, _)| rel == "src/tls.rs").unwrap();
    let read_key = function(tls, "fn read_key");
    // opened without following a link, checked on the open file, mode and owner, and only then read
    let (open, mode, owner, read) = (
        read_key.find("O_NOFOLLOW").expect("no symbolic link"),
        read_key.find("mode & 0o077").expect("the mode check"),
        read_key.find("euid()").expect("the owner check"),
        read_key.find("read_to_end").expect("the read"),
    );
    assert!(
        open < mode && owner < read && mode < read,
        "checks come before the read"
    );
    assert!(
        !read_key.contains("{bytes") && !read_key.contains("println"),
        "no key bytes in a message"
    );
}

#[test]
fn the_documents_state_the_servers_own_claim() {
    // The claim for `klatch-server` is the one this audit backs; the shorter claim for `klatch` ("no telemetry, no
    // outbound calls") is not reused for it (#1236).
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let claim = "klatch-server opens loopback listeners only, plus one mutual TLS listener that exists only when you name your own certificates, starts no program and makes no outbound connection";
    for doc in ["SECURITY.md", "README.md"] {
        // the documents wrap their lines: compare with the whitespace folded
        let text = std::fs::read_to_string(root.join(doc)).unwrap();
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(text.contains(claim), "{doc} does not state: {claim}");
    }
}
