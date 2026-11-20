//! The same properties at larger bounds (#659, #660).

use super::common::*;
use crate::domain::{clean_bytes, overlaps, scopes_collide};

// ---------------------------------------------------------------------------------------------------------------
// Larger bounds (#659, #660). Each of these is one step past a bound the first set stopped at. Not here: a harness
// for the `clean_path` wrapper itself (a `vec!` and `String::from_utf8_lossy`): at 1 byte for the path and the cwd it
// ran 11 minutes without a result and at 2 bytes 19 minutes, so it was removed; RESEARCH-NOTES section 52.8d says so.
// ---------------------------------------------------------------------------------------------------------------

/// The normal-form property one byte past the first set: path and cwd up to 4 bytes from `/ . a`, an output buffer of
/// 10 bytes.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(12)]
fn clean_path_is_normal_up_to_4_bytes() {
    let (path, lp) = bytes_of::<4>(b"/.a");
    let (cwd, lc) = bytes_of::<4>(b"/.a");
    let mut out = [0u8; 10];
    let n = clean_bytes(&path[..lp], &cwd[..lc], &mut out);
    let r = &out[..n];
    assert!(n >= 1 && r[0] == b'/');
    assert!(r.windows(2).all(|w| w != b"//"));
    assert!(n == 1 || r[n - 1] != b'/');
    assert!(no_dot_segments(r));
}

/// `overlaps` is symmetric and reflexive one character past the first set: each path up to 4 characters from `/ a b`.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(14)]
fn overlaps_is_symmetric_and_reflexive_up_to_4_bytes() {
    let a = text::<4>(b"/ab");
    let b = text::<4>(b"/ab");
    assert_eq!(overlaps(a, b), overlaps(b, a));
    assert!(overlaps(a, a));
}

/// A deeper component structure than the first harness (one directory and one child): `/d`, `/d/e`, `/d/e/r`, and the
/// text-prefix sibling `/d/er`, names of one byte (a or b). A directory overlaps everything beneath it, a name that merely
/// continues another is a different component, and a path overlaps its own parent and child.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(14)]
fn overlaps_follows_components_two_levels_deep() {
    let pick = |s: &[u8]| s[kani::any::<usize>() % s.len()];
    let (d, e, r) = (pick(b"ab"), pick(b"ab"), pick(b"ab"));
    let top = std::str::from_utf8(&[b'/', d]).unwrap().to_owned();
    let mid = std::str::from_utf8(&[b'/', d, b'/', e]).unwrap().to_owned();
    let low = std::str::from_utf8(&[b'/', d, b'/', e, b'/', r])
        .unwrap()
        .to_owned();
    let sibling = std::str::from_utf8(&[b'/', d, b'/', e, r])
        .unwrap()
        .to_owned();
    assert!(overlaps(&top, &mid) && overlaps(&mid, &top));
    assert!(overlaps(&top, &low) && overlaps(&low, &top));
    assert!(overlaps(&mid, &low) && overlaps(&low, &mid));
    // `/d/er` is not under `/d/e`, but it is under `/d`
    assert!(!overlaps(&mid, &sibling) && !overlaps(&sibling, &mid));
    assert!(overlaps(&top, &sibling) && overlaps(&sibling, &top));
    assert!(!overlaps(&low, &sibling) && !overlaps(&sibling, &low));
}

/// `scopes_collide` at 5 characters from `a b *`, two past the first set.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(8)]
fn scopes_collide_up_to_5_bytes() {
    let a = text::<5>(b"ab*");
    let b = text::<5>(b"ab*");
    assert_eq!(scopes_collide(a, b), scopes_collide(b, a));
    assert!(scopes_collide(a, a));
    let neither_shared = a != "*" && b != "*";
    if neither_shared {
        assert_eq!(scopes_collide(a, b), a == b);
    }
}
