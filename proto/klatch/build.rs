//! Records the commit this binary was built from, for `klatch --version` (#1651).
//!
//! It reads git when git and a checkout are there, and says `unknown` otherwise (a crates.io or tarball build, no git
//! installed). It never fails the build, never touches the network, and reruns when HEAD or the branch it names moves.
//! The dirty marker is computed when the script runs, so it follows commits, not later edits to the working tree.
use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn commit() -> String {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    // A packaged crate carries this file and sits where a parent checkout's git could answer for it: not its commit.
    if dir.is_empty() || Path::new(&dir).join(".cargo_vcs_info.json").exists() {
        return "unknown".into();
    }
    let Some(sha) = git(&["-C", &dir, "rev-parse", "--short=12", "HEAD"]).filter(|s| !s.is_empty())
    else {
        return "unknown".into();
    };
    let dirty = git(&["-C", &dir, "status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|s| !s.is_empty());
    if dirty { format!("{sha}-dirty") } else { sha }
}

fn watch(dir: &str) {
    // In a linked worktree `.git` is a file; ask git where the real paths are.
    if let Some(head) = git(&["-C", dir, "rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(r) = git(&["-C", dir, "symbolic-ref", "-q", "HEAD"])
        && let Some(p) = git(&["-C", dir, "rev-parse", "--git-path", &r])
    {
        println!("cargo:rerun-if-changed={p}");
    }
    if let Some(p) = git(&["-C", dir, "rev-parse", "--git-path", "packed-refs"]) {
        println!("cargo:rerun-if-changed={p}");
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=KLATCH_BUILD_COMMIT");
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        watch(&dir);
    }
    // A packager without git (or with a vendored tree) can state the commit itself; it is data, never a command.
    let c = match std::env::var("KLATCH_BUILD_COMMIT") {
        Ok(v) if !v.trim().is_empty() => v
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect(),
        _ => commit(),
    };
    println!("cargo:rustc-env=KLATCH_BUILD_COMMIT={c}");
}
