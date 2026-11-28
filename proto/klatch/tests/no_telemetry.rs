//! R24 (#80): no telemetry, no third-party calls, no unverified remote installer. This is an audit by inspection: it
//! checks what the binary could do (its dependencies and its source), not what it did on one run, and it cannot prove a
//! negative about code it does not read. What it pins is that nothing in klatch opens a network connection or runs a
//! network program, and that the documents never tell anyone to pipe a downloaded script into a shell.

mod net_scan;

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// The path of a source file relative to the crate.
fn rel_of(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .display()
        .to_string()
}

/// Whether a line of a manifest is neither a comment nor blank.
fn is_content(t: &str) -> bool {
    !t.starts_with('#') && !t.is_empty()
}

/// The lines of the `[features]` section of this crate's Cargo.toml, without comments and blank lines.
fn feature_lines(toml: &str) -> Vec<&str> {
    let mut in_features = false;
    let mut out = Vec::new();
    for t in toml.lines().map(str::trim) {
        if t.starts_with('[') {
            in_features = t == "[features]";
        } else if in_features && is_content(t) {
            out.push(t);
        }
    }
    out
}

/// The body of each feature in this crate's Cargo.toml, by name (a feature may span several lines).
fn feature_bodies() -> std::collections::BTreeMap<String, String> {
    let toml = std::fs::read_to_string(manifest("Cargo.toml")).unwrap();
    let mut current = String::new();
    let mut bodies = std::collections::BTreeMap::<String, String>::new();
    for t in feature_lines(&toml) {
        let (name, rest) = t.split_once('=').unzip();
        if let Some(name) = name {
            current = name.trim().to_string();
        }
        bodies
            .entry(current.clone())
            .or_default()
            .push_str(rest.unwrap_or(t));
    }
    assert!(!bodies.is_empty(), "a features section");
    bodies
}

/// Whether `lines` has `decl` directly after the line `attr`.
fn declared_right_after(text: &str, attr: &str, decl: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines.windows(2).any(|w| w[0] == attr && w[1] == decl)
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for e in read.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .is_some_and(|n| n == "target" || n == "corpus" || n == "artifacts")
            {
                continue;
            }
            walk_rs(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn read_all(files: Vec<PathBuf>) -> Vec<(PathBuf, String)> {
    files
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect()
}

/// The shipped source: `src`.
fn rust_sources() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    walk_rs(&manifest("src"), &mut files);
    files.sort();
    read_all(files)
}

/// Everything Rust that this crate builds or runs: `src`, `build.rs` (which runs on the machine of whoever installs it),
/// `tests` and `fuzz` (#1783).
fn all_sources() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    for d in ["src", "tests", "fuzz"] {
        walk_rs(&manifest(d), &mut files);
    }
    if manifest("build.rs").is_file() {
        files.push(manifest("build.rs"));
    }
    files.sort();
    read_all(files)
}

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
    "nix",
    "rustix",
];

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
        .current_dir(manifest(""))
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
    for n in NETWORK {
        assert!(
            !deps.contains(n),
            "a networking crate is in klatch's dependencies: {n}"
        );
    }
    assert!(deps.len() > 20, "the tree looks empty: {deps:?}");
}

/// Test-only modules that may run a program chosen at run time: they run the test binary again in a child process with
/// a private environment, or start a stand-in for the coding agent's process, so key, registry and home paths can be
/// tested without the real ones. Exact paths relative to the crate, never a suffix: a new file needs a new line here,
/// and must be declared `#[cfg(test)] mod name;` in its parent so it is never part of the shipped binary (#771).
const TEST_ONLY_SPAWNERS: &[&str] = &[
    "src/admin/command_tests.rs",
    "src/harness/linux_real.rs",
    "src/member/custody/survivor_tests.rs",
    "src/member/identity/survivor_tests.rs",
];

/// Shipped modules that may run a program chosen at run time because they choose it from a fixed list of absolute paths
/// and never look a name up through PATH: the process table reader (#1568). Exact path, never a suffix, and the file may
/// hold exactly one `Command::new(` (checked below), so a second spawn needs a new line here.
const FIXED_PATH_SPAWNERS: &[&str] = &["src/harness/ps_snapshot.rs"];

/// Test-only modules that may open a loopback socket: the unit tests of the optional real Jev transport (`jev-http`,
/// #871) start a server on 127.0.0.1 inside the test and talk to it, so the transport can be tested without leaving the
/// machine. Exact paths, and each must be declared `#[cfg(test)] mod name;` in its parent so it is never part of a
/// shipped binary. The transport itself (`src/jev/http.rs`) names no socket type and is not exempt.
const TEST_ONLY_LOOPBACK_SERVERS: &[&str] = &["src/jev/http/tests.rs"];

