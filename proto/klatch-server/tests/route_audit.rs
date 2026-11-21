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

/// Every route path of the crate: the value of each `pub const ...PATH: &str = "/x"` and the literal path of each
/// `("METHOD", "/x")` match arm.
fn routes() -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (file, text) in sources() {
        if file.ends_with("_tests.rs") {
            continue;
        }
        for line in non_test(&text).lines() {
            let t = line.trim();
            if t.starts_with("pub const ") && t.contains("PATH: &str = \"/") {
                if let Some(p) = path_literal(t) {
                    found.push((file.clone(), p));
                }
            }
            let is_arm = [
                "(\"GET\", \"/",
                "(\"POST\", \"/",
                "(\"PUT\", \"/",
                "(\"DELETE\", \"/",
            ]
            .iter()
            .any(|a| t.contains(a));
            if is_arm {
                if let Some(p) = path_literal(t) {
                    found.push((file.clone(), p));
                }
            }
        }
    }
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
    for want in ["/v1/stream", "/v1/frames", "/mcp", "/github"] {
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

/// What a caller can forge or that says only where the caller is. None of it may decide who someone is: the one
/// identity is a worker credential (`Authorization: Bearer` with `X-Klatch-Member`), checked live, or the HMAC of a
/// webhook. Behind a proxy these headers are the proxy's word, which klatch does not take (#110).
#[test]
fn no_source_reads_a_forwarded_header_or_trusts_a_peer_address() {
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
    let mut problems = Vec::new();
    for (file, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("").to_ascii_lowercase();
            for f in FORBIDDEN {
                if code.contains(f) {
                    problems.push(format!("{file}:{}: names {f}", n + 1));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{problems:?}");
}
