//! The tests of `test_dir` (#683, #812, #995). They live in their own file so the crates that include `test_dir.rs`
//! with a path attribute (the binary, every integration test, the bench) do not each run them again.

use super::test_dir::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;
const POLL: Duration = Duration::from_millis(20);
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[test]
fn the_directory_is_removed_when_the_value_is_dropped() {
    let t = TestDir::new("drop-removes");
    let dir = t.to_path_buf();
    std::fs::write(dir.join("rooms.db"), b"data").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("x"), b"more").unwrap();
    assert!(dir.exists());
    drop(t);
    assert!(!dir.exists(), "the directory was left behind: {dir:?}");
}

#[test]
fn the_directory_is_removed_when_the_test_panics() {
    let seen = Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let inner = seen.clone();
    let result = std::panic::catch_unwind(move || {
        let t = TestDir::new("panic-removes");
        *inner.lock().unwrap() = t.to_path_buf();
        std::fs::write(t.join("rooms.db"), b"data").unwrap();
        panic!("a failing test");
    });
    assert!(result.is_err());
    let dir = seen.lock().unwrap().clone();
    assert!(!dir.as_os_str().is_empty());
    assert!(!dir.exists(), "a panic left the directory behind: {dir:?}");
}

#[test]
fn two_directories_with_one_tag_are_different() {
    let (a, b) = (TestDir::new("same"), TestDir::new("same"));
    assert_ne!(&*a, &*b);
    drop(a);
    assert!(b.exists(), "dropping one removed the other");
}

#[test]
fn the_guard_fires_when_the_directory_passes_the_cap_and_not_before() {
    let fired = Arc::new(AtomicBool::new(false));
    let flag = fired.clone();
    let t = TestDir::with_cap("guard-fires", 1024 * 1024, move |_| {
        flag.store(true, Ordering::SeqCst)
    });
    std::fs::write(t.join("under"), vec![0u8; 512 * 1024]).unwrap();
    std::thread::sleep(POLL * 5);
    assert!(!fired.load(Ordering::SeqCst), "fired under the cap");
    std::fs::create_dir(t.join("deep")).unwrap();
    std::fs::write(t.join("deep").join("over"), vec![0u8; 1024 * 1024]).unwrap();
    for _ in 0..200 {
        if fired.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(POLL);
    }
    panic!("the guard did not fire on a directory over the cap");
}

/// #812, #995: a test that makes its own directory under the temp directory cleans it on the happy path only and has no
/// size cap. The unit tests, the integration tests and the bench use `TestDir`; this scan keeps a new helper from going back to the by-hand form.
#[test]
fn no_unit_test_makes_a_temp_directory_by_hand() {
    fn walk(dir: &Path, hits: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, hits);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                if name == "test_dir.rs" || name == "test_dir_tests.rs" {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap();
                for (i, line) in text.lines().enumerate() {
                    // a TMPDIR passed on to a child process and the live jev report file are not scratch directories
                    let allowed = line.contains("\"TMPDIR\"")
                            || line.contains("klatch-jev-live.txt")
                            // the hosted judge's own private working directory, removed on drop (#960)
                            || line.contains("klatch-judge-run-")
                            // the one shared stand-in script of the keyring tests, made once and never a scratch directory
                            || line.contains("klatch-fake-tool-")
                            // places that read where the temp directory is, to scrub it from a message or to check a
                            // path is under it (the bench), and make nothing
                            || line.contains("let tmp = std::env::temp_dir().to_string_lossy()")
                            || line.contains("starts_with(std::env::temp_dir())");
                    if line.contains("temp_dir()") && !allowed {
                        hits.push(format!("{}:{}", p.display(), i + 1));
                    }
                }
            }
        }
    }
    let klatch = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    // the integration tests and the bench crate reach TestDir through a path attribute too (#995)
    for root in [
        klatch.join("src"),
        klatch.join("tests"),
        klatch.join("../klatch-bench/src"),
    ] {
        walk(&root, &mut hits);
    }
    assert!(
        hits.is_empty(),
        "use TestDir instead of temp_dir(): {hits:#?}"
    );
}