/// The module a file declares: its file name without `.rs`.
fn module_name(rel: &str) -> &str {
    rel.rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".rs"))
        .unwrap_or("")
}

fn may_open_loopback_socket(rel: &str) -> bool {
    let name = module_name(rel);
    TEST_ONLY_LOOPBACK_SERVERS.contains(&rel)
        && parent_text(rel).is_some_and(|text| declared_under_cfg_test(&text, name))
}

/// The one file that may name `UnixStream` (and no other socket type): the Rust client's connect to the klatch-server
/// daemon over a local Unix socket (#1095). It connects to a path on this machine and listens for nothing. It is
/// allowed only while its parent declares it `mod socket;` directly under `#[cfg(all(unix, feature = "client-socket"))]`,
/// a feature that is in no default feature (see `the_client_socket_is_in_no_default_feature`), so the shipped binary
/// never contains it.
const CLIENT_SOCKET_FILE: &str = "src/client/socket.rs";

fn may_connect_unix_socket(rel: &str) -> bool {
    rel == CLIENT_SOCKET_FILE
        && parent_text(rel).is_some_and(|text| {
            declared_right_after(
                &text,
                "#[cfg(all(unix, feature = \"client-socket\"))]",
                "mod socket;",
            )
        })
}

/// The one file that may name `TcpStream` (and no other socket type): the Rust client's connect to `klatch-server http`
/// (#1122). It connects to a loopback port and listens for nothing (the url is checked before a socket is opened). It
/// is allowed only while its parent declares it `mod http;` directly under `#[cfg(feature = "client-http")]`, a feature
/// that is in no default feature (see `the_client_http_is_in_no_default_feature`), so the shipped binary never
/// contains it. It names no other socket type, and no resolver (`ToSocketAddrs`, `lookup_host`).
const CLIENT_HTTP_FILE: &str = "src/client/http.rs";

fn may_connect_tcp(rel: &str) -> bool {
    rel == CLIENT_HTTP_FILE
        && parent_text(rel).is_some_and(|text| {
            declared_right_after(&text, "#[cfg(feature = \"client-http\")]", "mod http;")
        })
}

/// The text of the file that declares the module at `rel` (`src/a/b.rs` is declared in `src/a.rs` or `src/a/mod.rs`),
/// for the first of them that exists.
fn parent_text(rel: &str) -> Option<String> {
    let (dir, _) = rel.rsplit_once('/')?;
    [format!("{dir}.rs"), format!("{dir}/mod.rs")]
        .iter()
        .find_map(|p| std::fs::read_to_string(manifest(p)).ok())
}

/// Whether `text` declares `mod name;` directly under `#[cfg(test)]` (other attributes may sit between them).
fn declared_under_cfg_test(text: &str, name: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let want = format!("mod {name};");
    lines.iter().enumerate().any(|(i, l)| {
        *l == want
            && lines[..i]
                .iter()
                .rev()
                .take_while(|a| a.starts_with("#["))
                .any(|a| *a == "#[cfg(test)]")
    })
}

/// Whether the file at `rel` may run a program chosen at run time: it is listed in `exempt` by its exact path and
/// `parent_text` (the declaring file) declares it under `#[cfg(test)]`.
fn is_test_only_spawner(
    rel: &str,
    exempt: &[&str],
    parent_text: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let name = module_name(rel);
    exempt.contains(&rel)
        && parent_text(rel).is_some_and(|text| declared_under_cfg_test(&text, name))
}

/// What is wrong with a `Command::new(...)` on this line of code, if anything: it runs a program that is not on the
/// list, or one chosen at run time outside the client and the test-only modules.
fn command_problem(rel: &str, code: &str, programs: &[&str]) -> Option<String> {
    let arg = &code[code.find("Command::new(")? + "Command::new(".len()..];
    match arg.strip_prefix('"').and_then(|a| a.split('"').next()) {
        Some(program) if programs.contains(&program) => None,
        Some(program) => Some(format!("runs {program}")),
        None if rel.ends_with("src/client.rs") => None,
        None if FIXED_PATH_SPAWNERS.contains(&rel) => None,
        None if is_test_only_spawner(rel, TEST_ONLY_SPAWNERS, &parent_text) => None,
        None => Some("runs a program chosen at run time".to_string()),
    }
}

