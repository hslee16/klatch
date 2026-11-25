//! Every route this crate serves is a row of the authorization audit (#161, epic #152), and no source reads a place a
//! caller could forge or that says only where it is (#110, #161: "no operation trusts the caller's address or process").
//!
//! The audit is one file per row under `proto/audit/` (assembled into `proto/AUTHZ-AUDIT.md` by `proto/scripts/authz-audit`,
//! not committed). `klatch/tests/authz_audit.rs` keeps the wire methods and commands complete; this test does the same for
//! the HTTP routes, which that one cannot see: a path constant or a route arm that no audit row names fails here, so a
//! new route cannot skip the audit.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

fn read_all(dir: &Path, ext: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    walk(dir, ext, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p.display().to_string(), text)
        })
        .collect()
}

fn sources() -> Vec<(String, String)> {
    read_all(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), "rs")
}

/// The text of a source file before its `#[cfg(test)]` module, so test fixtures are not routes.
fn non_test(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or(text)
}

/// The first `"/path"` string literal of a line, if it has one.
fn path_literal(line: &str) -> Option<String> {
    let at = line.find("\"/")?;
    let rest = &line[at + 1..];
    Some(rest.split('"').next()?.to_string())
}

/// Whether a trimmed line declares a route path: `pub const ...PATH: &str = "/x"`.
fn is_route_const(t: &str) -> bool {
    t.starts_with("pub const ") && t.contains("PATH: &str = \"/")
}

/// Whether a trimmed line is a `("METHOD", "/x")` match arm.
fn is_route_arm(t: &str) -> bool {
    const ARMS: &[&str] = &[
        "(\"GET\", \"/",
        "(\"POST\", \"/",
        "(\"PUT\", \"/",
        "(\"DELETE\", \"/",
    ];
    ARMS.iter().any(|a| t.contains(a))
}

/// The route paths a source file declares outside its tests.
fn routes_in(text: &str) -> Vec<String> {
    non_test(text)
        .lines()
        .map(str::trim)
        .filter(|t| is_route_const(t) || is_route_arm(t))
        .filter_map(path_literal)
        .collect()
}

/// Every route path of the crate: the value of each `pub const ...PATH: &str = "/x"` and the literal path of each
/// `("METHOD", "/x")` match arm.
fn routes() -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = sources()
        .into_iter()
        .filter(|(file, _)| !file.ends_with("_tests.rs"))
        .flat_map(|(file, text)| routes_in(&text).into_iter().map(move |p| (file.clone(), p)))
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The audit with the backtick spans of its text: a route is named only when a span holds it, so a path that merely
/// appears in a sentence does not count.
fn audit_spans() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../audit");
    let doc = read_all(&dir, "md")
        .into_iter()
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join("\n");
    doc.split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect()
}

#[test]
fn the_route_scan_finds_the_routes_that_exist() {
    let paths: Vec<String> = routes().into_iter().map(|(_, p)| p).collect();
    for want in ["/v1/stream", "/v1/frames", "/v1/ws", "/mcp", "/github"] {
        assert!(
            paths.iter().any(|p| p == want),
            "{want} not found in {paths:?}"
        );
    }
}

#[test]
fn every_route_is_named_by_a_row_of_the_authorization_audit() {
    let spans = audit_spans();
    let missing: Vec<String> = routes()
        .into_iter()
        .filter(|(_, p)| !spans.iter().any(|s| s.split_whitespace().any(|w| w == p)))
        .map(|(f, p)| format!("{p} (in {f})"))
        .collect();
    assert!(
        missing.is_empty(),
        "routes with no audit row under proto/audit/ (add a row file naming the route in backticks): {missing:?}"
    );
}

/// The one use of a peer address that is not identity (#1493): `loopback::peer_octets` hands the TLS listener the octets
/// of a connection's address so that it can COUNT handshakes per source address. The address is read in the loopback
/// module only (once), and `peer_octets` is named in that module and in `tls_limits.rs` only.
const ADDRESS_READER: &str = "src/loopback.rs";
const OCTETS_USERS: &[&str] = &["src/loopback.rs", "src/tls_limits.rs"];

/// What a caller can forge or that says only where the caller is, none of which may decide who someone is.
const FORBIDDEN: &[&str] = &[
    "x-forwarded",
    "forwarded-for",
    "x-real-ip",
    "x-remote-user",
    "x-client-cert",
    "x-ssl-client",
    "x-client-verify",
    "x-amzn-oidc",
    "peer_addr",
    "connectinfo",
];

/// A line without its `//` comment, lower-cased.
fn code_of(line: &str) -> String {
    line.split("//").next().unwrap_or("").to_ascii_lowercase()
}

/// How many times the token `peer_addr` stands in code (a word boundary on both sides, any form of call, comments
/// stripped): `TcpStream::peer_addr(s)` counts as one, a mention in a comment as none.
fn peer_addr_tokens(text: &str) -> usize {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.lines()
        .map(|line| {
            let code = code_of(line);
            code.match_indices("peer_addr")
                .filter(|(at, m)| {
                    !code[..*at].chars().next_back().is_some_and(word)
                        && !code[at + m.len()..].chars().next().is_some_and(word)
                })
                .count()
        })
        .sum()
}

