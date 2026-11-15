//! Property tests with Hegel (epic #396). Each draws generated cases (100 by default, well under a second in all) and,
//! when one fails, shrinks it to a minimal counterexample and prints how to replay it. They are **evidence, not proof**:
//! a pass means no counterexample was found in the cases tried.
//!
//! Run them alone: `cargo test -p klatch --test properties`. Run more cases:
//! `HEGEL_TEST_CASES=100000 cargo test -p klatch --test properties`. Replay a failure from its blob or seed: see
//! CONTRIBUTING.md. Draw plain integers and map them to values inside the test: Hegel prints and shrinks those without
//! further setup.

use hegel::TestCase;
use hegel::generators as gs;
use klatch::domain::{SHARED_SCOPE, clean_path, overlaps, scopes_collide};
use klatch::protocol::{decode_client_line, decode_server_line};

/// Path segments that hit the interesting cases often: dots, empties, ordinary names and a non-ASCII one. A test draws
/// indices into this list (shrinking toward the first entry).
const SEGMENTS: [&str; 7] = ["a", "b", "..", ".", "", "c.d", "ä"];
const SCOPES: [&str; 5] = ["*", "", "a", "b", "ab"];

fn indices() -> impl hegel::generators::PrintableGenerator<Vec<u8>> {
    gs::vecs(gs::integers::<u8>().max_value(SEGMENTS.len() as u8 - 1))
}

fn path_of(indices: &[u8], rooted: bool) -> String {
    let joined = indices
        .iter()
        .map(|&i| SEGMENTS[i as usize])
        .collect::<Vec<_>>()
        .join("/");
    if rooted { format!("/{joined}") } else { joined }
}

#[hegel::test]
fn a_cleaned_path_is_absolute_has_no_dot_segments_and_cleaning_again_changes_nothing(tc: TestCase) {
    let rooted = tc.draw(gs::booleans());
    let path = path_of(&tc.draw(indices()), rooted);
    let cwd = path_of(&tc.draw(indices()), true);
    let cleaned = clean_path(&path, &cwd);
    assert!(cleaned.starts_with('/'), "not absolute: {cleaned:?}");
    assert!(!cleaned.contains("//"), "doubled slash: {cleaned:?}");
    assert!(
        cleaned.split('/').all(|s| s != "." && s != ".."),
        "dot segment left in {cleaned:?} (from {path:?} in {cwd:?})"
    );
    // an absolute path does not depend on the directory it is cleaned from, and cleaning twice is cleaning once
    assert_eq!(clean_path(&cleaned, "/elsewhere"), cleaned);
    assert_eq!(clean_path(&cleaned, &cwd), cleaned);
    // appending "x/.." to a path changes nothing; this is the property that notices a dropped ".." under some depth.
    // (An empty relative path is the exception to how it is spelled: "" plus "/x/.." would be an absolute path.)
    let longer = if path.is_empty() {
        "x/..".to_string()
    } else {
        format!("{path}/x/..")
    };
    assert_eq!(
        clean_path(&longer, &cwd),
        cleaned,
        "appending x/.. to {path:?}"
    );
}

#[hegel::test]
fn scopes_collide_is_symmetric_reflexive_and_the_shared_scope_collides_with_everything(
    tc: TestCase,
) {
    let pick = |i: u8| SCOPES[i as usize];
    let a = pick(tc.draw(gs::integers::<u8>().max_value(SCOPES.len() as u8 - 1)));
    let b = pick(tc.draw(gs::integers::<u8>().max_value(SCOPES.len() as u8 - 1)));
    assert_eq!(scopes_collide(a, b), scopes_collide(b, a), "{a:?} {b:?}");
    assert!(scopes_collide(a, a), "{a:?} does not collide with itself");
    assert!(scopes_collide(SHARED_SCOPE, b) && scopes_collide(a, SHARED_SCOPE));
    if a != SHARED_SCOPE && b != SHARED_SCOPE && a != b {
        assert!(
            !scopes_collide(a, b),
            "different scopes {a:?} {b:?} collide"
        );
    }
}

#[hegel::test]
fn path_overlap_is_symmetric_and_a_path_overlaps_itself_and_what_is_beneath_it(tc: TestCase) {
    let a = path_of(&tc.draw(indices()), tc.draw(gs::booleans()));
    let b = path_of(&tc.draw(indices()), tc.draw(gs::booleans()));
    assert_eq!(overlaps(&a, &b), overlaps(&b, &a), "{a:?} {b:?}");
    assert!(overlaps(&a, &a), "{a:?}");
    let below = format!("{}/x", a.trim_end_matches('/'));
    assert!(overlaps(&a, &below), "{a:?} vs {below:?}");
}

#[hegel::test]
fn the_decoders_never_panic_and_refuse_what_is_not_a_version_1_frame(tc: TestCase) {
    let line = tc.draw(gs::text());
    // the property is the absence of a panic; an error is the right answer for almost all of these
    let _ = decode_client_line(&line);
    let _ = decode_server_line(&line);
    // a JSON object whose version is not 1 is never accepted
    let n = tc.draw(gs::integers::<u32>());
    tc.assume(n != 1);
    let framed = format!(r#"{{"v":{n},"type":"ping"}}"#);
    assert!(decode_client_line(&framed).is_err(), "accepted {framed}");
    assert!(decode_server_line(&framed).is_err(), "accepted {framed}");
}