const SOCKETS: [&str; 9] = [
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "UnixStream",
    "UnixListener",
    "UnixDatagram",
    "TcpSocket",
    "UnixSeqpacket",
    "UnixSeqpacketListener",
];

/// Programs klatch starts by name: git (to find a repository), ps (the process table), secret-tool, busctl (only to ask whether a locked keyring hid a key, #561) and security
/// (the keystores), codex (only `--version` and `queue --help`, in `klatch doctor`: no login, no model call), claude (`--version` in `klatch doctor` and before `klatch wake`, #482: no login, no model call; and, only when an operator opts in with `klatch judge run --hosted-judge`, one headless call per queued item, #960), and sh only in a unit test that waits for a fake keystore script to be executable. Only that opt-in `claude` call reaches a model service (and only because the operator named `--hosted-judge`); every other listed program is run for a local answer and is not asked to talk to a network. The Rust client starts the binary the caller names.
const PROGRAMS: &[&str] = &[
    "git",
    "ps",
    "secret-tool",
    "busctl",
    "/usr/bin/security",
    "codex",
    "claude",
    "sh",
];

/// Whether the file at `rel` is allowed to name this socket type.
fn socket_allowed(rel: &str, socket: &str) -> bool {
    may_open_loopback_socket(rel)
        || (socket == "UnixStream" && may_connect_unix_socket(rel))
        || (socket == "TcpStream" && may_connect_tcp(rel))
}

#[test]
fn the_source_opens_no_socket_and_runs_only_the_programs_it_is_known_to_run() {
    let mut problems = Vec::new();
    for (path, text) in rust_sources() {
        let rel = rel_of(&path);
        // identifiers and `Command::new` calls come from the tokenizer: a `//` in a string hides nothing
        for (line, name) in net_scan::ids(&text) {
            if SOCKETS.contains(&name.as_str()) && !socket_allowed(&rel, &name) {
                problems.push(format!("{rel}:{line}: uses {name}"));
            }
        }
        for (line, code) in net_scan::command_new_calls(&text) {
            problems.extend(
                command_problem(&rel, &code, PROGRAMS).map(|p| format!("{rel}:{line}: {p}")),
            );
        }
    }
    assert!(
        problems.is_empty(),
        "outbound capability in the source:\n{}",
        problems.join("\n")
    );
}

/// A use of an OS-level network call that is allowed, by exact file and exact line, with the reason. `test_only` needs
/// the file to be declared `#[cfg(test)] mod name;` by its parent, so it is never part of a shipped binary. Nothing
/// else is allowed: a new line needs a new entry here, and an entry that matches no line (or more than one) fails
/// `every_network_allowance_matches_exactly_one_real_line`.
struct NetAllowance {
    file: &'static str,
    line: &'static str,
    test_only: bool,
    reason: &'static str,
}

const NET_ALLOWED: &[NetAllowance] = &[
    NetAllowance {
        file: "tests/harness_peer_identity.rs",
        line: "use std::os::unix::net::{UnixListener, UnixStream};",
        test_only: true,
        reason: "an integration test (experiment 14.3, #29) of the peer-pid lookup: it listens on a Unix socket in its own \
                 temporary directory and connects to it from the same test, so the pid of a local process can be read; it \
                 is a test crate, never part of a shipped binary, and no network is involved",
    },
    NetAllowance {
        file: "tests/harness_peer_identity.rs",
        line: "fn getsockopt(",
        test_only: true,
        reason: "the same test: getsockopt(LOCAL_PEERPID or SO_PEERCRED) on that local Unix socket, to learn the pid at the \
                 other end; it declares the one function it calls and nothing that opens, binds or connects a socket",
    },
    NetAllowance {
        file: "src/harness/hooks_pointer/tests.rs",
        line: "unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) },",
        test_only: true,
        reason: "the #1770 fixture: an unnamed pair of connected Unix sockets inside the test process, used only to hand the \
             hooks-pointer reader a socket file handle so its type check can be tested; no name, no address, nothing \
             leaves the process",
    },
];

fn net_allowance_for<'a>(
    rel: &str,
    code: &str,
    parent: &dyn Fn(&str) -> Option<String>,
) -> Option<&'a NetAllowance> {
    NET_ALLOWED.iter().find(|a| {
        a.file == rel
            && a.line == code.trim()
            && (!a.test_only
                || rel.starts_with("tests/")
                || parent(rel).is_some_and(|text| declared_under_cfg_test(&text, module_name(rel))))
    })
}

