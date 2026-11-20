//! Tests for the `NotFound` guards of `FileKeys` that a mutation run found unpinned (#723, from #407): a failure that is
//! not "no such file" must surface as an error, never read as a missing key or a finished removal.

use super::*;

fn scratch(tag: &str) -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new(&format!("keys-survivor-{tag}"))
}

#[test]
fn a_read_that_fails_for_a_reason_other_than_not_found_is_an_error_not_a_missing_key() {
    let d = scratch("notdir");
    // the key directory is a plain file, so looking inside it fails with "not a directory", not "not found"
    let blocker = d.join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    assert!(
        FileKeys::new(&blocker).get("store").is_err(),
        "an unreadable key location read as no key"
    );
}

#[test]
fn a_delete_that_fails_for_a_reason_other_than_not_found_is_an_error() {
    let d = scratch("deldir");
    std::fs::create_dir_all(d.join("store.key")).unwrap();
    assert!(
        FileKeys::new(&d).delete("store").is_err(),
        "a failed removal was reported as done"
    );
    assert!(d.join("store.key").is_dir(), "the directory was touched");
}
