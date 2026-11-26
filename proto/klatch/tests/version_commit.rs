//! `klatch --version` names the commit the binary was built from (#1651), so a bench run can record what it ran.
use std::process::Command;

fn version_line() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_klatch"))
        .env_remove("KLATCH_IDENTITY_FILE")
        .arg("--version")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The commit field: a short hex sha with an optional `-dirty` marker, or `unknown` when git was not available.
fn commit_field(line: &str) -> String {
    let rest = line
        .split("(commit ")
        .nth(1)
        .unwrap_or_else(|| panic!("no commit in: {line}"));
    rest.split([',', ')']).next().unwrap().to_string()
}

#[test]
fn version_names_the_build_commit_and_keeps_the_leading_version() {
    let line = version_line();
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        line.starts_with(&format!("klatch {version} (commit ")),
        "{line}"
    );
    assert!(
        line.contains(", protocol ") && line.contains(", store schema "),
        "{line}"
    );
    let c = commit_field(&line);
    let sha = c.strip_suffix("-dirty").unwrap_or(&c);
    let ok = sha == "unknown" || (sha.len() >= 7 && sha.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(ok, "bad commit field {c:?} in: {line}");
}

#[test]
fn in_a_git_checkout_the_commit_is_the_real_head() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let head = Command::new("git")
        .args(["-C", manifest, "rev-parse", "--short=12", "HEAD"])
        .output();
    let Ok(head) = head else { return };
    if !head.status.success() {
        return;
    }
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let c = commit_field(&version_line());
    let sha = c.strip_suffix("-dirty").unwrap_or(&c);
    assert!(
        head.starts_with(sha) || sha.starts_with(&head),
        "head {head} but --version says {c}"
    );
}
