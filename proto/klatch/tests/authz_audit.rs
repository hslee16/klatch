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

/// The first cell of every table row, per section (a `##` heading starts a new one; a header row and the separator
/// are skipped), as `(section, cell)`. A union merge (#1065) keeps both sides' lines, so two branches that each
/// edited the same row leave two copies of it; the rows of a section are unique by this cell.
fn row_keys(doc: &str) -> Vec<(String, String)> {
    let mut section = String::new();
    let mut keys = Vec::new();
    for line in doc.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            section = h.to_string();
        } else if line.starts_with('|') {
            let cell = line
                .trim_start_matches('|')
                .split('|')
                .next()
                .unwrap_or("")
                .trim();
            if cell.starts_with('`') {
                keys.push((section.clone(), cell.to_string()));
            }
        }
    }
    keys
}

/// The keys that occur more than once in one section.
fn duplicate_rows(doc: &str) -> Vec<(String, String)> {
    let keys = row_keys(doc);
    let mut dups: Vec<(String, String)> = keys
        .iter()
        .filter(|k| keys.iter().filter(|o| o == k).count() > 1)
        .cloned()
        .collect();
    dups.sort();
    dups.dedup();
    dups
}

#[test]
fn a_row_seen_twice_is_found() {
    let doc = "## A\n\n| Op | x |\n|---|---|\n| `one` | a |\n| `two` | b |\n| `one` | c |\n\n## B\n\n| `one` | d |\n";
    assert_eq!(
        duplicate_rows(doc),
        vec![("A".to_string(), "`one`".to_string())],
        "the same key in another section is not a duplicate"
    );
    assert!(duplicate_rows("## A\n| `one` | a |\n| `two` | b |\n").is_empty());
}

#[test]
fn audit_rows_are_unique_by_operation() {
    assert!(
        row_keys(&audit()).len() > 50,
        "the row parser found almost no rows"
    );
    let dups = duplicate_rows(&audit());
    assert!(
        dups.is_empty(),
        "AUTHZ-AUDIT.md has a row twice (a union merge of two edits of one row keeps both; keep the right one and \
         delete the other): {dups:?}"
    );
}

/// The key of every row of the "Gaps found and ticketed" table (#1072). The first cell is prose, so the key is the
/// FIRST ticket number in the last cell: a gap that says `#835` and the same gap reworded as `fixed in #835 (...)`
/// are one gap. A row with no ticket number is keyed by its whole last cell. A union merge (#1065) of two edits of
/// one gap row keeps the stale and the new text, and they share this key.
///
/// No allowlist is needed today: a row that names a second ticket (the `release_crossings` row names #834 after its
/// own #836) is keyed by its first. If a future row must share its first ticket with another gap, list that key in
/// `SHARED_GAP_KEYS` with the reason; never loosen the rule for every row.
const SHARED_GAP_KEYS: &[&str] = &[];

fn gap_keys(doc: &str) -> Vec<String> {
    let mut in_gaps = false;
    let mut keys = Vec::new();
    for line in doc.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            in_gaps = h.starts_with("Gaps found and ticketed");
        } else if in_gaps && line.starts_with('|') {
            let last = line
                .trim()
                .trim_end_matches('|')
                .rsplit('|')
                .next()
                .unwrap_or("")
                .trim();
            if last == "Ticket" || last.starts_with("---") {
                continue;
            }
            let key = last
                .find('#')
                .and_then(|i| {
                    let digits: String = last[i + 1..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    (!digits.is_empty()).then(|| format!("#{digits}"))
                })
                .unwrap_or_else(|| last.to_string());
            keys.push(key);
        }
    }
    keys
}

fn duplicate_gaps(doc: &str) -> Vec<String> {
    let keys = gap_keys(doc);
    let mut dups: Vec<String> = keys
        .iter()
        .filter(|k| {
            keys.iter().filter(|o| o == k).count() > 1 && !SHARED_GAP_KEYS.contains(&k.as_str())
        })
        .cloned()
        .collect();
    dups.sort();
    dups.dedup();
    dups
}

#[test]
fn a_gap_row_seen_twice_is_found() {
    let doc = "## Gaps found and ticketed\n\n| Gap | Ticket |\n|---|---|\n| old words | #835 |\n| other | #836, #834 |\n| new words | fixed in #835 (`x`) |\n| a | fixed here (`f`) |\n| b | fixed here (`g`) |\n\n## Other\n\n| x | #835 |\n";
    assert_eq!(duplicate_gaps(doc), vec!["#835".to_string()]);
    assert!(
        duplicate_gaps(
            "## Gaps found and ticketed\n| Gap | Ticket |\n|---|---|\n| a | #1 |\n| b | #2, #1 |\n"
        )
        .is_empty(),
        "a second ticket in the last cell does not key the row"
    );
    let same = "## Gaps found and ticketed\n| a | fixed here (`f`) |\n| b | fixed here (`f`) |\n";
    assert_eq!(duplicate_gaps(same), vec!["fixed here (`f`)".to_string()]);
}

#[test]
fn gap_rows_are_unique_by_ticket() {
    assert!(
        gap_keys(&audit()).len() >= 10,
        "the gap parser found almost no rows"
    );
    let dups = duplicate_gaps(&audit());
    assert!(
        dups.is_empty(),
        "AUTHZ-AUDIT.md has a gap twice (a union merge of two edits of one gap keeps the stale and the new text; \
         keep the new row and delete the stale one): {dups:?}"
    );
}

/// The real #1065 replay: `git merge-file --union` of the recorded #1028/#1029 conflict (commit 62157c2, parents
/// a790555 and bae6eef). Both sides edited the #835 and #836 gap rows, so each shows twice.
#[test]
fn the_replayed_union_merge_of_two_gap_edits_is_caught() {
    let replayed = read("tests/fixtures/authz-audit-replayed-union.md");
    assert_eq!(
        duplicate_gaps(&replayed),
        vec!["#835".to_string(), "#836".to_string()]
    );
}
