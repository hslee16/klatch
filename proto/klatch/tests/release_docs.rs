//! The release documents cannot drift from the code (#433): VERSIONING.md states the protocol and store schema
//! versions this build has, and CHANGELOG.md has an entry for this version.

use std::path::Path;

fn read(rel: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel),
    )
    .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn versioning_md_states_the_current_protocol_and_schema_versions() {
    let doc = read("VERSIONING.md");
    let protocol = format!(
        "Wire protocol version: {}",
        klatch::protocol::PROTOCOL_VERSION
    );
    let schema = format!("Store schema version: {}", klatch::store::SCHEMA_VERSION);
    assert!(
        doc.contains(&protocol),
        "VERSIONING.md must say '{protocol}'"
    );
    assert!(doc.contains(&schema), "VERSIONING.md must say '{schema}'");
}

#[test]
fn the_changelog_has_an_entry_for_this_version() {
    let log = read("CHANGELOG.md");
    assert!(
        log.contains("## [Unreleased]"),
        "Keep a Changelog keeps an Unreleased section"
    );
    let heading = format!("## [{}]", env!("CARGO_PKG_VERSION"));
    assert!(
        log.contains(&heading),
        "CHANGELOG.md has no '{heading}' entry"
    );
}

#[test]
fn the_release_checklist_names_every_place_a_version_lives() {
    let doc = read("RELEASING.md");
    for place in [
        "proto/Cargo.toml",
        "clients/python/pyproject.toml",
        "clients/python/package/wheel_init.py",
        "clients/typescript/package.json",
        "README.md",
        "Formula/klatch.rb",
    ] {
        assert!(doc.contains(place), "RELEASING.md does not mention {place}");
    }
}
