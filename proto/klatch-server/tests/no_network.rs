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
//!
//! A tripwire, not a proof (#1787): the scans read source text with a tokenizer (`tests/net_scan`), so they stop the
//! obvious and the cheap evasions, and they cannot see what macros build from names, `crate::` or re-exported paths,
//! `cfg_attr`, `include!(concat!(..))`, a build script that emits `cargo:rustc-cfg=test`, or symbols resolved at run time.
//! The real boundary is the dependency-tree tests (no networking crate in the tree or in `Cargo.lock`), the exact-line
//! allowlists with their reasons, and review. `net_scan/mod.rs` says the same at more length.

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
const DAEMON_ONLY: &[&str] = &["src/daemon.rs", "src/daemon_peer.rs", "src/daemon_tests.rs"];

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

/// The socket calls and the program starts of a file, read from tokens (#1783): `TcpStream :: connect`, `<TcpStream>::connect`
/// and the `loopback::Stream` alias of it, `connect_timeout`, the same for `bind` on `TcpListener` and its alias
/// `Listener`, whatever the spacing and the lines, and every way to start a program.
fn token_problems(rel: &str, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut add = |line: usize, what: &str| problems.push(format!("{rel}:{line}: {what}"));
    // a connect is allowed only in the loopback listener's own tests; the daemon probes its own Unix path
    if !is_in(rel, TEST_CLIENT) {
        for method in ["connect", "connect_timeout"] {
            for line in net_scan::assoc_calls_aliased(text, TCP_STREAMS, method) {
                add(line, "opens an outbound connection");
            }
        }
        if !is_in(rel, DAEMON_ONLY) {
            for line in net_scan::assoc_calls_aliased(text, &["UnixStream"], "connect") {
                add(line, "opens an outbound connection");
            }
        }
    }
    // no listener but the one in `bind_loopback`
    if !is_in(rel, LOOPBACK_ONLY) {
        for line in net_scan::assoc_calls_aliased(text, TCP_LISTENERS, "bind") {
            add(line, "binds a listener outside the loopback module");
        }
    }
    // no program is started, by any route
    for u in net_scan::command_uses(text) {
        match u {
            net_scan::CommandUse::Call {
                line,
                program: Some(p),
                ..
            } if !PROGRAMS.contains(&p.as_str()) => add(line, &format!("runs {p}")),
            net_scan::CommandUse::Call {
                line,
                program: None,
                ..
            } => add(line, "runs a program chosen at run time"),
            net_scan::CommandUse::Indirect { line, what } => add(line, &what),
            net_scan::CommandUse::Call { .. } => {}
        }
    }
    for (line, name) in net_scan::ids(text) {
        if name == "Command" {
            add(line, "names the process Command type");
        }
    }
    problems.sort();
    problems.dedup();
    problems
}

/// The types a TCP connection is made through: the std type and the `loopback::Stream` alias of it (`pub type Stream =
/// TcpStream`).
const TCP_STREAMS: &[&str] = &["TcpStream", "Stream"];

/// The types a TCP listener is made through: the std type and the `loopback::Listener` alias of it.
const TCP_LISTENERS: &[&str] = &["TcpListener", "Listener"];

/// Everything one line of source may not do.
fn line_problems(rel: &str, code: &str, at: &str) -> Vec<String> {
    let mut problems = socket_problems(rel, code, at);
    problems.extend(name_problems(rel, code, at));
    problems
}

