//! The files every pull request edits were split by family so parallel pull requests stop colliding (#860, #533).
//! This keeps them split: no file under those directories grows past the cap, so a new command goes in the file of
//! its family or a new file, not into one hub again.

use std::path::{Path, PathBuf};

const CAP: usize = 600;

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_split_directories_have_no_file_over_the_line_cap() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for dir in ["commands", "args", "kani_proofs"] {
        rust_files(&src.join(dir), &mut files);
    }
    assert!(
        !files.is_empty(),
        "no files found under the split directories"
    );
    let long: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let lines = std::fs::read_to_string(f).unwrap().lines().count();
            (lines > CAP).then(|| format!("{} has {lines} lines", f.display()))
        })
        .collect();
    assert!(
        long.is_empty(),
        "split by family, the cap is {CAP}: {long:?}"
    );
}

#[test]
fn the_old_commands_file_is_gone() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        !src.join("commands.rs").exists(),
        "commands.rs is the directory commands/ now"
    );
}
