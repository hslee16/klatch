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
    walk(&manifest("src"), &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect()
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
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let at = format!("{rel}:{}", n + 1);
            problems.extend(
                SOCKETS
                    .iter()
                    .filter(|s| code.contains(*s) && !socket_allowed(&rel, s))
                    .map(|s| format!("{at}: uses {s}")),
            );
            problems.extend(command_problem(&rel, code, PROGRAMS).map(|p| format!("{at}: {p}")));
        }
    }
    assert!(
        problems.is_empty(),
        "outbound capability in the source:\n{}",
        problems.join("\n")
    );
}

/// The `libc` calls that make, connect, bind, listen on, accept on, send or receive through a socket, or resolve a name
/// (#1771). A socket made through one of them names no socket type, so the type-name list (`SOCKETS`) cannot see it.
const LIBC_NET_CALLS: &[&str] = &[
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
];

/// `libc` names that only a socket user needs: address families, socket types, protocols, option levels and options,
/// message flags, and the address and message structs. A prefix of the name after `libc::`.
const LIBC_NET_PREFIXES: &[&str] = &[
    "AF_",
    "PF_",
    "SOCK_",
    "IPPROTO_",
    "SOL_",
    "SO_",
    "MSG_",
    "sockaddr",
    "in_addr",
    "in6_addr",
    "addrinfo",
    "msghdr",
    "cmsghdr",
    "ip_mreq",
    "ipv6_mreq",
];

/// Modules and crates of other network APIs, as they are written in a path.
const NET_MODULES: &[&str] = &[
    "nix::sys::socket",
    "rustix::net",
    "socket2",
    "tokio::net",
    "mio::net",
    "async_std::net",
    "smol::net",
];

/// Names that look a host up or turn a name into an address.
const RESOLVERS: &[&str] = &[
    "getaddrinfo",
    "gethostbyname",
    "getnameinfo",
    "to_socket_addrs",
    "ToSocketAddrs",
    "lookup_host",
];

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

const NET_ALLOWED: &[NetAllowance] = &[NetAllowance {
    file: "src/harness/hooks_pointer/tests.rs",
    line: "unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) },",
    test_only: true,
    reason: "the #1770 fixture: an unnamed pair of connected Unix sockets inside the test process, used only to hand the \
             hooks-pointer reader a socket file handle so its type check can be tested; no name, no address, nothing \
             leaves the process",
}];

fn net_allowance_for<'a>(
    rel: &str,
    code: &str,
    parent: &dyn Fn(&str) -> Option<String>,
) -> Option<&'a NetAllowance> {
    NET_ALLOWED.iter().find(|a| {
        a.file == rel
            && a.line == code.trim()
            && (!a.test_only
                || parent(rel).is_some_and(|text| declared_under_cfg_test(&text, module_name(rel))))
    })
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether `code` holds `word` as a whole identifier (not inside a longer one).
fn has_word(code: &str, word: &str) -> bool {
    code.match_indices(word).any(|(i, _)| {
        !code[..i].chars().next_back().is_some_and(is_ident)
            && !code[i + word.len()..].chars().next().is_some_and(is_ident)
    })
}

