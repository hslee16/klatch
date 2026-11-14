//! The `klatch` crate is one package for the binary and for Rust integrators. The `client` feature must give the
//! protocol and the client without the engine: no SQLCipher, no OpenSSL, no keystore, no command line parser. With
//! no features at all only the dependency-free domain types remain.

use std::process::Command;

fn tree(features: &[&str]) -> String {
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
        .args(features)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "cargo tree {features:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

fn crates(tree: &str) -> Vec<String> {
    tree.lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

const HEAVY: &[&str] = &[
    "rusqlite",
    "libsqlite3-sys",
    "openssl-sys",
    "openssl-src",
    "security-framework",
    "chacha20poly1305",
    "clap",
    "anyhow",
    "libc",
];

#[test]
fn the_client_feature_has_no_engine_dependencies() {
    let deps = crates(&tree(&["--no-default-features", "--features", "client"]));
    for heavy in HEAVY {
        assert!(
            !deps.iter().any(|d| d == heavy),
            "client pulls in {heavy}: {deps:?}"
        );
    }
    for needed in ["serde", "serde_json"] {
        assert!(
            deps.iter().any(|d| d == needed),
            "client needs {needed}: {deps:?}"
        );
    }
}

#[test]
fn with_no_features_only_the_domain_types_remain() {
    let deps = crates(&tree(&["--no-default-features"]));
    assert_eq!(
        deps,
        vec!["klatch".to_string()],
        "no dependencies expected: {deps:?}"
    );
}

#[test]
fn the_default_features_are_the_whole_tool() {
    let deps = crates(&tree(&[]));
    for needed in ["rusqlite", "clap", "serde_json"] {
        assert!(
            deps.iter().any(|d| d == needed),
            "default build needs {needed}: {deps:?}"
        );
    }
}
