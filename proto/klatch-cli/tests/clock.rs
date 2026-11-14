//! End-to-end tests: run the real binary against a throwaway KLATCH_HOME.
mod common;

use common::*;
use std::process::Command;

#[cfg(not(feature = "test-clock"))]
#[test]
fn the_real_build_ignores_klatch_now_so_nobody_can_move_its_clock() {
    let h = Home::new();
    // Claiming to be in the year 2001 must change nothing: alice joined a moment ago.
    let who = who_age_with_now(&h, Some("1000000000"));
    assert!(
        who.contains("alice\tlast seen 0s ago") || who.contains("alice\tlast seen 1s ago"),
        "{who}"
    );
    let version = String::from_utf8_lossy(
        &Command::new(env!("CARGO_BIN_EXE_klatch"))
            .arg("--version")
            .output()
            .unwrap()
            .stdout,
    )
    .into_owned();
    assert!(
        !version.contains("test-clock"),
        "a real build says it is a test build: {version}"
    );
}

#[cfg(feature = "test-clock")]
#[test]
fn the_test_build_honours_klatch_now_and_says_it_is_a_test_build() {
    let h = Home::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let who = who_age_with_now(&h, Some(&(now + 700).to_string()));
    assert!(
        who.contains("alice\tlast seen 700s ago") || who.contains("alice\tlast seen 701s ago"),
        "{who}"
    );
    let version = String::from_utf8_lossy(
        &Command::new(env!("CARGO_BIN_EXE_klatch"))
            .arg("--version")
            .output()
            .unwrap()
            .stdout,
    )
    .into_owned();
    assert!(version.contains("test-clock"), "{version}");
}
