//! `klatch-server` starts no program and makes no outbound connection. Since phase 2 (#1058) it may listen on TCP, but
//! only in `src/loopback.rs`, which refuses every address but 127.0.0.1 and ::1. Since #102 it may also listen on a Unix
//! domain socket and ask the kernel who is on the other end, but only in `src/daemon.rs`. This is an audit by inspection, like the one for
//! the `klatch` binary (klatch/tests/no_telemetry.rs): it checks what the crate could do (its dependencies and its
//! source), not what one run did. The rules: socket types and `std::net` only in the loopback module (and its tests, for
//! a client connecting to their own listener); no `TcpStream::connect` anywhere else; exactly one `TcpListener::bind`,
//! after the loopback check; the Unix socket types and the peer-credential calls only in the daemon module, which has
//! one `UnixListener::bind` and one `UnixStream::connect` (the probe of its own socket path that tells a live daemon from
//! a dead one's file); no UDP, datagram socket, name resolution or HTTP client anywhere; no `Command`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn rust_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap()
                .display()
                .to_string();
            (rel, std::fs::read_to_string(&p).unwrap())
        })
        .collect()
}

/// The one source file that may name the socket types and open a listener (#1058): the loopback listener, which refuses
/// every address but 127.0.0.1 and ::1. Phases 3 and 4 reuse it; no other file gets this allowance.
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

/// The code of a line: what is left of it before a `//` comment, and not the text of a string.
fn code_of(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

#[test]
fn the_source_opens_no_socket_outside_the_loopback_module_and_starts_no_program() {
    // Socket types and the network module: only the loopback module (and, for a client, its tests) may name them.
    const SOCKETS: &[&str] = &["TcpStream", "TcpListener", "std::net", "SocketAddr"];
    // Never anywhere, not even in the loopback module: datagram and unix sockets, name resolution, an HTTP client, an
    // all-interfaces address.
    const NEVER: &[&str] = &[
        "UdpSocket",
        "UnixDatagram",
        "ToSocketAddrs",
        "ureq",
        "reqwest",
        "hyper",
        "UNSPECIFIED",
    ];
    // Programs this crate may start by literal name: none. (The `klatch` library starts git, ps and the keystores for
    // the member setup; that is the library's own audit.)
    const PROGRAMS: &[&str] = &[];
    let mut problems = Vec::new();
    let sources = rust_sources();
    assert!(!sources.is_empty());
    for (rel, text) in &sources {
        for (n, line) in text.lines().enumerate() {
            let code = code_of(line);
            let at = format!("{rel}:{}", n + 1);
            let may_name_sockets =
                LOOPBACK_ONLY.contains(&rel.as_str()) || TEST_CLIENT.contains(&rel.as_str());
            // `std::net::Shutdown` is what `UnixStream::shutdown` takes: not a network type
            let net_code = code.replace("std::net::Shutdown", "");
            if !may_name_sockets {
                problems.extend(
                    SOCKETS
                        .iter()
                        .filter(|s| net_code.contains(*s))
                        .map(|s| format!("{at}: uses {s}")),
                );
            } else if DAEMON_ONLY.contains(&rel.as_str()) {
                problems.extend(
                    SOCKETS
                        .iter()
                        .filter(|s| net_code.contains(*s))
                        .map(|s| format!("{at}: the daemon module names the network type {s}")),
                );
            }
            if !DAEMON_ONLY.contains(&rel.as_str()) {
                problems.extend(
                    DAEMON_NAMES
                        .iter()
                        .filter(|s| code.contains(*s))
                        .map(|s| format!("{at}: uses {s} outside the daemon module")),
                );
            }
            problems.extend(
                NEVER
                    .iter()
                    .filter(|s| code.contains(*s))
                    .map(|s| format!("{at}: uses {s}")),
            );
            // No outbound connection: a connect is allowed only in the loopback listener's own tests.
            if (code.contains("TcpStream::connect")
                || code.contains("connect_timeout(")
                || (code.contains("UnixStream::connect(") && !DAEMON_ONLY.contains(&rel.as_str())))
                && !TEST_CLIENT.contains(&rel.as_str())
            {
                problems.push(format!("{at}: opens an outbound connection"));
            }
            // No listener but the one in `bind_loopback`.
            if code.contains("TcpListener::bind") && !LOOPBACK_ONLY.contains(&rel.as_str()) {
                problems.push(format!(
                    "{at}: binds a listener outside the loopback module"
                ));
            }
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
        }
    }
    assert!(
        problems.is_empty(),
        "outbound capability in the source:\n{}",
        problems.join("\n")
    );
}

#[test]
fn there_is_exactly_one_bind_and_the_loopback_check_runs_before_it() {
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
    let start = text.find("pub fn bind_loopback").expect("bind_loopback");
    let body = &text[start..];
    let check = body
        .find("check_loopback(")
        .expect("the address is checked");
    let bind = body.find("TcpListener::bind").unwrap();
    assert!(check < bind, "the loopback check comes before the bind");
    // the only addresses the module accepts are spelled out in one function, and it does not use `is_loopback`
    let allowed = text.split("fn is_allowed_ip").nth(1).unwrap();
    let allowed = allowed.split("\n}\n").next().unwrap();
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
    assert_eq!(count("UnixStream::connect("), 1, "one probe, of the daemon's own path");
    for forbidden in ["TcpListener", "TcpStream", "TcpSocket", "0.0.0.0", "SocketAddr"] {
        assert_eq!(count(forbidden), 0, "{forbidden}");
    }
    // the order in the accept loop: who the peer is, the uid check, and only then a thread for the connection
    let accept = text.find("fn run(").expect("run");
    let body = &text[accept..];
    let (cred, uid, spawn) = (
        body.find("peer_cred(&stream)").expect("peer credentials"),
        body.find("local_evidence(peer").expect("the uid check"),
        body.find("scope.spawn(").expect("the connection thread"),
    );
    assert!(cred < uid && uid < spawn, "the uid check comes before anything is served");
    // the socket is born 0600 (umask) and made 0600, and the directory is checked before the path is cleared
    assert!(text.contains("umask(0o177)") && text.contains("from_mode(0o600)"));
    let dir_check = body.find("checked_directory(").unwrap();
    let clear = body.find("clear_path(").unwrap();
    assert!(dir_check < clear);
}

#[test]
fn no_http_or_networking_crate_is_in_the_dependency_tree() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "tree",
            "-p",
            "klatch-server",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8(out.stdout).unwrap();
    let deps: Vec<&str> = tree
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
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
        "tungstenite",
        "tokio-tungstenite",
        "tokio",
        "async-std",
        "smol",
        "mio",
        "socket2",
        "rustls",
        "native-tls",
        "openssl",
        "quinn",
        "hickory-resolver",
        "trust-dns-resolver",
    ];
    for n in NETWORK {
        assert!(
            !deps.contains(n),
            "a networking crate is in klatch-server's dependencies: {n}"
        );
    }
    assert!(deps.len() > 10, "the tree looks empty: {deps:?}");
}

#[test]
fn the_crate_is_not_published() {
    let toml =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(
        toml.lines().any(|l| l.trim() == "publish = false"),
        "klatch-server stays unpublished until the zero-trust gate"
    );
}