#[test]
fn the_source_opens_no_socket_outside_the_loopback_module_and_starts_no_program() {
    let sources = rust_sources();
    assert!(!sources.is_empty());
    let problems: Vec<String> = sources
        .iter()
        .flat_map(|(rel, text)| {
            let by_line = text.lines().enumerate().flat_map(move |(n, line)| {
                line_problems(rel, code_of(line), &format!("{rel}:{}", n + 1))
            });
            by_line.chain(token_problems(rel, text))
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
            net_scan::assoc_calls_aliased(text, TCP_LISTENERS, "bind")
                .into_iter()
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
    let code_text = code.join("\n");
    assert_eq!(
        net_scan::assoc_calls_aliased(&code_text, &["UnixListener"], "bind").len(),
        1,
        "one listener"
    );
    assert_eq!(
        net_scan::assoc_calls_aliased(&code_text, &["UnixStream"], "connect").len(),
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
        "nix",
        "rustix",
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
const TLS_MODULES: &[&str] = &[
    "src/tls.rs",
    "src/tls_bridge.rs",
    "src/tls_crl.rs",
    "src/crl_sig.rs",
    // the tests of tls_crl.rs, split out for the line cap (#1689): they build a rustls error to classify
    "src/tls_crl_log_tests.rs",
];

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

// ---- OS-level network calls (#1781, #1783): the same scanner as the klatch crate's guard --------------------------------

/// What the scan may keep in this crate: an exact file and an exact line, and why. Nothing else is allowed. The daemon
/// listens on a Unix socket and the loopback listener on a loopback TCP port by design, so the Rust types that do that
/// are governed by the rules above (`SOCKETS`, `DAEMON_NAMES`); what is allowed here is the `libc` and module-path use
/// the type rules cannot see.
struct NetAllowance {
    file: &'static str,
    line: &'static str,
    /// How many times the line occurs in the file (the Linux and the macOS branch may share one).
    count: usize,
    reason: &'static str,
}

const PEER: &str = "the daemon asks the kernel who is at the other end of a connection it accepted on its own Unix socket \
                    (SO_PEERCRED on Linux, getpeereid and LOCAL_PEERPID on macOS), so it can refuse a peer of another uid \
                    before it reads a byte; the call sends and connects nothing, and the line is part of that one function";

#[rustfmt::skip]
const NET_ALLOWED: &[NetAllowance] = &[
    NetAllowance { file: "src/daemon.rs", line: "let a: libc::sockaddr_un = unsafe { std::mem::zeroed() };", count: 1, reason: "a zeroed sockaddr_un read only for the length of its sun_path array (the longest socket path); no socket is made and nothing is called" },
    NetAllowance { file: "src/daemon_peer.rs", line: "let mut c: libc::ucred = unsafe { std::mem::zeroed() };", count: 1, reason: PEER },
    NetAllowance { file: "src/daemon_peer.rs", line: "let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;", count: 1, reason: PEER },
    NetAllowance { file: "src/daemon_peer.rs", line: "libc::getsockopt(", count: 2, reason: PEER },
    NetAllowance { file: "src/daemon_peer.rs", line: "libc::SOL_SOCKET,", count: 1, reason: PEER },
    NetAllowance { file: "src/daemon_peer.rs", line: "libc::SO_PEERCRED,", count: 1, reason: PEER },
    NetAllowance { file: "src/daemon_peer.rs", line: "(&mut c as *mut libc::ucred).cast(),", count: 1, reason: PEER },
    NetAllowance { file: "src/daemon_peer.rs", line: "if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {", count: 1, reason: PEER },
    NetAllowance { file: "src/daemon_peer.rs", line: "let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;", count: 1, reason: PEER },
    NetAllowance { file: "src/loopback.rs", line: "if libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) != 0 {", count: 1, reason: "connected_pair: two connected local sockets inside the process, the connection type the TLS listener hands the HTTP handlers while it carries the other end's bytes through TLS; unnamed, no address, nothing listens and nothing leaves the process" },
    NetAllowance { file: "tests/client_socket.rs", line: "if said && std::os::unix::net::UnixStream::connect(self.sock()).is_ok() {", count: 1, reason: "an integration test of the Rust client's socket connect (#1095): it probes the path of the daemon it started in its own temporary directory; a local Unix socket, a test crate" },
    NetAllowance { file: "tests/daemon.rs", line: "use std::os::unix::net::UnixStream;", count: 1, reason: "an integration test of the daemon: it connects to the daemon it started on a Unix socket in its own temporary directory; a test crate" },
    NetAllowance { file: "tests/daemon.rs", line: "drop(std::os::unix::net::UnixListener::bind(dir.sock()).unwrap());", count: 1, reason: "the daemon test that leaves a dead socket file behind: it binds and drops a Unix listener in its own temporary directory to make the stale file the daemon must replace" },
    NetAllowance { file: "tests/daemon.rs", line: "let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();", count: 1, reason: "a daemon test that needs a peer which hangs up: a Unix listener in its own temporary directory that accepts one connection and closes it, to test the client against it" },
];

fn net_allowance_for(rel: &str, raw_line: &str) -> Option<&'static NetAllowance> {
    NET_ALLOWED
        .iter()
        .find(|a| a.file == rel && a.line == raw_line.trim())
}

/// Files only tests may hold, with the one file that declares each (`#[cfg(test)] #[path = "..."] mod ..;`).
const TEST_ONLY_FILES: &[(&str, &str)] = &[("src/daemon.rs", "src/daemon_tests.rs")];

/// The test-only files a `#[path]` or `include!` must not pull into another file.
fn protected_test_files() -> Vec<&'static str> {
    TEST_CLIENT
        .iter()
        .copied()
        .chain(TEST_ONLY_FILES.iter().map(|(_, f)| *f))
        .collect()
}

/// Whether `text` declares `#[path = "target"] mod ..;` exactly once, directly under `#[cfg(test)]`, read from tokens: a
/// comment cannot fake the attribute and a second, unconditional declaration of the same file is not accepted.
fn declared_under_cfg_test(text: &str, target: &str) -> bool {
    let decls: Vec<_> = net_scan::path_decls(text)
        .into_iter()
        .filter(|(t, _)| t == target)
        .collect();
    decls.len() == 1 && decls[0].1.iter().any(|a| a == "cfg(test)")
}

/// The OS-level network findings of one file that no allowance covers. `std::os::unix::net` is the daemon's own and is
/// only allowed in the daemon files (`DAEMON_ONLY`); everything else needs an exact entry.
fn net_problems(rel: &str, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    // a `#[path]` or `include!` that reaches a file that only tests may hold, except the one declaration by its parent
    let paths = net_scan::path_targets(text)
        .into_iter()
        .filter_map(|(line, target)| {
            let resolved = net_scan::resolve_relative(rel, &target);
            let legit = TEST_ONLY_FILES.iter().any(|(parent, file)| {
                *parent == rel && *file == resolved && declared_under_cfg_test(text, &target)
            });
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            if !resolved.ends_with(".rs") {
                return Some(format!(
                    "{rel}:{line}: a #[path] or include! of {resolved}, which is not a Rust file"
                ));
            }
            if !root.join(&resolved).is_file() {
                return Some(format!(
                    "{rel}:{line}: a #[path] or include! of {resolved}, which does not exist"
                ));
            }
            (protected_test_files().contains(&resolved.as_str()) && !legit)
                .then(|| format!("{rel}:{line}: a #[path] or include! of {resolved}"))
        });
    net_scan::findings(text)
        .into_iter()
        .filter(|f| !(f.what == "std::os::unix::net" && is_in(rel, DAEMON_ONLY)))
        .filter_map(|f| {
            let raw = lines.get(f.line.wrapping_sub(1)).copied().unwrap_or("");
            net_allowance_for(rel, raw)
                .is_none()
                .then(|| format!("{rel}:{}: {}: {}", f.line, f.what, raw.trim()))
        })
        .chain(paths)
        .collect()
}

#[test]
fn the_source_and_tests_make_no_os_level_network_call_but_the_allowed_ones() {
    let problems: Vec<String> = all_sources()
        .iter()
        .flat_map(|(rel, text)| net_problems(rel, text))
        .collect();
    assert!(
        problems.is_empty(),
        "an OS-level network call or name (an exact NET_ALLOWED entry with a reason is the only way to keep one):\n{}",
        problems.join("\n")
    );
}

/// Every allowance names a file that exists, its line occurs there exactly `count` times, and it has a reason: a stale or
/// widened entry fails.
#[test]
fn every_network_allowance_matches_its_real_lines() {
    let sources = all_sources();
    for a in NET_ALLOWED {
        let (_, text) = sources
            .iter()
            .find(|(rel, _)| rel == a.file)
            .unwrap_or_else(|| panic!("{} is allowed but does not exist", a.file));
        let hits = text.lines().filter(|l| l.trim() == a.line).count();
        assert_eq!(hits, a.count, "{}: the allowed line {:?}", a.file, a.line);
        assert!(a.reason.len() > 40, "{}: a reason is needed", a.file);
        assert!(
            a.file.starts_with("src/") || a.file.starts_with("tests/"),
            "{}",
            a.file
        );
    }
}

/// The allowance is a list of exact lines of exact files; growing it is a deliberate act.
#[test]
fn the_network_allowance_covers_only_its_own_lines() {
    assert_eq!(
        NET_ALLOWED.len(),
        14,
        "a new allowance needs a review and this number changed"
    );
    for a in NET_ALLOWED {
        assert!(
            net_allowance_for(a.file, &format!("   {}  ", a.line)).is_some(),
            "{}",
            a.line
        );
        for other in [
            "src/other.rs",
            "tests/other.rs",
            "src/daemon_peers.rs",
            "x/src/daemon_peer.rs",
        ] {
            assert!(
                net_allowance_for(other, a.line).is_none(),
                "{other}: {}",
                a.line
            );
        }
        assert!(
            net_allowance_for(a.file, &format!("{} libc::connect(1, 2, 3);", a.line)).is_none()
        );
        assert!(net_allowance_for(a.file, &format!("{}x", a.line)).is_none());
    }
    // a call of the family that is not on an allowed line is a problem, in the daemon files too
    for rel in [
        "src/daemon_peer.rs",
        "src/loopback.rs",
        "src/daemon.rs",
        "tests/daemon.rs",
    ] {
        for call in [
            "fn f() { unsafe { libc::connect(1, 2, 3) }; }",
            "fn f() { unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) }; }",
            "fn f() { unsafe { libc::bind(1, 2, 3) }; }",
            "fn f() { unsafe { libc::listen(1, 2) }; }",
            "fn f() { unsafe { libc::syscall(41) }; }",
        ] {
            assert!(!net_problems(rel, call).is_empty(), "{rel}: {call}");
        }
    }
    // std::os::unix::net is the daemon's own: allowed in its files, a problem anywhere else without an exact entry
    let net = "use std::os::unix::net::UnixListener;\n";
    for rel in DAEMON_ONLY {
        assert!(net_problems(rel, net).is_empty(), "{rel}");
    }
    assert!(!net_problems("src/wire_http.rs", net).is_empty());
}

/// The guard reads `tests` as well as `src` (#1783).
#[test]
fn the_guard_reads_tests_as_well_as_src() {
    let rels: Vec<String> = all_sources().into_iter().map(|(r, _)| r).collect();
    for want in [
        "src/daemon.rs",
        "tests/no_network.rs",
        "tests/daemon.rs",
        "tests/scan_common/mod.rs",
    ] {
        assert!(rels.iter().any(|r| r == want), "{want} is not read");
    }
}

/// Every way past the first version of the guards is reported in a file with no allowance.
#[test]
fn every_evasion_of_the_first_guard_is_flagged_here_too() {
    for (what, file, src) in net_scan::evasions() {
        assert!(
            !net_problems(file, &src).is_empty(),
            "not flagged: {what}:\n{src}"
        );
    }
}

/// A `#[path]` or `include!` that reaches a file only tests may hold is reported.
#[test]
fn a_path_or_include_of_a_test_only_file_is_flagged_here_too() {
    let protected = protected_test_files();
    assert!(protected.len() >= 2, "{protected:?}");
    for target in protected {
        for (what, file, src) in net_scan::path_evasions(target) {
            assert!(
                !net_problems(&file, &src).is_empty(),
                "not flagged: {what} of {target}:\n{src}"
            );
        }
    }
}

/// Words in strings and comments, and libc names that make no network use, are not reported.
#[test]
fn comments_strings_and_ordinary_libc_use_are_not_flagged_here() {
    for (what, src) in net_scan::clean_sources() {
        assert!(
            net_problems("src/some/new_file.rs", &src).is_empty(),
            "flagged: {what}"
        );
    }
}

/// The old line scans read the source with its comments removed by the tokenizer: a `//` in a string no longer hides the
/// rest of its line from them (#1783).
#[test]
fn the_line_scans_see_past_a_double_slash_in_a_string() {
    let text = net_scan::strip_comments("let u = \"https://x\"; let l = TcpListener::bind(a);\n");
    let problems: Vec<String> = text
        .lines()
        .flat_map(|l| line_problems("src/some/new_file.rs", code_of(l), "at"))
        .collect();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("TcpListener") || p.contains("listener")),
        "{problems:?}"
    );
}

