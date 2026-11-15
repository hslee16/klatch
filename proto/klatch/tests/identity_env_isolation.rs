//! #613: an inherited KLATCH_IDENTITY_FILE (a credentialed worker running the suite) must never reach a klatch process
//! a test spawns, or every test that builds its own home would run as that worker. Each test that starts the binary
//! must clear the variable (or set it on purpose) in the same statement. proto/scripts/gate also runs the key suites
//! with a bogus value set, so the behaviour is proven as well as the source shape.

use std::path::Path;

const SPAWN: &str = "Command::new(env!(\"CARGO_BIN_EXE_klatch\"))";

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn every_test_that_spawns_klatch_clears_or_sets_the_identity_file() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"),
        &mut files,
    );
    let mut bad = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for (at, _) in text.match_indices(SPAWN) {
            // a builder chain, or the few lines after `let mut c = Command::new(..);` that configure it
            let mut end = (at + 300).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let statement = &text[at..end];
            let handled = statement.contains("env_remove(\"KLATCH_IDENTITY_FILE\")")
                || statement.contains("env(\"KLATCH_IDENTITY_FILE\"");
            if !handled {
                let line = text[..at].matches('\n').count() + 1;
                bad.push(format!("{}:{line}", f.display()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "spawns that inherit KLATCH_IDENTITY_FILE: {bad:#?}"
    );
}
