//! Hegel against a hand-rolled random loop (the in-tree fuzz style), finding planted bugs in a copy of `clean_path`
//! (epic #396, bench #390). Both get the same property. Run with `-- --nocapture` to see the numbers.

use hegel::generators as gs;
use hegel::{Database, Hegel, Settings, TestCase};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};

/// `clean_path` from klatch-domain with one planted bug each.
fn clean(path: &str, cwd: &str, bug: u8) -> String {
    let joined = if path.starts_with('/') { path.to_string() } else { format!("{cwd}/{path}") };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            // bug 1: a ".." at the root is kept instead of dropped
            ".." if bug == 1 && parts.is_empty() => parts.push(".."),
            // bug 2: a ".." under exactly three directories is ignored
            ".." if bug == 2 && parts.len() == 3 => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The property: no ".." segment survives, and appending "/x/.." to a path changes nothing (a stronger property is
/// what notices bug 2: ignoring a ".." leaves no ".." behind, so the first half alone cannot see it).
fn violates(path: &str, cwd: &str, bug: u8) -> bool {
    let cleaned = clean(path, cwd, bug);
    cleaned.split('/').any(|s| s == "..") || clean(&format!("{path}/x/.."), cwd, bug) != cleaned
}

const SEGMENTS: [&str; 7] = ["a", "b", "..", ".", "", "c.d", "ä"];

fn join(indices: &[u8]) -> String {
    indices.iter().map(|&i| SEGMENTS[i as usize]).collect::<Vec<_>>().join("/")
}

/// Cases Hegel ran before the property first failed (None: it never failed), and the shrunk counterexample it printed.
fn hegel_structured(bug: u8, cases: u64) -> Option<usize> {
    let calls = AtomicUsize::new(0);
    let first = AtomicUsize::new(0);
    let result = catch_unwind(AssertUnwindSafe(|| {
        Hegel::new(|tc: TestCase| {
            let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
            let path = join(&tc.draw(gs::vecs(gs::integers::<u8>().max_value(6))));
            if violates(&path, "/", bug) {
                let _ = first.compare_exchange(0, n, Ordering::SeqCst, Ordering::SeqCst);
                panic!("clean_path kept a '..' segment for {path:?}");
            }
        })
        .settings(Settings::new().test_cases(cases).database(Database::Disabled).seed(Some(7)))
        .run();
    }));
    result.is_err().then(|| first.load(Ordering::SeqCst))
}

/// The same property with Hegel's plain `text()` generator and nothing structured.
fn hegel_text(bug: u8, cases: u64) -> Option<usize> {
    let calls = AtomicUsize::new(0);
    let first = AtomicUsize::new(0);
    let result = catch_unwind(AssertUnwindSafe(|| {
        Hegel::new(|tc: TestCase| {
            let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
            let path = tc.draw(gs::text());
            if violates(&path, "/", bug) {
                let _ = first.compare_exchange(0, n, Ordering::SeqCst, Ordering::SeqCst);
                panic!("clean_path kept a '..' segment for {path:?}");
            }
        })
        .settings(Settings::new().test_cases(cases).database(Database::Disabled).seed(Some(7)))
        .run();
    }));
    result.is_err().then(|| first.load(Ordering::SeqCst))
}

/// The in-tree fuzz style: a xorshift generator over a small alphabet, no shrinking.
fn xorshift(bug: u8, iterations: usize) -> Option<usize> {
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let alphabet: Vec<char> = "ab./".chars().collect();
    for n in 1..=iterations {
        let len = (next() % 13) as usize;
        let path: String = (0..len).map(|_| alphabet[(next() % 4) as usize]).collect();
        if violates(&path, "/", bug) {
            return Some(n);
        }
    }
    None
}

#[test]
fn hegel_finds_both_planted_bugs_and_the_numbers_are_printed() {
    for bug in [1u8, 2] {
        let hegel = hegel_structured(bug, 5_000);
        let text = hegel_text(bug, 5_000);
        let fuzz = xorshift(bug, 200_000);
        println!(
            "bug {bug}: hegel (structured) failed at case {hegel:?}; hegel (plain text) {text:?}; xorshift (200000 tries) {fuzz:?}"
        );
        assert!(hegel.is_some(), "hegel did not find planted bug {bug} in 5000 cases");
    }
}
