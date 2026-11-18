//! Kani bounded proofs of the pure security functions (#404, DESIGN section 32 item 66). Compiled only under
//! `cargo kani` (`cfg(kani)`); not part of any build, test, CI or gate run. Run them with `proto/scripts/kani`.
//!
//! Every harness states its bound. A result is a bounded proof: "no counterexample up to N", never "proven". What a
//! harness leaves out is listed with it, and RESEARCH-NOTES section 52 collects them.

use crate::domain::{
    clean_bytes, clean_path, dominates_in, overlaps, scopes_collide, valid_kind, valid_name,
    valid_scope,
};

/// Text of at most `N` characters drawn from `alphabet`, with a symbolic length. The alphabet is the characters a
/// function treats specially plus one ordinary letter, which is enough to reach every branch of a string function
/// that only looks at those characters. Built from a byte array and borrowed, not grown in a `String`, which costs
/// the solver far more than the function under test.
fn text<const N: usize>(alphabet: &[u8]) -> &'static str {
    let mut bytes = [0u8; N];
    for b in bytes.iter_mut() {
        let i: usize = kani::any();
        kani::assume(i < alphabet.len());
        *b = alphabet[i];
    }
    let len: usize = kani::any();
    kani::assume(len <= N);
    let leaked: &'static [u8; N] = Box::leak(Box::new(bytes));
    std::str::from_utf8(&leaked[..len]).unwrap()
}

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

/// Up to `N` bytes from `alphabet`, as an array and a length.
fn bytes_of<const N: usize>(alphabet: &[u8]) -> ([u8; N], usize) {
    let mut bytes = [0u8; N];
    for b in bytes.iter_mut() {
        let i: usize = kani::any();
        kani::assume(i < alphabet.len());
        *b = alphabet[i];
    }
    let len: usize = kani::any();
    kani::assume(len <= N);
    (bytes, len)
}

fn no_dot_segments(b: &[u8]) -> bool {
    // a segment is `.` or `..` when it sits between `/`, or at either end, with nothing else in it
    let mut start = 0;
    for i in 0..=b.len() {
        if i == b.len() || b[i] == b'/' {
            let seg = &b[start..i];
            if seg == b"." || seg == b".." {
                return false;
            }
            start = i + 1;
        }
    }
    true
}

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

// ---------------------------------------------------------------------------------------------------------------
// valid_name, valid_kind, valid_scope (domain/text.rs, domain/scope.rs): the gate against newlines and fake headers.
// Bound: up to 4 arbitrary ASCII characters (non-ASCII only as concrete cases); the length limits (40 and 80) are checked on the
// boundary lengths only, as concrete strings.
// Not covered: strings between 5 and 39 characters, so the alphabet property is a bounded result and the length
// limits are two spot checks (ticket filed).
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

// ---------------------------------------------------------------------------------------------------------------
// Larger bounds and the clean_path wrapper (#659, #660). Each of these was written to be one step past a bound the
// first set stopped at; where a run did not finish in its time limit the harness was removed and RESEARCH-NOTES
// section 52.8d says so.
// ---------------------------------------------------------------------------------------------------------------

/// The wrapper adds nothing: `clean_path` (a `vec!` of the right size, `clean_bytes`, `String::from_utf8_lossy`) returns
/// the bytes `clean_bytes` computes. Bound: path and cwd up to 2 bytes each from `/ . a`. Not covered: longer paths.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn clean_path_returns_what_clean_bytes_computes() {
    let (path, lp) = bytes_of::<2>(b"/.a");
    let (cwd, lc) = bytes_of::<2>(b"/.a");
    let (p, c) = (
        std::str::from_utf8(&path[..lp]).unwrap(),
        std::str::from_utf8(&cwd[..lc]).unwrap(),
    );
    let got = clean_path(p, c);
    let mut out = [0u8; 6];
    let n = clean_bytes(&path[..lp], &cwd[..lc], &mut out);
    assert!(got.as_bytes() == &out[..n]);
}

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

