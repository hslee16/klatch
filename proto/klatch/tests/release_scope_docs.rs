//! The README, SECURITY.md, UNINSTALL.md and DESIGN.md say what the first release is (#1237, #1238, epic #1232): a public
//! local-only pre-release, not a 1.0, not for sensitive data, with klatch-server as a second binary. They must not keep
//! sentences from the time before the daemon and the loopback listeners existed, and the scope, wire, platform and
//! known-limit statements are pinned so a later edit cannot drop one quietly.

use std::path::Path;

/// The file with every run of whitespace collapsed to one space, so a pinned phrase may wrap across lines.
fn read(rel: &str) -> String {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel),
    )
    .unwrap_or_else(|e| panic!("{rel}: {e}"));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lower(rel: &str) -> String {
    read(rel).to_lowercase()
}

#[test]
fn the_user_documents_keep_no_sentence_from_before_the_daemon() {
    for doc in ["README.md", "SECURITY.md", "UNINSTALL.md"] {
        let text = lower(doc);
        for stale in [
            "no daemon",
            "no network transport",
            "opens no socket",
            "nothing to encrypt yet",
            "not yet released",
            "nothing built",
            "nothing is built",
            "no network transport exists yet",
        ] {
            assert!(
                !text.contains(stale),
                "{doc} still says '{stale}', which is not true of what ships"
            );
        }
    }
}

#[test]
fn the_integration_guide_does_not_deny_the_daemon_and_the_loopback_transport() {
    let text = lower("INTEGRATION.md");
    assert!(!text.contains("there is no daemon and no network transport"));
    assert!(text.contains("klatch-server"));
}

#[test]
fn design_no_longer_says_nothing_is_built_or_that_the_store_is_plaintext() {
    let text = lower("DESIGN.md");
    for stale in [
        "nothing is built",
        "stores plaintext",
        "stores and transmits plaintext",
        "transmits plaintext",
        "no public release",
        "the release does not happen until",
    ] {
        assert!(!text.contains(stale), "DESIGN.md still says '{stale}'");
    }
    assert!(
        text.contains("no unmarked release"),
        "the gate is worded as 'no unmarked release' (decision of 2026-11-14, #419)"
    );
}

#[test]
fn the_scope_sentence_is_in_the_readme_and_security() {
    for doc in ["README.md", "SECURITY.md"] {
        let text = lower(doc);
        for phrase in [
            "local-only pre-release",
            "not a 1.0",
            "sensitive data",
            "outside security review",
        ] {
            assert!(text.contains(phrase), "{doc} lacks '{phrase}'");
        }
    }
}

#[test]
fn the_wire_statement_is_stated_honestly_in_both_documents() {
    for doc in ["README.md", "SECURITY.md"] {
        let text = read(doc);
        for phrase in [
            "Remote transports are not part of this release",
            "stdio, a Unix socket",
            "none of these is encrypted",
            "none leaves the machine",
            "a non-loopback listener refuses to start",
            "clients/recipes/behind-a-tunnel.md",
            "never forward or proxy the port",
        ] {
            assert!(text.contains(phrase), "{doc} lacks '{phrase}'");
        }
    }
}

#[test]
fn klatch_server_has_its_own_claim_and_the_klatch_claim_is_kept() {
    let security = read("SECURITY.md");
    assert!(
        security.contains("`klatch-server`")
            && security.contains("loopback listeners only")
            && security.contains("proto/klatch-server/tests/no_network.rs"),
        "SECURITY.md needs a separate klatch-server claim pinned by no_network.rs"
    );
    assert!(
        security.contains("proto/klatch/tests/no_telemetry.rs"),
        "the klatch claim stays and names its test"
    );
}

#[test]
fn the_platforms_are_all_tested_and_the_manual_workflow_is_named() {
    for doc in ["README.md", "SECURITY.md"] {
        let text = read(doc);
        assert!(
            text.contains("manual Platform tests workflow")
                && !text.contains("built and smoke tested"),
            "{doc} must say Linux aarch64 and macOS x86_64 are tested by the manual Platform tests workflow"
        );
    }
    let readme = read("README.md");
    for phrase in [
        "macOS arm64",
        "Linux x86_64",
        "Linux aarch64",
        "macOS x86_64",
        "Windows",
        "unsupported",
    ] {
        assert!(
            readme.contains(phrase),
            "README platform table lacks '{phrase}'"
        );
    }
}

#[test]
fn macos_key_storage_agrees_between_readme_and_security() {
    for doc in ["README.md", "SECURITY.md"] {
        let text = read(doc).replace("On macOS", "on macOS");
        assert!(
            text.contains("on macOS the key is in a file readable only by you"),
            "{doc} must say that on macOS the key is in a file readable only by you (#480)"
        );
    }
}

#[test]
fn the_known_limits_and_the_out_of_scope_list_are_in_the_readme() {
    let text = read("README.md");
    for phrase in [
        "Out of scope",
        "Known limits",
        "xattr -d com.apple.quarantine",
        "Unattributed",
        "#563",
        "#561",
        "#1027",
        "newest release only",
        "Optional tools",
        "proto/scripts/team",
    ] {
        assert!(text.contains(phrase), "README lacks '{phrase}'");
    }
}

#[test]
fn uninstall_removes_both_binaries_the_socket_and_the_daemon() {
    let text = read("UNINSTALL.md");
    for phrase in [
        "klatch-server",
        "--socket",
        "SIGTERM",
        "klatch-server daemon",
        "security delete-generic-password",
    ] {
        assert!(text.contains(phrase), "UNINSTALL.md lacks '{phrase}'");
    }
}

#[test]
fn the_admin_passphrase_is_described_as_one_line_with_a_marked_placeholder() {
    let text = read("SECURITY.md");
    assert!(
        text.contains("PASSPHRASE-PROOF-PLACEHOLDER"),
        "SECURITY.md carries the marked placeholder until #1233 merges (finish it, then delete this check)"
    );
}