/// The files a test-only allowance names: a `#[path]` or `include!` that reaches one of them from shipped code would
/// pull a test fixture, or a spawner, into a shipped binary (#1783).
fn protected_files() -> Vec<&'static str> {
    NET_ALLOWED
        .iter()
        .map(|a| a.file)
        .chain(TEST_ONLY_SPAWNERS.iter().copied())
        .chain(TEST_ONLY_LOOPBACK_SERVERS.iter().copied())
        .collect()
}

/// OS-level network calls and names in one source file (#1771, #1783): what `net_scan::findings` reports, the socket type
/// names of `SOCKETS` (in `src` only; the integration tests are test code and test the client), and a `#[path]` or
/// `include!` that names a protected test-only file. A line with an allowance (`NET_ALLOWED`) is not a problem.
fn net_problems(rel: &str, text: &str, parent: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut problems = Vec::new();
    let mut report = |line: usize, what: String| {
        let raw = lines.get(line.wrapping_sub(1)).copied().unwrap_or("");
        if net_allowance_for(rel, raw, parent).is_none() {
            problems.push(format!("{rel}:{line}: {what}: {}", raw.trim()));
        }
    };
    for f in net_scan::findings(text) {
        if f.what == "std::os::unix::net" && may_connect_unix_socket(rel) {
            continue;
        }
        report(f.line, f.what);
    }
    if rel.starts_with("src/") {
        for (line, name) in net_scan::ids(text) {
            if SOCKETS.contains(&name.as_str()) && !socket_allowed(rel, &name) {
                report(line, format!("the socket type {name}"));
            }
        }
    }
    let protected = protected_files();
    for (line, target) in net_scan::path_targets(text) {
        let resolved = net_scan::resolve_relative(rel, &target);
        if protected.contains(&resolved.as_str()) {
            report(
                line,
                format!("a #[path] or include! of the test-only file {resolved}"),
            );
        }
    }
    problems.sort();
    problems.dedup();
    problems
}

/// A throwaway source line per pattern that the guard must flag, with the pattern's name for the message.
fn flagged_snippets() -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = Vec::new();
    for call in [
        "socket",
        "socketpair",
        "connect",
        "bind",
        "listen",
        "accept",
        "accept4",
        "sendto",
        "sendmsg",
        "sendmmsg",
        "recvfrom",
        "recvmsg",
        "recvmmsg",
        "send",
        "recv",
        "getaddrinfo",
        "getnameinfo",
        "gethostbyname",
        "gethostbyaddr",
        "getsockname",
        "getpeername",
        "setsockopt",
        "getsockopt",
        "shutdown",
    ] {
        v.push((
            "a libc call by path",
            format!("    let r = unsafe {{ libc::{call}(fd, std::ptr::null_mut(), 0) }};\n"),
        ));
        v.push((
            "a libc call by import",
            format!("use libc::{{c_int, {call}}};\nfn f() {{ unsafe {{ {call}(1, 2, 3) }}; }}\n"),
        ));
        v.push((
            "a libc call by import over lines",
            format!("use libc::{{\n    c_int,\n    {call} as raw_call,\n}};\n"),
        ));
        v.push((
            "an extern declaration",
            format!("unsafe extern \"C\" {{\n    fn {call}(a: i32, b: i32, c: i32) -> i32;\n}}\n"),
        ));
    }
    for what in [
        "libc::AF_INET",
        "libc::AF_INET6",
        "libc::AF_UNIX",
        "libc::AF_NETLINK",
        "libc::SOCK_STREAM",
        "libc::SOCK_DGRAM",
        "libc::IPPROTO_TCP",
        "libc::SOL_SOCKET",
        "libc::sockaddr_in",
        "libc::sockaddr_un",
        "libc::addrinfo",
        "libc::msghdr",
    ] {
        v.push((
            "a libc socket constant or type",
            format!("    let x = {what};\n"),
        ));
    }
    v.push(("a glob import of libc", "use libc::*;\n".to_string()));
    v.push((
        "a glob import of libc in braces",
        "use libc::{c_int, *};\n".to_string(),
    ));
    for what in [
        "nix::sys::socket::socket(a, b, c, d)",
        "rustix::net::socket(a, b, c)",
        "socket2::Socket::new(a, b, c)",
        "tokio::net::TcpStream::connect(a)",
        "mio::net::TcpListener::bind(a)",
        "async_std::net::UdpSocket::bind(a)",
        "smol::net::TcpStream::connect(a)",
        "std::os::unix::net::UnixDatagram::unbound()",
        "std::os::unix::net::UnixListener::bind(a)",
        "std::net::TcpStream::connect(a)",
        "std::net::UdpSocket::bind(a)",
        "std::net::TcpListener::bind(a)",
        "addr.to_socket_addrs()",
        "std::net::ToSocketAddrs::to_socket_addrs(&a)",
        "tokio::net::lookup_host(a)",
        "TcpSocket::new_v4()",
        "UnixSeqpacket::connect(a)",
    ] {
        v.push((
            "another network crate or std name",
            format!("    let c = {what};\n"),
        ));
    }
    v
}

