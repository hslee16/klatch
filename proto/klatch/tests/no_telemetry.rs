//! R24 (#80): no telemetry, no third-party calls, no unverified remote installer. This is an audit by inspection: it
//! checks what the binary could do (its dependencies and its source), not what it did on one run, and it cannot prove a
//! negative about code it does not read. What it pins is that nothing in klatch opens a network connection or runs a
//! network program, and that the documents never tell anyone to pipe a downloaded script into a shell.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn rust_sources() -> Vec<(PathBuf, String)> {
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
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect()
}

#[test]
fn no_networking_crate_is_in_the_dependency_tree() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "tree",
            "-p",
            "klatch",
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
        "reqwest-middleware",
    ];
    for n in NETWORK {
        assert!(
            !deps.contains(n),
            "a networking crate is in klatch's dependencies: {n}"
        );
    }
    assert!(deps.len() > 20, "the tree looks empty: {deps:?}");
}

#[test]
fn the_source_opens_no_socket_and_runs_only_the_programs_it_is_known_to_run() {
    const SOCKETS: &[&str] = [
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "UnixStream",
        "UnixListener",
        "UnixDatagram",
    ]
    .as_slice();
    // Programs klatch starts by name: git (to find a repository), ps (the process table), secret-tool and security
    // (the keystores), codex (only `--version` and `queue --help`, in `klatch doctor`: no login, no model call), claude (only `--version`, in `klatch doctor` and before `klatch wake`, #482: no login, no model call), and sh only in a unit test that waits for a fake keystore script to be executable. None of them is asked to talk to a network. The Rust client starts the binary the caller names.
    const PROGRAMS: &[&str] = &[
        "git",
        "ps",
        "secret-tool",
        "/usr/bin/security",
        "codex",
        "claude",
        "sh",
    ];
    let mut problems = Vec::new();
    for (path, text) in rust_sources() {
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .display()
            .to_string();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for s in SOCKETS {
                if code.contains(s) {
                    problems.push(format!("{rel}:{}: uses {s}", n + 1));
                }
            }
            if let Some(i) = code.find("Command::new(") {
                let arg = &code[i + "Command::new(".len()..];
                match arg.strip_prefix('"').and_then(|a| a.split('"').next()) {
                    Some(program) if PROGRAMS.contains(&program) => {}
                    Some(program) => problems.push(format!("{rel}:{}: runs {program}", n + 1)),
                    None if rel.ends_with("src/client.rs") => {}
                    // a unit test that runs its own test binary again, in a child process with its own environment
                    None if rel.ends_with("src/admin/command_tests.rs") => {}
                    // test-only modules that run the test binary again with a private environment, or a stand-in
                    // for the coding agent's process, so key and registry paths can be tested without the real ones
                    None if rel.ends_with("/survivor_tests.rs") => {}
                    None => problems.push(format!(
                        "{rel}:{}: runs a program chosen at run time",
                        n + 1
                    )),
                }
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
fn std_net_is_only_used_to_read_addresses() {
    for (path, text) in rust_sources() {
        for line in text.lines().filter(|l| l.contains("std::net")) {
            for item in [
                "TcpStream",
                "TcpListener",
                "UdpSocket",
                "ToSocketAddrs",
                "lookup_host",
            ] {
                assert!(!line.contains(item), "{}: {line}", path.display());
            }
        }
    }
}

#[test]
fn the_documents_never_tell_anyone_to_pipe_a_download_into_a_shell() {
    for doc in [
        "README.md",
        "UNINSTALL.md",
        "RELEASING.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "install/install.sh",
    ] {
        let text = std::fs::read_to_string(repo(doc)).unwrap();
        for (n, line) in text.lines().enumerate() {
            let piped = line.contains("| sh")
                || line.contains("| bash")
                || line.contains("|sh")
                || line.contains("|bash");
            let fetches = line.contains("curl") || line.contains("wget");
            assert!(
                !(piped && fetches),
                "{doc}:{}: pipes a download into a shell: {line}",
                n + 1
            );
        }
    }
}

#[test]
fn the_installer_talks_only_to_github() {
    let script = std::fs::read_to_string(repo("install/install.sh")).unwrap();
    for word in script.split(|c: char| c.is_whitespace() || "\"'()<>".contains(c)) {
        if let Some(rest) = word
            .strip_prefix("https://")
            .or_else(|| word.strip_prefix("http://"))
        {
            let host = rest.split(['/', ':']).next().unwrap();
            assert!(
                host == "github.com" || host == "api.github.com" || host.starts_with('$'),
                "install.sh mentions another host: {word}"
            );
            assert!(
                word.starts_with("https://"),
                "install.sh uses plain http: {word}"
            );
        }
    }
}
