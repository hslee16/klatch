//! No source file of this crate grows past the line cap (#1633, as `hub_files_stay_split.rs` does for the library): a new
//! piece goes in a file of its family or a new file, not into one hub again.

use std::path::Path;

const CAP: usize = 600;

#[test]
fn no_source_file_is_over_the_line_cap() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut seen = 0;
    let mut long = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            seen += 1;
            let lines = std::fs::read_to_string(&path).unwrap().lines().count();
            if lines > CAP {
                long.push(format!("{} has {lines} lines", path.display()));
            }
        }
    }
    assert!(seen > 10, "no sources found");
    assert!(long.is_empty(), "the cap is {CAP}: {long:?}");
}