/// A throwaway crate directory under cargo's temporary directory, with these files.
fn temp_crate(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("server-guard-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    root
}

/// Only the build output is skipped, never a directory named `target` under `src` or `tests` (#1783).
#[test]
fn a_directory_named_target_under_src_is_read() {
    let evil = "fn f() { unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) }; }\n";
    let root = temp_crate(
        "walk",
        &[
            ("src/lib.rs", "// ok\n"),
            ("src/target/mod.rs", evil),
            ("src/artifacts/mod.rs", evil),
            ("tests/corpus/t.rs", evil),
            ("target/b.rs", evil),
        ],
    );
    let files = all_sources_in(&root);
    let rels: Vec<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
    for want in [
        "src/lib.rs",
        "src/target/mod.rs",
        "src/artifacts/mod.rs",
        "tests/corpus/t.rs",
    ] {
        assert!(rels.contains(&want), "{want}: {rels:?}");
    }
    assert!(!rels.contains(&"target/b.rs"), "{rels:?}");
    for (rel, text) in &files {
        if rel.contains("target/mod") {
            assert!(!net_problems(rel, text).is_empty(), "{rel}");
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// The sockets the loopback module makes by an alias, and every spelling of a connect or a bind, are found from tokens
/// (#1783): `pub type Stream = TcpStream;` and `pub type Listener = TcpListener;` are the way in.
#[test]
fn a_connect_or_a_bind_is_found_by_tokens_through_the_aliases_and_any_spacing() {
    for (what, rel, src) in [
        (
            "the Stream alias from another file",
            "src/wire_http.rs",
            "fn f() { crate::loopback::Stream::connect(\"evil:443\"); }\n",
        ),
        (
            "the Listener alias from another file",
            "src/wire_http.rs",
            "fn f() { crate::loopback::Listener::bind(\"0.0.0.0:80\"); }\n",
        ),
        (
            "spaces around ::",
            "src/loopback.rs",
            "fn f() { TcpStream :: connect(a); }\n",
        ),
        (
            "UFCS connect",
            "src/loopback.rs",
            "fn f() { <TcpStream>::connect(a); }\n",
        ),
        (
            "a split path",
            "src/wire_http.rs",
            "fn f() { TcpStream\n    ::\n    connect(a); }\n",
        ),
        (
            "connect_timeout",
            "src/wire_http.rs",
            "fn f() { TcpStream::connect_timeout(&a, t); }\n",
        ),
        (
            "a bind from another file",
            "src/wire_http.rs",
            "fn f() { TcpListener :: bind(a); }\n",
        ),
        (
            "a UFCS bind from another file",
            "src/daemon.rs",
            "fn f() { <TcpListener>::bind(a); }\n",
        ),
        (
            "a Unix connect outside the daemon",
            "src/wire_http.rs",
            "fn f() { UnixStream :: connect(p); }\n",
        ),
    ] {
        assert!(!token_problems(rel, src).is_empty(), "not flagged: {what}");
    }
    // loopback.rs may bind, but the count of binds in it is read from tokens, so a second one in any spelling is seen
    let count = |src: &str| net_scan::assoc_calls_aliased(src, TCP_LISTENERS, "bind").len();
    assert_eq!(
        count(
            "TcpListener :: bind(a); <TcpListener>::bind(b); Listener::bind(c); loopback::Listener\n::\nbind(d);"
        ),
        4
    );
    assert_eq!(
        count("// TcpListener::bind(a)\nlet s = \"TcpListener::bind\";"),
        0
    );
    // the real files pass: the one bind is in loopback.rs
    let bind_in_loopback =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/loopback.rs"))
            .unwrap();
    assert_eq!(count(&net_scan::strip_comments(&bind_in_loopback)), 1);
}

/// Every way to start a program is found by identifier (#1783); the server starts none.
#[test]
fn a_program_start_is_found_by_identifier_in_every_form() {
    for (what, src) in [
        (
            "a call",
            "fn f() { std::process::Command::new(\"curl\"); }\n",
        ),
        ("a type alias", "type C = std::process::Command;\n"),
        (
            "map with Command::new",
            "use std::process::{Stdio, Command};\nfn f() { v.map(Command::new); }\n",
        ),
        ("a UFCS call", "fn f() { <Command>::new(\"curl\"); }\n"),
        ("a path value", "fn f() { let mk = Command::new; }\n"),
        ("only an import", "use std::process::{Stdio, Command};\n"),
    ] {
        assert!(
            !token_problems("src/some/new_file.rs", src).is_empty(),
            "not flagged: {what}"
        );
    }
    assert!(
        token_problems(
            "src/some/new_file.rs",
            "use std::process::ExitCode;\nfn f() { let c = \"Command::new\"; }\n"
        )
        .is_empty()
    );
}

/// A declaration is read from tokens: a comment cannot fake `#[cfg(test)]`, and a second declaration of the same file is
/// not accepted (#1783).
#[test]
fn the_declaration_of_the_daemon_tests_is_read_from_tokens_and_must_be_the_only_one() {
    let real = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/daemon.rs"))
        .unwrap();
    assert!(
        net_problems("src/daemon.rs", &real)
            .iter()
            .all(|p| !p.contains("daemon_tests")),
        "the real declaration is fine"
    );
    for (what, extra) in [
        (
            "a second unconditional declaration",
            "\n#[path = \"daemon_tests.rs\"]\nmod shipped;\n",
        ),
        (
            "a second conditional declaration",
            "\n#[cfg(test)]\n#[path = \"daemon_tests.rs\"]\nmod shipped;\n",
        ),
    ] {
        let text = format!("{real}{extra}");
        assert!(
            net_problems("src/daemon.rs", &text)
                .iter()
                .any(|p| p.contains("daemon_tests")),
            "not flagged: {what}"
        );
    }
    let faked = "/*\n#[cfg(test)]\n#[ */\n#[path = \"daemon_tests.rs\"]\nmod tests;\n";
    assert!(!declared_under_cfg_test(faked, "daemon_tests.rs"));
    assert!(declared_under_cfg_test(
        "#[ cfg ( test ) ]\n#[path = \"daemon_tests.rs\"]\nmod tests;\n",
        "daemon_tests.rs"
    ));
}

/// A `#[path]` is read whatever its extension, and a missing target is a finding (#1783).
#[test]
fn a_path_is_read_whatever_its_extension_and_a_missing_target_is_a_finding() {
    for (what, rel, src) in [
        (
            "a non-Rust target",
            "src/some/new_file.rs",
            "#[path = \"sock.inc\"]\nmod sock;\n",
        ),
        (
            "a non-Rust target that exists",
            "src/some/new_file.rs",
            "#[path = \"../../Cargo.toml\"]\nmod sock;\n",
        ),
        (
            "an include! of a non-Rust file",
            "src/some/new_file.rs",
            "include!(\"sock.inc\");\n",
        ),
        (
            "a missing Rust target",
            "src/some/new_file.rs",
            "#[path = \"nowhere.rs\"]\nmod nowhere;\n",
        ),
    ] {
        assert!(!net_problems(rel, src).is_empty(), "not flagged: {what}");
    }
}

/// The shared tokenizer matches rustc on the lexer edge cases, keeps step on every file of this crate, and so the
/// `ring_paths` reading of `scan_common` no longer trips on `'\\''` or a C string (#1783).
#[test]
fn the_tokenizer_matches_rustc_and_ring_paths_uses_it() {
    net_scan::check_lexer_cases();
    for (rel, text) in all_sources() {
        assert!(
            net_scan::brackets_balance(&text),
            "the brackets of {rel} do not balance in the tokens"
        );
    }
    assert_eq!(
        ring_paths("let q = '\\''; let d = \"\"; ring::digest::SHA1;"),
        ["digest"]
    );
    assert_eq!(
        ring_paths("quotes! { '\\'', '\"' } ring::hmac::Key;"),
        ["hmac"]
    );
    assert_eq!(
        ring_paths("let _a = cr\"\\\"; let _b = c\"\"; ring::rand::SystemRandom;"),
        ["rand"]
    );
}

/// A connect or a bind through any name the file gives a socket type is judged like the original (#1783): `use .. as`,
/// `type X = ..`, an alias of an alias, for the loopback aliases and the std types.
#[test]
fn a_connect_or_a_bind_through_any_alias_name_is_judged_like_the_original() {
    for (what, rel, src) in [
        (
            "a use rename of the Stream alias",
            "src/wire_http.rs",
            "use crate::loopback::Stream as S;\nfn f() { S::connect(\"evil:443\"); }\n",
        ),
        (
            "a type alias of the Listener alias",
            "src/wire_http.rs",
            "type L = crate::loopback::Listener;\nfn f() { L::bind(\"0.0.0.0:80\"); }\n",
        ),
        (
            "an alias of an alias",
            "src/wire_http.rs",
            "use crate::loopback::Stream as S;\ntype T = S;\nfn f() { T::connect(a); }\n",
        ),
        (
            "a type alias of TcpStream",
            "src/wire_http.rs",
            "type C = std::net::TcpStream;\nfn f() { C::connect(a); }\n",
        ),
        (
            "a use rename of TcpListener",
            "src/wire_http.rs",
            "use std::net::TcpListener as T;\nfn f() { T::bind(a); }\n",
        ),
        (
            "a use rename inside braces",
            "src/wire_http.rs",
            "use std::net::{TcpStream as C, IpAddr};\nfn f() { C::connect(a); }\n",
        ),
        (
            "a Unix alias",
            "src/wire_http.rs",
            "use std::os::unix::net::UnixStream as U;\nfn f() { U::connect(p); }\n",
        ),
        (
            "a UFCS call through an alias",
            "src/wire_http.rs",
            "type L = crate::loopback::Listener;\nfn f() { <L>::bind(a); }\n",
        ),
    ] {
        let found = token_problems(rel, src);
        assert!(!found.is_empty(), "not flagged: {what}");
    }
    // the bind count sees the alias names too
    let count = |src: &str| net_scan::assoc_calls_aliased(src, TCP_LISTENERS, "bind").len();
    assert_eq!(
        count("type L = Listener; fn f() { L::bind(a); Listener::bind(b); }"),
        2
    );
    assert_eq!(
        count("use std::net::TcpListener as T; type U = T; fn f() { U::bind(a); }"),
        1
    );
    // an unrelated alias is not a socket
    assert!(
        token_problems(
            "src/wire_http.rs",
            "type M = std::collections::HashMap<u8, u8>;\nfn f() { M::new(); }\n"
        )
        .is_empty()
    );
}

/// An invisible mark that rustc reads as white space, between path segments, hides nothing from the guard (#1783).
#[test]
fn a_whitespace_mark_between_path_segments_hides_no_socket_call() {
    for (what, src) in [
        (
            "a mark before ::",
            "fn f() { Stream\u{200e}::connect(\"evil:443\"); }\n",
        ),
        (
            "a mark after ::",
            "fn f() { Stream::\u{200f}connect(\"evil:443\"); }\n",
        ),
        (
            "a bind",
            "fn f() { Listener\u{200e}::bind(\"0.0.0.0:80\"); }\n",
        ),
        ("a Command", "fn f() { Command\u{200e}::new(\"curl\"); }\n"),
        (
            "a next-line character",
            "fn f() { TcpStream\u{85}::connect(a); }\n",
        ),
        (
            "a shebang line holding a quote",
            "#!/x\"\nfn f() { TcpStream::connect(a); }\n",
        ),
    ] {
        assert!(
            !token_problems("src/wire_http.rs", src).is_empty(),
            "not flagged: {what}"
        );
    }
}

/// Source that is not valid Rust (an attribute never closed) is a finding, not a panic of the scanner.
#[test]
fn an_attribute_that_is_never_closed_is_a_finding_not_a_panic() {
    net_scan::check_unclosed_attributes();
}
