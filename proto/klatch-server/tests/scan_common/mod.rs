//! What the scans of `klatch-server`'s source and dependency tree share (`no_network.rs`, `no_network_ws.rs`): the source files
//! under `src/`, the code of a line, and the crate names `cargo tree` prints. Not a test file.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The source scanner shared with the `klatch` crate's guard (`klatch/tests/net_scan/mod.rs`): a tokenizer, so a `//` in a
/// string hides nothing (#1783). Included by path: a test helper, not a dependency.
#[path = "../../../klatch/tests/net_scan/mod.rs"]
pub mod net_scan;

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
            (
                rel,
                net_scan::strip_comments(&std::fs::read_to_string(&p).unwrap()),
            )
        })
        .collect()
}

/// Every Rust file this crate builds or runs: `src` and `tests` (there is no build script), as `(path relative to the
/// crate, text with the comments removed)`.
pub fn all_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in ["src", "tests"] {
        walk(&root.join(d), &mut files);
    }
    if root.join("build.rs").is_file() {
        files.push(root.join("build.rs"));
    }
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(root).unwrap().display().to_string();
            (rel, std::fs::read_to_string(&p).unwrap())
        })
        .collect()
}

/// The code of a line of `rust_sources()`: the line itself, because the comments are already gone (a split at `//` would
/// also cut a line at the `//` of a URL in a string, #1783).
pub fn code_of(line: &str) -> &str {
    line
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

/// Where the crate `ring` is named in `text`, as tokens and not as lines (#1704): the segment that follows each `ring::`
/// (empty when it is a group `ring::{..}` or the end of a path), and one `""` for every other naming of the crate (`use
/// ring;`, `use ring as rr;`, `extern crate ring as rr;`), so a second use on the line of an allowed one is seen.
pub fn ring_paths(text: &str) -> Vec<String> {
    let tokens = tokens_of(text);
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if t != "ring" {
            continue;
        }
        let before = i.checked_sub(1).map(|j| tokens[j].as_str());
        let followed_by_path = tokens.get(i + 1).is_some_and(|n| n == "::");
        if followed_by_path {
            let next = tokens.get(i + 2).map(String::as_str).unwrap_or("");
            let word = next.chars().all(|c| c.is_alphanumeric() || c == '_');
            out.push(if word { next } else { "" }.to_string());
        } else if matches!(before, Some("use" | "crate"))
            || tokens.get(i + 1).is_some_and(|n| n == "as")
        {
            out.push(String::new());
        }
    }
    out
}

/// The tokens of Rust source with comments, string literals and char literals left out: words, `::`, and every other
/// character on its own. Enough to read paths; it parses nothing else.
fn tokens_of(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if ch == '"' {
            i += 1;
            while i < c.len() && c[i] != '"' {
                i += if c[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
        } else if ch == '\'' {
            // a char literal ('x', '\n', '"'); anything else with a quote is a lifetime
            if c.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < c.len() && c[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if c.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if ch.is_alphanumeric() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            let word: String = c[start..i].iter().collect();
            // a raw string: r"..", r#".."#, br".."
            let mut hashes = 0;
            if matches!(word.as_str(), "r" | "br") {
                while c.get(i + hashes) == Some(&'#') {
                    hashes += 1;
                }
            }
            let raw = matches!(word.as_str(), "r" | "br") && c.get(i + hashes) == Some(&'"');
            if word == "r"
                && hashes == 1
                && !raw
                && c.get(i + 1).is_some_and(|x| x.is_alphabetic() || *x == '_')
            {
                // a raw identifier (r#ring): the word after the `r#`, read as itself
                i += 1;
            } else if raw {
                i += hashes + 1;
                while i < c.len()
                    && !(c[i] == '"' && (0..hashes).all(|k| c.get(i + 1 + k) == Some(&'#')))
                {
                    i += 1;
                }
                i += 1 + hashes;
            } else {
                out.push(word);
            }
        } else if ch == ':' && c.get(i + 1) == Some(&':') {
            out.push("::".into());
            i += 2;
        } else {
            if !ch.is_whitespace() {
                out.push(ch.to_string());
            }
            i += 1;
        }
    }
    out
}