#[test]
fn every_network_pattern_is_flagged_in_a_throwaway_source() {
    let never = |_: &str| None;
    for (what, src) in flagged_snippets() {
        let found = net_problems("src/some/new_file.rs", &src, &never);
        assert!(!found.is_empty(), "not flagged: {what}:\n{src}");
    }
}

/// The guard reads `build.rs`, `tests` and `fuzz` as well as `src` (#1783): `build.rs` runs on the machine of whoever
/// installs the crate, and the others are code the project runs.
#[test]
fn the_guard_reads_build_rs_tests_and_fuzz_as_well_as_src() {
    let rels: Vec<String> = all_sources().iter().map(|(p, _)| rel_of(p)).collect();
    for want in [
        "build.rs",
        "tests/no_telemetry.rs",
        "tests/net_scan/mod.rs",
        "src/lib.rs",
    ] {
        assert!(
            rels.iter().any(|r| r == want),
            "{want} is not read: {} files",
            rels.len()
        );
    }
    assert!(
        rels.iter().any(|r| r.starts_with("fuzz/")),
        "fuzz is not read"
    );
    assert!(
        rels.iter().any(|r| r.starts_with("tests/")),
        "tests are not read"
    );
}

/// Every way past the first version of the guard (#1783) is reported, in a file with no allowance.
#[test]
fn every_evasion_of_the_first_guard_is_flagged() {
    let never = |_: &str| None;
    for (what, file, src) in net_scan::evasions() {
        let found = net_problems(file, &src, &never);
        assert!(!found.is_empty(), "not flagged: {what}:\n{src}");
    }
}

/// A `#[path]` or `include!` that reaches a protected test-only file is reported, whichever file does it.
#[test]
fn a_path_or_include_of_a_protected_test_only_file_is_flagged() {
    let never = |_: &str| None;
    let protected = protected_files();
    assert!(protected.len() >= 8, "{protected:?}");
    for target in protected {
        for (what, file, src) in net_scan::path_evasions(target) {
            let found = net_problems(&file, &src, &never);
            assert!(!found.is_empty(), "not flagged: {what} of {target}:\n{src}");
        }
    }
}

/// Words in strings and comments, and libc names that make no network use, are not reported.
#[test]
fn comments_strings_and_ordinary_libc_use_are_not_flagged() {
    let never = |_: &str| None;
    for (what, src) in net_scan::clean_sources() {
        let found = net_problems("src/some/new_file.rs", &src, &never);
        assert!(found.is_empty(), "flagged: {what}: {found:?}");
    }
}

/// The scanner reads a source as tokens: what a line search could not.
#[test]
fn the_scanner_reads_tokens_not_lines() {
    use net_scan::*;
    // a // in a string does not hide the rest of the line, and a comment does not make code
    let f = findings("let _u = \"//\"; unsafe { libc::connect(1, 2, 3) }; // libc::bind\n");
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].what.contains("connect"));
    // the line of a finding is the line of the token, across a split path
    let f = findings(
        "fn f() {\n    unsafe {\n        libc\n            ::\n            socket(1, 2, 3)\n    };\n}\n",
    );
    assert_eq!(f.iter().map(|x| x.line).collect::<Vec<_>>(), [3]);
    // an alias is followed, and only libc's own aliases
    assert_eq!(
        findings("use libc as l;\nfn f() { l::socket(1, 2, 3); }\n").len(),
        1
    );
    assert!(findings("use other as l;\nfn f() { l::socket(1, 2, 3); }\n").is_empty());
    // comments are stripped with the line count kept, strings stay
    assert_eq!(
        strip_comments("a // x\nb /* y\nz */ c \"//\"\n")
            .lines()
            .count(),
        3
    );
    assert!(strip_comments("let u = \"//\"; // x\n").contains("\"//\""));
    assert!(!strip_comments("let u = 1; // x\n").contains('x'));
    // a #[path] and an include! are resolved against the file that names them
    assert_eq!(
        path_targets("#[path = \"a/b.rs\"] mod m;\ninclude!(\"c.rs\");\n"),
        [(1, "a/b.rs".to_string()), (2, "c.rs".to_string())]
    );
    assert_eq!(resolve_relative("src/x/y.rs", "../z/w.rs"), "src/z/w.rs");
    assert_eq!(resolve_relative("src/x.rs", "x/w.rs"), "src/x/w.rs");
}

