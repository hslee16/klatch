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

#[test]
fn the_rollback_plan_names_how_to_pull_back_each_place_a_release_goes() {
    let doc = read("RELEASING.md");
    assert!(doc.contains("## Rolling back"), "RELEASING.md needs a 'Rolling back' section");
    let plan = doc.split("## Rolling back").nth(1).unwrap();
    for (place, how) in [
        ("crates.io", "cargo yank"),
        ("PyPI", "Yank release"),
        ("npm", "npm deprecate"),
        ("GitHub release", "gh release delete-asset"),
        ("Homebrew", "git revert"),
    ] {
        assert!(plan.contains(place) && plan.contains(how), "the rollback plan must say how to pull back {place} ({how})");
    }
    assert!(plan.contains("Not yet exercised"), "say plainly which of these have not been run");
}
