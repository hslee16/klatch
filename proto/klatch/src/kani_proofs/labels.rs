//! Harnesses for the sensitivity label chain.

use crate::domain::{ChainError, check_chain, default_cap_in, dominates_in, min_in};

// ---------------------------------------------------------------------------------------------------------------
// LabelChain::dominates (domain/labels.rs), through `dominates_in`, the function that holds its logic and that
// `dominates` calls: sensitivity labels in a total order, unknown labels fail closed. `LabelChain` itself wraps a heap
// `Vec<String>`, which the solver could not finish (three attempts timed out at 500 to 600 s), so the harness runs the
// same function on a stack array of names.
// Bound: a chain of 2 to 4 names, the first k of public, internal, private, secret; queried labels are those four
// names or one name outside every chain, symbolic.
// Not covered: other chain contents (the names are opaque to the order), `LabelChain::parse` (a `split`, `trim` and
// `collect` into a heap `Vec`, which the solver cannot finish), and the wrappers themselves, each a one-line
// delegation (`new` keeps only the message text for the three `ChainError`s, which a unit test pins).
// `check_chain`, `min_in` and `default_cap_in` are the functions `LabelChain::new`, `min` and `default_cap` call (#658).
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

/// A name for the chain-building harness: three valid names and one invalid (it has a space).
fn any_chain_name() -> &'static str {
    match kani::any::<u8>() % 4 {
        0 => "a",
        1 => "b",
        2 => "c",
        _ => "x y",
    }
}

/// `check_chain` (what `LabelChain::new` runs) accepts a list exactly when it has at least two names, every name is a
/// valid name and none repeats, and names the first fault. Bound: lists of 0 to 4 names drawn from `a`, `b`, `c` and
/// the invalid `x y`, so a repeat is forced for any all-valid list of 4. Not covered: other name text (the valid-name
/// rule has its own harnesses) and lists longer than 4.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn a_chain_needs_two_or_more_valid_distinct_names_and_the_first_fault_is_named() {
    let all = [
        any_chain_name(),
        any_chain_name(),
        any_chain_name(),
        any_chain_name(),
    ];
    let len: usize = kani::any();
    kani::assume(len <= 4);
    let names = &all[..len];
    let bad = |n: &str| n == "x y";
    let first_bad = (0..len).find(|&i| bad(names[i]));
    let first_repeat = (0..len).find(|&i| (0..i).any(|j| names[j] == names[i]));
    let got = check_chain(names);
    if len < 2 {
        assert!(got == Err(ChainError::TooFew));
    } else if let Some(i) = first_bad {
        assert!(got == Err(ChainError::BadName(i)));
    } else if let Some(i) = first_repeat {
        assert!(got == Err(ChainError::Repeat(i)));
    } else {
        assert!(got == Ok(()));
    }
}

/// `min_in` (what `LabelChain::min` calls) is the lower of two known labels and `None` when either is unknown.
/// Bound: the same chains and labels as the dominance harnesses.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn min_is_the_lower_known_label_and_none_for_an_unknown_one() {
    let chain = &NAMES[..chain_len()];
    let (x, y) = (any_label(), any_label());
    let got = min_in(chain, x, y);
    match (position(chain, x), position(chain, y)) {
        (Some(i), Some(j)) => {
            let r = got.unwrap();
            assert!(if i <= j { r == x } else { r == y });
            assert!(dominates_in(chain, x, r) && dominates_in(chain, y, r));
        }
        (None, _) | (_, None) => assert!(got.is_none()),
    }
}

/// `default_cap_in` (what `LabelChain::default_cap` calls) is always a name of the chain: `private` when the chain has
/// it, else the name just below the top. Bound: chains of 2 to 4 names, the first k of the default names.
#[kani::proof]
#[kani::solver(cadical)]
#[kani::unwind(10)]
fn the_default_cap_is_always_a_name_in_the_chain() {
    let chain = &NAMES[..chain_len()];
    let cap = default_cap_in(chain);
    assert!(position(chain, cap).is_some());
    if position(chain, "private").is_some() {
        assert!(cap == "private");
    } else {
        assert!(cap == chain[chain.len() - 2]);
    }
}
