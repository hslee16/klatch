//! What the scans of `klatch-server`'s source and dependency tree share (`no_network.rs`, `no_network_ws.rs`): the source files
//! under `src/`, the code of a line, and the crate names `cargo tree` prints. Not a test file.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn rust_sources() -> Vec<(String, String)> {
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
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap()
                .display()
                .to_string();
            (rel, std::fs::read_to_string(&p).unwrap())
        })
        .collect()
}

/// The code of a line: what is left of it before a `//` comment, and not the text of a string.
pub fn code_of(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

/// The first words of the lines `cargo tree` prints for these arguments (offline, no prefix): crate names.
pub fn tree(args: &[&str]) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .arg("tree")
        .args(args)
        .args(["--prefix", "none", "--offline"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(String::from)
        .collect()
}

/// The names of the crates in klatch-server's dependency tree over the given kinds of edge (`cargo tree`, offline).
pub fn dependency_names(edges: &str) -> Vec<String> {
    tree(&["-p", "klatch-server", "--edges", edges])
}
