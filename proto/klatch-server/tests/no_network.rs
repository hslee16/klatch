//! `klatch-server` in phase 1 (#1057, epic #1056) opens no network socket and starts no program. This is an audit by
//! inspection, like the one for the `klatch` binary (klatch/tests/no_telemetry.rs): it checks what the crate could do
//! (its dependencies and its source), not what one run did. When a later phase opens a loopback listener, its file is
//! named in `LOOPBACK_ONLY` and the rule for the address is tested beside it; until then the list is empty.

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

/// Source files allowed to open a (loopback) listener. Empty in phase 1.
const LOOPBACK_ONLY: &[&str] = &[];

/// The code of a line: what is left of it before a `//` comment, and not the text of a string.
fn code_of(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

#[test]
fn the_source_opens_no_socket_and_starts_no_program() {
    const SOCKETS: &[&str] = &[
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "UnixStream",
        "UnixListener",
        "UnixDatagram",
        "std::net",
        "ToSocketAddrs",
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
            if !LOOPBACK_ONLY.contains(&rel.as_str()) {
                problems.extend(
                    SOCKETS
                        .iter()
                        .filter(|s| code.contains(*s))
                        .map(|s| format!("{at}: uses {s}")),
                );
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