#[test]
fn the_source_makes_no_os_level_network_call() {
    let mut problems = Vec::new();
    for (path, text) in all_sources() {
        problems.extend(net_problems(&rel_of(&path), &text, &parent_text));
    }
    assert!(
        problems.is_empty(),
        "an OS-level network call or name in the source (add an exact NET_ALLOWED entry with a reason only if it is \
         a test fixture with no network, never for shipped code):\n{}",
        problems.join("\n")
    );
}

/// Every allowance names a file that exists, and its line occurs exactly once there: a stale or widened entry fails.
#[test]
fn every_network_allowance_matches_exactly_one_real_line() {
    for a in NET_ALLOWED {
        let text = std::fs::read_to_string(manifest(a.file))
            .unwrap_or_else(|_| panic!("{} is allowed but does not exist", a.file));
        let hits = text
            .lines()
            .filter(|l| l.split("//").next().unwrap_or("").trim() == a.line)
            .count();
        assert_eq!(hits, 1, "{}: the allowed line {:?}", a.file, a.line);
        assert!(a.reason.len() > 40, "{}: a reason is needed", a.file);
        assert!(
            a.test_only,
            "{}: only a test fixture may be allowed here",
            a.file
        );
        assert!(
            a.file.starts_with("tests/")
                || parent_text(a.file)
                    .is_some_and(|t| declared_under_cfg_test(&t, module_name(a.file))),
            "{}: not a test crate and not declared #[cfg(test)] by its parent",
            a.file
        );
    }
}

/// The allowance is for one line of one file, declared one way. It is the whole list, and growing it is a deliberate act.
#[test]
fn the_network_allowance_is_three_exact_lines_and_covers_nothing_else() {
    assert_eq!(
        NET_ALLOWED.len(),
        3,
        "a new allowance needs a review and this number changed"
    );
    let declared = |_: &str| Some("#[cfg(test)]\nmod tests;\n".to_string());
    let plain = |_: &str| Some("mod tests;\n".to_string());
    let none = |_: &str| None;
    for a in NET_ALLOWED {
        // the exact line in the exact file is allowed; the same line elsewhere is not
        assert!(
            net_allowance_for(a.file, &format!("    {}  ", a.line), &declared).is_some(),
            "{}",
            a.line
        );
        for other in [
            "src/harness/hooks_pointer.rs",
            "src/harness/other/tests.rs",
            "src/x/src/harness/hooks_pointer/tests.rs",
            "tests/other.rs",
            "src/tests/harness_peer_identity.rs",
        ] {
            assert!(
                net_allowance_for(other, a.line, &declared).is_none(),
                "{other}: {}",
                a.line
            );
        }
        // a changed line, or a second call on it, is not the allowed line
        for changed in [
            format!("{} libc::connect(1, 2, 3);", a.line),
            format!("{}x", a.line),
            a.line
                .replace("net::", "net2::")
                .replace("AF_UNIX", "AF_INET")
                .replace("SOCK_STREAM", "SOCK_DGRAM"),
        ] {
            if changed != a.line {
                assert!(
                    net_allowance_for(a.file, &changed, &declared).is_none(),
                    "{changed}"
                );
            }
        }
        // a module file needs its parent to declare it under cfg(test); a test crate (under tests/) is its own root
        if !a.file.starts_with("tests/") {
            assert!(
                net_allowance_for(a.file, a.line, &plain).is_none(),
                "declared without cfg(test)"
            );
            assert!(
                net_allowance_for(a.file, a.line, &none).is_none(),
                "not declared"
            );
        }
        // and every other network call in the allowed file is still a problem
        for call in [
            "fn f() { unsafe { libc::connect(fd, a, 3) }; }",
            "fn f() { unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) }; }",
            "fn f() { unsafe { libc::socketpair(1, 2, 3, p) }; }",
            "fn f() { unsafe { libc::syscall(41) }; }",
        ] {
            assert!(
                !net_problems(a.file, &format!("{call}\n"), &declared).is_empty(),
                "{call}"
            );
        }
    }
}