// ---------------------------------------------------------------------------------------------------------------
// LabelChain::dominates (domain/labels.rs), through `dominates_in`, the function that holds its logic and that
// `dominates` calls: sensitivity labels in a total order, unknown labels fail closed. `LabelChain` itself wraps a heap
// `Vec<String>`, which the solver could not finish (three attempts timed out at 500 to 600 s), so the harness runs the
// same function on a stack array of names.
// Bound: a chain of 2 to 4 names, the first k of public, internal, private, secret; queried labels are those four
// names or one name outside every chain, symbolic.
// Not covered: other chain contents (the names are opaque to the order), `LabelChain::new` and `parse` (uniqueness
// and name checks), the wrapper methods `rank`, `min` and `default_cap`.
// ---------------------------------------------------------------------------------------------------------------

const NAMES: [&str; 4] = ["public", "internal", "private", "secret"];

fn any_label() -> &'static str {
    match kani::any::<u8>() % 5 {
        0 => "public",
        1 => "internal",
        2 => "private",
        3 => "secret",
        _ => "outside",
    }
}

fn chain_len() -> usize {
    let k: usize = kani::any();
    kani::assume((2..=4).contains(&k));
    k
}

fn position(chain: &[&str], l: &str) -> Option<usize> {
    chain.iter().position(|n| *n == l)
}

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn dominance_is_reflexive_total_and_antisymmetric_and_fails_closed() {
    let chain = &NAMES[..chain_len()];
    let (x, y) = (any_label(), any_label());
    let known = position(chain, x).is_some();
    // reflexive on known labels only; an unknown label dominates nothing, itself included
    assert!(dominates_in(chain, x, x) == known);
    if known && position(chain, y).is_some() {
        // total, and antisymmetric
        assert!(dominates_in(chain, x, y) || dominates_in(chain, y, x));
        if dominates_in(chain, x, y) && dominates_in(chain, y, x) {
            assert!(x == y);
        }
    } else {
        // fail closed on either side
        assert!(!dominates_in(chain, x, y));
    }
    // the top of the chain dominates every known label, the bottom only itself
    if known {
        assert!(dominates_in(chain, chain[chain.len() - 1], x));
        assert!(dominates_in(chain, chain[0], x) == (x == chain[0]));
    }
}

