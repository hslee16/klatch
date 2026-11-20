//! Harnesses for `valid_name`, `valid_kind`, `valid_scope` and `scopes_collide`.

use super::common::*;
use crate::domain::{
    safe_name_bytes, scope_bytes_ok, scopes_collide, valid_kind, valid_name, valid_scope,
};

// ---------------------------------------------------------------------------------------------------------------
// valid_name, valid_kind, valid_scope (domain/text.rs, domain/scope.rs): the gate against newlines and fake headers.
// Bound: the first group of harnesses draws up to 4 arbitrary ASCII characters and calls the real `valid_*` functions
// (non-ASCII only as concrete cases). The byte-fold harnesses (#657) call `safe_name_bytes` and `scope_bytes_ok`, the
// functions `valid_name`, `valid_kind` and `valid_scope` delegate to, on arrays of 12 arbitrary bytes (non-ASCII
// included), and check the 40 and 80 byte limits at every length from 0 to 90 on text made of `a`.
// Not covered: strings longer than 12 bytes with a mixed alphabet (a limit of the solver; the length harnesses use a
// single letter, so they say nothing about which characters sit in a long string), and the one-line delegation from
// `valid_*` to the byte functions, which only the 4 character harnesses and the unit tests exercise.
// ---------------------------------------------------------------------------------------------------------------

fn any_text<const N: usize>() -> &'static str {
    let bytes: [u8; N] = kani::any();
    kani::assume(bytes.is_ascii());
    let len: usize = kani::any();
    kani::assume(len <= N);
    let leaked: &'static [u8; N] = Box::leak(Box::new(bytes));
    std::str::from_utf8(&leaked[..len]).unwrap()
}

/// Non-ASCII text is never generated above (it costs the solver too much); these concrete cases cover it.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(8)]
fn non_ascii_is_refused() {
    for s in ["\u{e9}", "a\u{e9}", "\u{200b}", "\u{ff21}"] {
        assert!(!valid_name(s) && !valid_kind(s) && !valid_scope(s));
    }
}

fn safe_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(8)]
fn valid_name_accepts_only_the_safe_alphabet() {
    let s = any_text::<4>();
    if valid_name(&s) {
        assert!(!s.is_empty());
        assert!(s.chars().all(safe_char));
        // nothing that could end a line or fake a header
        assert!(!s.contains(['\n', '\r', ':', ' ', '/', '\0']));
    }
    if !s.is_empty() && s.chars().all(safe_char) {
        assert!(valid_name(&s));
    }
}

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(8)]
fn valid_kind_accepts_only_the_safe_alphabet() {
    let s = any_text::<4>();
    if valid_kind(&s) {
        assert!(!s.is_empty());
        assert!(s.chars().all(safe_char));
    }
    if !s.is_empty() && s.chars().all(safe_char) {
        assert!(valid_kind(&s));
    }
}

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(8)]
fn valid_scope_accepts_only_the_safe_alphabet_and_never_the_shared_marker() {
    let s = any_text::<4>();
    if valid_scope(&s) {
        assert!(!s.is_empty());
        assert!(s != "*");
        assert!(
            s.bytes()
                .all(|b| safe_char(b as char) || b == b':' || b == b'/')
        );
    }
}

/// The 40 and 80 byte limits, at the boundary: one length under or at the limit passes, one over fails. One harness
/// per function so each stays small; the text is a concrete run of `a`.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(45)]
fn name_length_limit_holds_at_the_boundary() {
    let s = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(valid_name(&s[..40]) && !valid_name(&s[..41]));
}

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(45)]
fn kind_length_limit_holds_at_the_boundary() {
    let s = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(valid_kind(&s[..40]) && !valid_kind(&s[..41]));
}

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(85)]
fn scope_length_limit_holds_at_the_boundary() {
    let s = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(valid_scope(&s[..80]) && !valid_scope(&s[..81]));
}