/// The calls the ticket names are in the list: dropping one makes this fail (and its throwaway snippet too).
#[test]
fn the_network_pattern_lists_keep_the_calls_they_must_catch() {
    use net_scan::*;
    for call in [
        "socket",
        "socketpair",
        "connect",
        "connectx",
        "bind",
        "listen",
        "accept",
        "paccept",
        "sendto",
        "sendmsg",
        "sendmsg_x",
        "recvfrom",
        "recvmsg",
        "getaddrinfo",
        "getifaddrs",
        "res_init",
        "syscall",
    ] {
        assert!(LIBC_NET_CALLS.contains(&call), "{call}");
    }
    for call in ["execvp", "system", "popen", "posix_spawnp", "fexecve"] {
        assert!(LIBC_SPAWN_CALLS.contains(&call), "{call}");
    }
    for prefix in [
        "AF_", "SOCK_", "IPPROTO_", "sockaddr", "SYS_", "INADDR_", "NETLINK_", "SIOC", "SO_",
        "MSG_", "PF_",
    ] {
        assert!(LIBC_NET_PREFIXES.contains(&prefix), "{prefix}");
    }
    assert!(LIBC_NET_NAMES.contains(&"socklen_t"));
    for module in [
        ["tokio", "net"].as_slice(),
        &["mio", "net"],
        &["socket2"],
        &["nix", "sys", "socket"],
        &["rustix", "net"],
    ] {
        assert!(NET_MODULES.contains(&module), "{module:?}");
    }
    for name in [
        "ToSocketAddrs",
        "lookup_host",
        "getaddrinfo",
        "to_socket_addrs",
        "getnameinfo",
    ] {
        assert!(RESOLVERS.contains(&name), "{name}");
    }
    for socket in [
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "UnixStream",
        "UnixListener",
        "UnixDatagram",
    ] {
        assert!(SOCKETS.contains(&socket), "{socket}");
    }
}

/// What a line importing from `std::net` names that only a socket user needs, if anything.
fn socket_item_in(rel: &str, line: &str) -> Option<&'static str> {
    const ITEMS: [&str; 5] = [
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "ToSocketAddrs",
        "lookup_host",
    ];
    ITEMS.into_iter().find(|item| {
        // the client's HTTP file imports `TcpStream` and nothing else of the kind
        line.contains(item) && !(*item == "TcpStream" && may_connect_tcp(rel))
    })
}

#[test]
fn std_net_is_only_used_to_read_addresses() {
    for (path, text) in rust_sources() {
        let rel = rel_of(&path);
        if may_open_loopback_socket(&rel) {
            continue;
        }
        let text = net_scan::strip_comments(&text);
        for line in text.lines().filter(|l| l.contains("std::net")) {
            assert!(
                socket_item_in(&rel, line).is_none(),
                "{}: {line}",
                path.display()
            );
        }
    }
}

/// The real Jev transport is an outbound network capability, so no default build may carry it: the feature is not in
/// `default`, `client` or `cli`, and no shipped feature turns on the HTTP client (#871).
#[test]
fn the_optional_jev_http_transport_is_in_no_default_feature() {
    let shipped = feature_bodies();
    for name in ["default", "client", "cli"] {
        let body = shipped
            .get(name)
            .unwrap_or_else(|| panic!("no feature {name}"));
        assert!(
            !body.contains("jev-http") && !body.contains("ureq"),
            "{name} turns on the Jev HTTP client: {body}"
        );
    }
    assert!(
        shipped["jev-http"].contains("dep:ureq"),
        "jev-http is where the HTTP client comes in"
    );
}

/// The Rust client's socket connect (#1095) is a feature of its own: no default feature turns it on, so the shipped
/// `klatch` binary contains no socket code, and it brings in nothing but the client.
#[test]
fn the_client_socket_is_in_no_default_feature() {
    let bodies = feature_bodies();
    for name in ["default", "client", "cli"] {
        assert!(
            !bodies[name].contains("client-socket"),
            "{name} turns on the client socket: {}",
            bodies[name]
        );
    }
    assert_eq!(
        bodies["client-socket"].trim(),
        "[\"client\"]",
        "client-socket brings in nothing but the client"
    );
}

/// The Rust client's HTTP connect (#1122) is a feature of its own, like the socket connect: no default feature turns it
/// on, so the shipped `klatch` binary (and so every wheel, the npm package and the Homebrew build, which build with the
/// default features) contains no network socket type, and it brings in nothing but the client.
#[test]
fn the_client_http_is_in_no_default_feature() {
    let bodies = feature_bodies();
    for (name, body) in &bodies {
        if name != "client-http" {
            assert!(
                !body.contains("client-http"),
                "{name} turns on the client HTTP transport: {body}"
            );
        }
    }
    assert_eq!(
        bodies["client-http"].trim(),
        "[\"client\"]",
        "client-http brings in nothing but the client"
    );
}