/// What a name imported from `libc` or declared in an `extern` block is, if it is a network name.
fn is_libc_net_name(name: &str) -> bool {
    name == "*"
        || LIBC_NET_CALLS.contains(&name)
        || LIBC_NET_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// The identifiers (and `*`) of a `use libc::...;` statement starting at `from`.
fn imported_names(rest: &str) -> Vec<String> {
    let stmt = rest.split(';').next().unwrap_or("");
    let mut names = Vec::new();
    let mut cur = String::new();
    for c in stmt.chars().chain(std::iter::once(' ')) {
        if is_ident(c) || c == '*' {
            cur.push(c);
        } else if !cur.is_empty() {
            names.push(std::mem::take(&mut cur));
        }
    }
    names
}

/// The functions declared in the `extern "..." { }` blocks of `text`, with the line each is on.
fn extern_declared_fns(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("extern \"") {
        let at = from + i;
        let after = &text[at..];
        // `extern "C" {` (a block), not `extern "C" fn` (a callback)
        let head_end = after.find(['{', ';', '(']).unwrap_or(after.len());
        let head = &after[..head_end];
        if after[head_end..].starts_with('{') && !head.contains(" fn ") {
            let mut depth = 0;
            let mut end = after.len();
            for (k, c) in after[head_end..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = head_end + k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let block = &after[head_end..end];
            for (k, _) in block.match_indices("fn ") {
                let name: String = block[k + 3..]
                    .chars()
                    .take_while(|c| is_ident(*c))
                    .collect();
                let line = text[..at + head_end + k].matches('\n').count() + 1;
                out.push((line, name));
            }
        }
        from = at + 8;
    }
    out
}

/// OS-level network calls and names in one source file (#1771): `libc` calls by path or by import, `libc` socket
/// constants and structs, an `extern` declaration of one, other network APIs by module path, resolvers, and the socket
/// type names of `SOCKETS`. A line with an allowance (`NET_ALLOWED`) is not a problem.
fn net_problems(rel: &str, text: &str, parent: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut problems = Vec::new();
    let mut report = |line: usize, what: String, code: &str| {
        if net_allowance_for(rel, code, parent).is_none() {
            problems.push(format!("{rel}:{line}: {what}: {}", code.trim()));
        }
    };
    let lines: Vec<&str> = text.lines().collect();
    let code_of = |l: &str| l.split("//").next().unwrap_or("").to_string();
    for (n, raw) in lines.iter().enumerate() {
        let code = code_of(raw);
        let at = n + 1;
        for call in LIBC_NET_CALLS {
            if has_word(&code, &format!("libc::{call}")) {
                report(at, format!("libc::{call}"), &code);
            }
        }
        for prefix in LIBC_NET_PREFIXES {
            let want = format!("libc::{prefix}");
            if code
                .match_indices(&want)
                .any(|(i, _)| !code[..i].chars().next_back().is_some_and(is_ident))
            {
                report(at, format!("{want}..."), &code);
            }
        }
        for module in NET_MODULES {
            if code.contains(module) {
                report(at, format!("the network API {module}"), &code);
            }
        }
        if code.contains("os::unix::net") && !may_connect_unix_socket(rel) {
            report(at, "std::os::unix::net".to_string(), &code);
        }
        for name in RESOLVERS {
            if has_word(&code, name) {
                report(at, format!("the resolver {name}"), &code);
            }
        }
        for socket in SOCKETS {
            if has_word(&code, socket) && !socket_allowed(rel, socket) {
                report(at, format!("the socket type {socket}"), &code);
            }
        }
    }
    // `use libc::name;` and `use libc::{a, b as c};`, on one line or several
    for (i, _) in text.match_indices("use libc::") {
        let before = &text[..i];
        if before.chars().next_back().is_some_and(is_ident) {
            continue;
        }
        let line = before.matches('\n').count();
        if code_of(lines.get(line).copied().unwrap_or(""))
            .trim()
            .is_empty()
        {
            continue; // the statement is in a comment
        }
        let names = imported_names(&text[i + "use libc::".len()..]);
        if let Some(bad) = names.iter().find(|n| is_libc_net_name(n)) {
            let first = code_of(lines.get(line).copied().unwrap_or(""));
            report(line + 1, format!("use libc::{bad}"), &first);
        }
    }
    for (line, name) in extern_declared_fns(text) {
        if LIBC_NET_CALLS.contains(&name.as_str()) {
            let code = code_of(lines.get(line - 1).copied().unwrap_or(""));
            report(line, format!("an extern declaration of {name}"), &code);
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

/// Plain code that must not be flagged: comments, strings that are not code, and libc calls that make no network use.
#[test]
fn ordinary_libc_use_and_comments_are_not_flagged() {
    let never = |_: &str| None;
    for src in [
        "// libc::socket(a, b, c) is what the guard looks for\nfn f() {}\n",
        "/// a doc line that says libc::connect\nfn f() {}\n",
        "fn f() { let _ = unsafe { libc::mkfifo(p, 0o600) }; }\n",
        "fn f() { let _ = unsafe { libc::tcgetsid(fd) }; }\n",
        "use libc::{c_int, pid_t};\n",
        "extern \"C\" fn restore(sig: libc::c_int) {}\n",
        "fn socket_path() -> PathBuf { PathBuf::new() }\nfn connect_timeout() {}\n",
        "let bind_address = 1; let listen_port = 2; let accept_all = true;\n",
        "use std::net::{IpAddr, SocketAddr};\n",
    ] {
        assert!(
            net_problems("src/some/new_file.rs", src, &never).is_empty(),
            "{src}"
        );
    }
}

#[test]
fn the_source_makes_no_os_level_network_call() {
    let mut problems = Vec::new();
    for (path, text) in rust_sources() {
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
            parent_text(a.file).is_some_and(|t| declared_under_cfg_test(&t, module_name(a.file))),
            "{}: not declared #[cfg(test)] by its parent",
            a.file
        );
    }
}

/// The allowance is for one line of one file, declared one way. It is the whole list, and growing it is a deliberate act.
#[test]
fn the_network_allowance_is_one_line_and_covers_nothing_else() {
    assert_eq!(
        NET_ALLOWED.len(),
        1,
        "a new allowance needs a review and this number changed"
    );
    let a = &NET_ALLOWED[0];
    let declared = |_: &str| Some("#[cfg(test)]\nmod tests;\n".to_string());
    let plain = |_: &str| Some("mod tests;\n".to_string());
    let none = |_: &str| None;
    let text = format!("    {}\n", a.line);
    // the exact line in the exact file, declared under cfg(test): allowed
    assert!(net_problems(a.file, &text, &declared).is_empty());
    // the same line in any other file, or the same file declared without cfg(test) or not declared: flagged
    for other in [
        "src/harness/hooks_pointer.rs",
        "src/harness/other/tests.rs",
        "src/x/src/harness/hooks_pointer/tests.rs",
    ] {
        assert!(!net_problems(other, &text, &declared).is_empty(), "{other}");
    }
    assert!(
        !net_problems(a.file, &text, &plain).is_empty(),
        "declared without cfg(test)"
    );
    assert!(
        !net_problems(a.file, &text, &none).is_empty(),
        "not declared"
    );
    // another network call in the allowed file is flagged
    for call in [
        "libc::connect(fd, a, 3)",
        "libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0)",
        "libc::socketpair(1, 2, 3, p)",
    ] {
        assert!(
            !net_problems(a.file, &format!("    {call}\n"), &declared).is_empty(),
            "{call}"
        );
    }
    // a changed line (another family, or a second call on the line) is flagged
    for changed in [
        a.line.replace("AF_UNIX", "AF_INET"),
        format!("{} libc::connect(1, 2, 3);", a.line),
        a.line.replace("SOCK_STREAM", "SOCK_DGRAM"),
    ] {
        assert!(
            !net_problems(a.file, &format!("    {changed}\n"), &declared).is_empty(),
            "{changed}"
        );
    }
}

/// The calls the ticket names are in the list: dropping one makes this fail (and its throwaway snippet too).
#[test]
fn the_network_pattern_lists_keep_the_calls_they_must_catch() {
    for call in [
        "socket",
        "socketpair",
        "connect",
        "bind",
        "listen",
        "accept",
        "sendto",
        "sendmsg",
        "recvfrom",
        "recvmsg",
        "getaddrinfo",
    ] {
        assert!(LIBC_NET_CALLS.contains(&call), "{call}");
    }
    for prefix in ["AF_", "SOCK_", "IPPROTO_", "sockaddr"] {
        assert!(LIBC_NET_PREFIXES.contains(&prefix), "{prefix}");
    }
    for module in ["tokio::net", "mio::net", "socket2", "nix::sys::socket"] {
        assert!(NET_MODULES.contains(&module), "{module}");
    }
    for name in [
        "ToSocketAddrs",
        "lookup_host",
        "getaddrinfo",
        "to_socket_addrs",
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