#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn dominance_is_transitive() {
    let chain = &NAMES[..chain_len()];
    let (x, y, z) = (any_label(), any_label(), any_label());
    if dominates_in(chain, x, y) && dominates_in(chain, y, z) {
        assert!(dominates_in(chain, x, z));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// is_loopback and check_bind (session/policy.rs): a plaintext listener only on loopback and only with a token.
// Bound: none on the address (all 2^32 IPv4 and all 2^128 IPv6 addresses, any port); the Security and token inputs
// are exhausted.
// Not covered: whether a transport calls check_bind at all (a caller property), name resolution.
// ---------------------------------------------------------------------------------------------------------------

#[cfg(feature = "cli")]
mod session_proofs {
    use crate::session::policy::{Refusal, Security, check_bind, is_loopback};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    fn any_ip() -> IpAddr {
        if kani::any() {
            IpAddr::V4(Ipv4Addr::from(kani::any::<[u8; 4]>()))
        } else {
            IpAddr::V6(Ipv6Addr::from(kani::any::<[u16; 8]>()))
        }
    }

    #[kani::proof]
    #[kani::solver(cadical)]
    fn loopback_is_exactly_127_slash_8_and_the_ipv6_forms() {
        let ip = any_ip();
        let expected = match ip {
            IpAddr::V4(v) => v.octets()[0] == 127,
            IpAddr::V6(v) => {
                let s = v.segments();
                let v6_loopback = s == [0, 0, 0, 0, 0, 0, 0, 1];
                let mapped_127 = s[..5] == [0; 5] && s[5] == 0xffff && (s[6] >> 8) == 127;
                v6_loopback || mapped_127
            }
        };
        assert_eq!(is_loopback(&ip), expected);
        // the addresses that listen on every interface are never loopback
        assert!(!is_loopback(&IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert!(!is_loopback(&IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
    }

    #[kani::proof]
    #[kani::solver(cadical)]
    fn check_bind_allows_plaintext_only_on_loopback_with_a_token() {
        let addr = SocketAddr::new(any_ip(), kani::any());
        let token: bool = kani::any();
        let security = match kani::any::<u8>() % 3 {
            0 => Security::Plaintext,
            1 => Security::MutualTls,
            _ => Security::Ssh,
        };
        let result = check_bind(&addr, security, token);
        if security == Security::Plaintext {
            assert_eq!(result.is_ok(), is_loopback(&addr.ip()) && token);
            if !is_loopback(&addr.ip()) {
                assert_eq!(result, Err(Refusal::PlaintextOffLoopback(addr)));
            }
        } else {
            assert!(result.is_ok());
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The sealed header (keys/sealed.rs) and the recovery code (keys/recovery.rs).
// ---------------------------------------------------------------------------------------------------------------

#[cfg(feature = "cli")]
mod key_proofs {
    use crate::keys::{KEY_LEN, Key, RevealError, is_sealed_text, looks_protected, reveal};

    /// Header parse. Bound: blobs of 0 to 30 bytes (the header is 28), every byte arbitrary.
    /// Not covered: blobs longer than 30 bytes, and whether a well-formed header opens (that is the AEAD, outside
    /// the reach of a model checker; the authenticated-encryption property tests cover it).
    #[kani::proof]
    #[kani::solver(cadical)]
    #[kani::unwind(40)]
    fn a_blob_is_protected_only_with_the_full_header_and_the_magic() {
        let bytes: [u8; 30] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 30);
        let blob = &bytes[..len];
        assert_eq!(looks_protected(blob), len >= 28 && blob[..2] == *b"KS");
        if len < 28 {
            // short input is refused before any byte past the header is read: no panic, one error
            let key = Key::from_bytes([0; KEY_LEN]);
            assert_eq!(reveal(&key, b"", blob), Err(RevealError::Malformed));
        }
    }

    /// An unknown version or algorithm is refused by name, before the cipher runs.
    #[kani::proof]
    #[kani::solver(cadical)]
    #[kani::unwind(40)]
    fn an_unknown_version_or_algorithm_is_refused_before_decryption() {
        let mut bytes: [u8; 30] = kani::any();
        bytes[0] = b'K';
        bytes[1] = b'S';
        let (v, a) = (bytes[2], bytes[3]);
        kani::assume(!(v == 1 && a == 1));
        let key = Key::from_bytes([0; KEY_LEN]);
        let want = if v == 1 {
            RevealError::UnknownAlgorithm(a)
        } else {
            RevealError::UnknownVersion(v)
        };
        assert_eq!(reveal(&key, b"", &bytes), Err(want));
    }

    /// The text form is recognised by its prefix alone.
    /// Bound: twelve concrete texts, no symbolic input: the exact prefix, with and without a body after it, and texts
    /// that differ from it in the first byte, a middle byte, the version byte, the last colon, by being one byte short,
    /// by being the empty string, and by holding the prefix after other text. Not covered: any other text. Concrete
    /// because validating symbolic bytes as UTF-8 did not finish in 600 s (sixteen bytes, one byte at a symbolic
    /// position, or forty-eight concrete validations), and `from_utf8_unchecked` is refused by the unsafe-code check
    /// in tests/hygiene.rs.
    #[kani::proof]
    #[kani::solver(cadical)]
    #[kani::unwind(40)]
    fn sealed_text_is_recognised_by_its_prefix() {
        for yes in [
            "klatch-sealed:1:",
            "klatch-sealed:1:AAAA",
            "klatch-sealed:1:klatch-sealed:1:",
        ] {
            assert!(is_sealed_text(yes));
        }
        for no in [
            "",
            "xlatch-sealed:1:",
            "klatch-xealed:1:",
            "klatch-sealed:2:",
            "klatch-sealed:1x",
            "klatch-sealed:1",
            "klatch-sealed:",
            " klatch-sealed:1:",
            "a message that mentions klatch-sealed:1: later",
        ] {
            assert!(!is_sealed_text(no));
        }
    }

    // The recovery code (keys/recovery.rs) has no harness: `restore` and `recovery_code` build and filter heap strings,
    // decode base32 into a `Vec` and hash with SHA-256, and a harness over input of only 2 characters (far short of
    // the 63 a real code has) still ran past 500 s. RESEARCH-NOTES section 52 and the ticket it names say what was
    // tried; the unit tests in keys/recovery.rs are the only cover for it.
}