/// Up to `N` arbitrary bytes (non-ASCII included), as an array and a length.
fn any_bytes<const N: usize>() -> ([u8; N], usize) {
    let bytes: [u8; N] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= N);
    (bytes, len)
}

/// The name alphabet written by range, a second way, so the harness does not repeat the function's own expression.
fn in_name_set(c: u8) -> bool {
    matches!(c, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.')
}

/// The scope alphabet: the name set, colon and slash.
fn in_scope_set(c: u8) -> bool {
    in_name_set(c) || c == b':' || c == b'/'
}

/// `valid_name` and `valid_kind` (through `safe_name_bytes`) accept exactly the non-empty runs of the safe alphabet.
/// Bound: up to 12 arbitrary bytes, so non-ASCII and control bytes are in the search.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(14)]
fn name_and_kind_bytes_accept_exactly_the_safe_alphabet_up_to_12_bytes() {
    let (bytes, len) = any_bytes::<12>();
    let b = &bytes[..len];
    let mut all_safe = true;
    for c in b {
        if !in_name_set(*c) {
            all_safe = false;
        }
    }
    assert_eq!(safe_name_bytes(b), len >= 1 && all_safe);
}

/// `valid_scope` (through `scope_bytes_ok`) accepts exactly the non-empty runs of the scope alphabet, and never `*`.
/// Bound: up to 12 arbitrary bytes.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(14)]
fn scope_bytes_accept_exactly_the_scope_alphabet_up_to_12_bytes() {
    let (bytes, len) = any_bytes::<12>();
    let b = &bytes[..len];
    let mut all_safe = true;
    for c in b {
        if !in_scope_set(*c) {
            all_safe = false;
        }
    }
    assert_eq!(scope_bytes_ok(b), len >= 1 && all_safe);
    if len == 1 && b[0] == b'*' {
        assert!(!scope_bytes_ok(b));
    }
}

/// The length limits at every length, not only at the boundary: a run of `a` is accepted when its length is 1 to 40
/// (name, kind) or 1 to 80 (scope). Bound: lengths 0 to 90, through the real `valid_*` functions.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(95)]
fn the_length_limits_hold_at_every_length_up_to_90() {
    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let n: usize = kani::any();
    kani::assume(n <= 90);
    let s = &A[..n];
    assert_eq!(valid_name(s), (1..=40).contains(&n));
    assert_eq!(valid_kind(s), (1..=40).contains(&n));
    assert_eq!(valid_scope(s), (1..=80).contains(&n));
}

/// One bad byte anywhere in a run at the length limit is refused, and a good one is accepted: the alphabet property at
/// 40 bytes (name, kind) and 80 bytes (scope) with one symbolic byte at a symbolic position and `a` everywhere else.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(85)]
fn one_symbolic_byte_in_a_full_length_run_decides_the_result() {
    let c: u8 = kani::any();
    let at: usize = kani::any();
    let mut name = [b'a'; 40];
    let mut scope = [b'a'; 80];
    kani::assume(at < 40);
    name[at] = c;
    scope[at] = c;
    assert_eq!(safe_name_bytes(&name), in_name_set(c));
    assert_eq!(scope_bytes_ok(&scope), in_scope_set(c));
}

/// A shared scope collides with every scope and a scope collides with itself, in both directions.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(8)]
fn scopes_collide_is_symmetric_and_shared_collides_with_all() {
    let a = text::<3>(b"ab*");
    let b = text::<3>(b"ab*");
    assert_eq!(scopes_collide(&a, &b), scopes_collide(&b, &a));
    assert!(scopes_collide(&a, &a));
    assert!(scopes_collide(&a, "*") && scopes_collide("*", &a));
    // with neither scope shared, they collide exactly when they are the same scope
    let neither_shared = a != "*" && b != "*";
    if neither_shared {
        assert_eq!(scopes_collide(&a, &b), a == b);
    }
}
