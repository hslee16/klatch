//! proto/AUTHZ-AUDIT.md lists every way in (#161): each wire method, each command and each `klatch admin` command.
//! This test reads the source and fails when one exists and the audit does not name it, so a new way in cannot skip
//! the audit, as `every_public_method_of_the_gateway_is_classified` does for the gateway's methods.

use std::path::Path;

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn audit() -> String {
    read("../AUTHZ-AUDIT.md")
}

/// `RotateKey` as clap spells it: `rotate-key`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The variants of `enum <name>` in a source file: lines indented four spaces that start with a capital letter,
/// up to the closing brace at the margin.
fn variants(src: &str, name: &str) -> Vec<String> {
    let start = src
        .lines()
        .position(|l| {
            l == format!("enum {name} {{")
                || l == format!("pub enum {name} {{")
                || l == format!("pub(crate) enum {name} {{")
        })
        .unwrap_or_else(|| panic!("enum {name} not found"));
    src.lines()
        .skip(start + 1)
        .take_while(|l| *l != "}")
        .filter(|l| l.starts_with("    ") && !l.starts_with("     "))
        .filter(|l| l.trim_start().starts_with(|c: char| c.is_ascii_uppercase()))
        .map(|l| {
            let word: String = l
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            kebab(&word)
        })
        .collect()
}

/// Every command of the binary: those `Cmd` keeps in `main.rs` and the families in `src/args/` (#860).
fn command_variants() -> Vec<String> {
    // the four flattened families are not commands themselves
    let mut all = variants(&read("src/main.rs"), "Cmd");
    all.retain(|c| !["messaging", "wait", "work", "agents"].contains(&c.as_str()));
    for (file, name) in [
        ("messaging", "MessagingCmd"),
        ("wait", "WaitCmd"),
        ("work", "WorkCmd"),
        ("agents", "AgentsCmd"),
    ] {
        all.extend(variants(&read(&format!("src/args/{file}.rs")), name));
    }
    all
}

/// The method names of the dispatch arms (`"post" => ...`, `"a" | "b" => ...`) in a source file.
fn arms(src: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        let Some((head, _)) = t.split_once(" =>") else {
            continue;
        };
        let names: Vec<&str> = head.split(" | ").collect();
        if !names.iter().all(|n| {
            n.len() > 2
                && n.starts_with('"')
                && n.ends_with('"')
                && n[1..n.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
        }) {
            continue;
        }
        found.extend(names.iter().map(|n| n[1..n.len() - 1].to_string()));
    }
    found
}

fn missing(names: &[String], doc: &str) -> Vec<String> {
    names
        .iter()
        .filter(|n| !doc.contains(&format!("`{n}`")))
        .cloned()
        .collect()
}

#[test]
fn the_parsers_find_what_they_should() {
    let cmds = command_variants();
    for known in ["join", "post", "serve", "worker", "admin", "hook"] {
        assert!(cmds.iter().any(|c| c == known), "{known} not in {cmds:?}");
    }
    let admin = variants(&read("src/admin/mod.rs"), "AdminCmd");
    for known in ["pause", "rotate-key", "room-ask-lease", "review-queue"] {
        assert!(admin.iter().any(|c| c == known), "{known} not in {admin:?}");
    }
    let wire = wire_methods();
    for known in ["post", "claim", "role_act", "subscribe", "unsubscribe"] {
        assert!(wire.iter().any(|c| c == known), "{known} not in {wire:?}");
    }
}

fn wire_methods() -> Vec<String> {
    let mut names = arms(&read("src/session/methods.rs"));
    names.extend(arms(&read("src/session/mod.rs")));
    names.sort();
    names.dedup();
    names
}

#[test]
fn every_wire_method_is_in_the_audit() {
    let gone = missing(&wire_methods(), &audit());
    assert!(
        gone.is_empty(),
        "add these wire methods to AUTHZ-AUDIT.md: {gone:?}"
    );
}

#[test]
fn every_command_is_in_the_audit() {
    let cmds = command_variants();
    let gone = missing(&cmds, &audit());
    assert!(
        gone.is_empty(),
        "add these commands to AUTHZ-AUDIT.md: {gone:?}"
    );
}

#[test]
fn every_admin_command_is_in_the_audit() {
    let cmds = variants(&read("src/admin/mod.rs"), "AdminCmd");
    let gone = missing(&cmds, &audit());
    assert!(
        gone.is_empty(),
        "add these admin commands to AUTHZ-AUDIT.md: {gone:?}"
    );
}