/// The allowance for `TcpStream` is for one file, declared one way; the same file elsewhere or another file is not it.
#[test]
fn only_the_client_http_file_may_name_a_tcp_stream() {
    assert!(may_connect_tcp("src/client/http.rs"));
    for other in [
        "src/client/socket.rs",
        "src/client.rs",
        "src/jev/http.rs",
        "src/gateway/http.rs",
        "src/client/http/tests.rs",
    ] {
        assert!(!may_connect_tcp(other), "{other}");
    }
}

/// Whether a line fetches a script and pipes it into a shell.
fn pipes_a_download_into_a_shell(line: &str) -> bool {
    let piped = ["| sh", "| bash", "|sh", "|bash"]
        .iter()
        .any(|p| line.contains(p));
    let fetches = line.contains("curl") || line.contains("wget");
    piped && fetches
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
            assert!(
                !pipes_a_download_into_a_shell(line),
                "{doc}:{}: pipes a download into a shell: {line}",
                n + 1
            );
        }
    }
}

fn is_github_host(host: &str) -> bool {
    host == "github.com" || host == "api.github.com" || host.starts_with('$')
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
                is_github_host(host),
                "install.sh mentions another host: {word}"
            );
            assert!(
                word.starts_with("https://"),
                "install.sh uses plain http: {word}"
            );
        }
    }
}

const EXEMPT_PATH: &str = "src/a/survivor_tests.rs";

fn exempt_with(rel: &str, parent: &dyn Fn(&str) -> Option<String>) -> bool {
    is_test_only_spawner(rel, &[EXEMPT_PATH], parent)
}

#[test]
fn a_test_module_is_exempt_only_by_exact_path_and_only_when_declared_under_cfg_test() {
    let declared = |_: &str| Some("#[cfg(test)]\nmod survivor_tests;\n".to_string());
    assert!(exempt_with(EXEMPT_PATH, &declared));
    // the same file name in another directory is not exempt, whatever its parent says
    for other in [
        "src/b/survivor_tests.rs",
        "src/a/b/survivor_tests.rs",
        "src/a/survivor_tests.rs.bak",
    ] {
        assert!(!exempt_with(other, &declared), "{other}");
    }
    // listed, but the parent declares it without #[cfg(test)], with cfg(not(test)), or not at all
    let plain = |_: &str| Some("mod survivor_tests;\n".to_string());
    let not_test = |_: &str| Some("#[cfg(not(test))]\nmod survivor_tests;\n".to_string());
    let other_mod = |_: &str| Some("#[cfg(test)]\nmod other_tests;\n".to_string());
    let separated = |_: &str| Some("#[cfg(test)]\nfn f() {}\nmod survivor_tests;\n".to_string());
    let none = |_: &str| None;
    for parent in
        [&plain, &not_test, &other_mod, &separated, &none] as [&dyn Fn(&str) -> Option<String>; 5]
    {
        assert!(!exempt_with(EXEMPT_PATH, parent));
    }
    // another attribute between the cfg and the module is fine
    let attr =
        |_: &str| Some("#[cfg(test)]\n#[allow(dead_code)]\nmod survivor_tests;\n".to_string());
    assert!(exempt_with(EXEMPT_PATH, &attr));
}

#[test]
fn every_listed_test_only_spawner_exists_and_is_declared_under_cfg_test_in_the_real_source() {
    for rel in TEST_ONLY_SPAWNERS {
        assert!(
            manifest(rel).is_file(),
            "{rel} is listed but does not exist: remove it from TEST_ONLY_SPAWNERS"
        );
        assert!(
            is_test_only_spawner(rel, TEST_ONLY_SPAWNERS, &parent_text),
            "{rel}: its parent does not declare it as #[cfg(test)] mod ...;"
        );
    }
}

#[test]
fn a_fixed_path_spawner_is_exempt_by_exact_path_and_runs_exactly_one_program() {
    let call = "let mut c = Command::new(program);";
    assert_eq!(
        command_problem("src/harness/ps_snapshot.rs", call, PROGRAMS),
        None
    );
    for other in ["src/harness/other.rs", "src/x/src/harness/ps_snapshot.rs"] {
        assert!(command_problem(other, call, PROGRAMS).is_some(), "{other}");
    }
    for rel in FIXED_PATH_SPAWNERS {
        let text = std::fs::read_to_string(manifest(rel)).expect("a listed spawner exists");
        assert_eq!(text.matches("Command::new(").count(), 1, "{rel}");
    }
}
