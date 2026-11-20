//! Harnesses for `clean_path` and `overlaps`.

use super::common::*;
use crate::domain::{clean_bytes, overlaps};

// ---------------------------------------------------------------------------------------------------------------
// clean_path (domain/toolguard.rs), through `clean_bytes`, the function that holds its logic: the path a tool call
// touches, with `.` and `..` resolved. `clean_path` itself only sizes a buffer, calls `clean_bytes` and converts the
// bytes to text; that wrapper is not covered (a growing heap string is out of the solver's reach, see RESEARCH-NOTES
// section 52).
// Bound: path up to 3 bytes and cwd up to 3 bytes, each byte one of / . a; an output buffer of 8 bytes; loops
// unwound 10 (20 for the dot-climbing harness, which has longer paths).
// Not covered: longer paths, any other byte (ordinary segment text to this function), the wrapper, symlinks (the
// function is textual by design).
// ---------------------------------------------------------------------------------------------------------------

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn clean_path_is_normal() {
    let (path, lp) = bytes_of::<3>(b"/.a");
    let (cwd, lc) = bytes_of::<3>(b"/.a");
    let mut out = [0u8; 8];
    let n = clean_bytes(&path[..lp], &cwd[..lc], &mut out);
    let r = &out[..n];
    assert!(n >= 1 && r[0] == b'/');
    assert!(r.windows(2).all(|w| w != b"//"));
    assert!(n == 1 || r[n - 1] != b'/');
    assert!(no_dot_segments(r));
}

/// A cleaned path is a fixed point, whatever the cwd. Bound: the same as above.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn clean_path_is_idempotent() {
    let (path, lp) = bytes_of::<3>(b"/.a");
    let (cwd, lc) = bytes_of::<3>(b"/.a");
    let mut out = [0u8; 8];
    let n = clean_bytes(&path[..lp], &cwd[..lc], &mut out);
    let mut again = [0u8; 8];
    let m = clean_bytes(&out[..n], &cwd[..lc], &mut again);
    assert_eq!(&again[..m], &out[..n]);
}

/// `..` can never climb above the root: a path made only of `..` segments is the root, from any cwd.
/// Bound: 2 or 3 `..` segments, written as a relative path; cwd up to 4 bytes.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(20)]
fn clean_path_dots_never_escape_the_root() {
    let (cwd, lc) = bytes_of::<4>(b"/.a");
    let ups: usize = kani::any();
    kani::assume(ups <= 3);
    // a relative path `../..` (no leading slash, or the cwd would be ignored and the check would say nothing)
    let mut path = [0u8; 8];
    let mut at = 0;
    for i in 0..ups {
        if i > 0 {
            path[at] = b'/';
            at += 1;
        }
        path[at] = b'.';
        path[at + 1] = b'.';
        at += 2;
    }
    // enough `..` to climb out of a cwd of up to 4 bytes (at most 2 components) only when `ups` is 2 or more
    kani::assume(ups >= 2);
    let mut out = [0u8; 16];
    let n = clean_bytes(&path[..at], &cwd[..lc], &mut out);
    assert_eq!(&out[..n], b"/");
}

/// `..` removes exactly the last segment, whatever its name: `/x/y/..` is `/x`, and `y/..` from `/x` is `/x`.
/// Bound: segment names of one byte, a or b. Not covered: longer names, deeper paths.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(14)]
fn clean_path_dot_dot_removes_exactly_one_segment() {
    let pick = |s: &[u8]| s[kani::any::<usize>() % s.len()];
    let (x, y) = (pick(b"ab"), pick(b"ab"));
    let mut out = [0u8; 12];
    let n = clean_bytes(&[b'/', x, b'/', y, b'/', b'.', b'.'], b"/", &mut out);
    assert!(out[..n] == [b'/', x]);
    let n = clean_bytes(&[y, b'/', b'.', b'.'], &[b'/', x], &mut out);
    assert!(out[..n] == [b'/', x]);
}

// ---------------------------------------------------------------------------------------------------------------
// overlaps (domain/text.rs): two claim paths conflict when one is a component prefix of the other.
// Bound: symmetry and reflexivity, each path up to 3 characters from / a b; the component harness, one directory
// name and one child name of one character each from a b. Loops unwound 12.
// Not covered: longer paths, characters other than / a b (ordinary to this function), paths that are not normalised
// (the caller cleans them; see clean_path above).
// ---------------------------------------------------------------------------------------------------------------

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(12)]
fn overlaps_is_symmetric_and_reflexive() {
    let a = text::<3>(b"/ab");
    let b = text::<3>(b"/ab");
    assert_eq!(overlaps(&a, &b), overlaps(&b, &a));
    assert!(overlaps(&a, &a));
}

/// A path and anything beneath it overlap; a sibling that only shares a name prefix does not (`/a` and `/ab`).
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(12)]
fn overlaps_is_by_component_not_by_text_prefix() {
    let pick = |s: &[u8]| s[kani::any::<usize>() % s.len()];
    let (d, r) = (pick(b"ab"), pick(b"ab"));
    let base = std::str::from_utf8(&[b'/', d]).unwrap().to_owned();
    let child = std::str::from_utf8(&[b'/', d, b'/', r]).unwrap().to_owned();
    let sibling = std::str::from_utf8(&[b'/', d, r]).unwrap().to_owned();
    assert!(overlaps(&base, &child) && overlaps(&child, &base));
    // a name that merely continues the directory name is a different component
    assert!(!overlaps(&base, &sibling) && !overlaps(&sibling, &base));
    // a trailing slash changes nothing
    assert!(overlaps(&format_slash(&base), &child));
}

fn format_slash(p: &str) -> String {
    let mut s = p.to_owned();
    s.push('/');
    s
}
