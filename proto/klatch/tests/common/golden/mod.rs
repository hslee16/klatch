//! Golden (snapshot) files for the security tests (#1480): the observable text of the admin proof, the key custody
//! and the consent hooks is committed under `tests/golden/` and compared byte for byte, so a refactor of that code
//! cannot change a prompt, a refusal or an audit row without a test saying so.
//!
//! - `check(name, text)` compares `text` to `tests/golden/<name>.txt` and fails with a line diff.
//! - Regenerate only on purpose: `KLATCH_WRITE_GOLDEN=1 cargo test -p klatch --test <file>`, then read the diff in git.
//! - `Scrub` rewrites what changes from run to run (the throwaway home, the binary path, process ids, terminal labels,
//!   times and random names) to fixed words, so a second run gives the same text.
//! - A golden holds prompts, messages and counts only. `check` refuses to write or compare a text that holds a secret
//!   the test typed (`forbid`), so a passphrase can never be committed by regenerating.
#![allow(dead_code)]

mod diff;
mod scrub;
mod sheet;

#[allow(unused_imports)]
pub use diff::diff;
#[allow(unused_imports)]
pub use scrub::Scrub;
#[allow(unused_imports)]
pub use sheet::Sheet;

use std::path::{Path, PathBuf};

/// Where the golden files are.
fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// Whether this run rewrites the goldens instead of comparing (only with `KLATCH_WRITE_GOLDEN=1`).
fn writing() -> bool {
    std::env::var("KLATCH_WRITE_GOLDEN").is_ok_and(|v| v == "1")
}

/// Fail when `text` holds any of the secrets the test typed.
#[track_caller]
pub fn forbid(text: &str, secrets: &[&str]) {
    for s in secrets {
        assert!(
            !s.is_empty() && !text.contains(s),
            "a golden must never hold a typed secret (found one, {} bytes long)",
            s.len()
        );
    }
}

/// Compare `actual` to the committed golden `name` (or rewrite it with `KLATCH_WRITE_GOLDEN=1`); `forbidden` are the
/// secrets the test typed, which must not appear.
#[track_caller]
pub fn check(name: &str, actual: &str, forbidden: &[&str]) {
    forbid(actual, forbidden);
    assert!(
        !actual.contains('\u{2014}'),
        "{name}: an em dash in a golden"
    );
    let path = dir().join(format!("{name}.txt"));
    if writing() {
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "golden {} is missing ({e}); write it with KLATCH_WRITE_GOLDEN=1",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "golden {name} differs (- committed, + this run; regenerate on purpose with KLATCH_WRITE_GOLDEN=1):\n{}",
        diff(&expected, actual)
    );
}