/// Every place the sources break the rules above: a forwarded or forgeable name anywhere (the file that counts forwarded
/// names excepted), `peer_addr` anywhere but once in the address reader, and `peer_octets` outside its two users.
fn identity_problems(sources: &[(String, String)]) -> Vec<String> {
    let mut problems: Vec<String> = sources
        .iter()
        .filter(|(file, _)| !file.ends_with("forwarded.rs"))
        .flat_map(|(file, text)| {
            text.lines().enumerate().flat_map(move |(n, line)| {
                let code = code_of(line);
                let reader = file.ends_with(ADDRESS_READER);
                let mut found: Vec<String> = FORBIDDEN
                    .iter()
                    .filter(|f| code.contains(**f))
                    .filter(|f| !(**f == "peer_addr" && reader))
                    .map(|f| format!("{file}:{}: names {f}", n + 1))
                    .collect();
                if code.contains("peer_octets") && !OCTETS_USERS.iter().any(|u| file.ends_with(u)) {
                    found.push(format!("{file}:{}: names peer_octets", n + 1));
                }
                found
            })
        })
        .collect();
    for (file, text) in sources {
        let n = peer_addr_tokens(text);
        let want = usize::from(file.ends_with(ADDRESS_READER));
        if n != want && (n > 0 || want > 0) && !file.ends_with("forwarded.rs") {
            problems.push(format!(
                "{file}: reads the peer address {n} time(s), not {want}"
            ));
        }
    }
    problems
}

/// What a caller can forge or that says only where the caller is. None of it may decide who someone is: the one
/// identity is a worker credential (`Authorization: Bearer` with `X-Klatch-Member`), checked live, or the HMAC of a
/// webhook. Behind a proxy these headers are the proxy's word, which klatch does not take (#110).
#[test]
fn no_source_reads_a_forwarded_header_or_trusts_a_peer_address() {
    let problems = identity_problems(&sources());
    assert!(problems.is_empty(), "{problems:?}");
}

/// The sources with `edit` applied to the text of the file that ends with `file`.
fn mutated(file: &str, edit: impl Fn(&str) -> String) -> Vec<(String, String)> {
    let mut all = sources();
    let mut hit = false;
    for (f, text) in &mut all {
        if f.ends_with(file) {
            *text = edit(text);
            hit = true;
        }
    }
    assert!(hit, "no source file {file}");
    all
}

#[test]
fn the_identity_audit_catches_a_second_reader_of_the_peer_address_by_mutation() {
    // M1: another module names `loopback::peer_octets`
    let m1 = mutated("src/wire_https.rs", |t| {
        format!("{t}\nfn leak() {{ let _ = crate::loopback::peer_octets; }}\n")
    });
    assert!(
        identity_problems(&m1)
            .iter()
            .any(|p| p.contains("wire_https.rs") && p.contains("peer_octets")),
        "a module that names peer_octets passed the audit"
    );
    // M2: the address is read a second time in the reader, in another form of call
    let m2 = mutated("src/loopback.rs", |t| {
        format!("{t}\nfn second(s: &TcpStream) {{ let _ = TcpStream::peer_addr(s); }}\n")
    });
    assert!(
        identity_problems(&m2)
            .iter()
            .any(|p| p.contains("loopback.rs")),
        "a second read of the peer address passed the audit"
    );
    // M2b: the one read is removed from the code and only a comment still names it
    let m2b = mutated("src/loopback.rs", |t| {
        t.replace(".peer_addr()", ".local_addr()") + "\n// the old read was sock.peer_addr()\n"
    });
    assert!(
        identity_problems(&m2b)
            .iter()
            .any(|p| p.contains("loopback.rs")),
        "a comment was counted as the one read"
    );
    // a name that merely contains the token is not a read (and `peer_address` is still refused elsewhere)
    assert_eq!(
        peer_addr_tokens("let x = my_peer_addr_2; // peer_addr()"),
        0
    );
    assert_eq!(peer_addr_tokens("a.peer_addr().ok()?; b.peer_addr()"), 2);
}

/// The one file that may spell the forwarded names counts them (#110): it is handed header names only, so it cannot read,
/// store or log a value. Its non-test text names no header type, no value accessor and no output.
#[test]
fn the_forwarded_header_counter_is_handed_names_and_never_a_value() {
    let (_, text) = sources()
        .into_iter()
        .find(|(f, _)| f.ends_with("src/forwarded.rs"))
        .expect("src/forwarded.rs");
    for banned in [
        "Head", ".header(", ".headers", "println", "write!", "Request",
    ] {
        assert!(
            !non_test(&text).contains(banned),
            "src/forwarded.rs must stay name-only, but names {banned}"
        );
    }
    // and its caller passes names, not pairs
    let (_, http) = sources()
        .into_iter()
        .find(|(f, _)| f.ends_with("src/wire_http.rs"))
        .unwrap();
    let squeezed: String = http.split_whitespace().collect();
    assert!(squeezed.contains("forwarded.note(head.headers.iter().map(|(n,_)|n.as_str()))"));
}
