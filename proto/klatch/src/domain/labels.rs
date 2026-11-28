//! Sensitivity labels in one total order (DESIGN D3, ticket #40): `public < internal < private < secret` by
//! default, or any other chain of names an administrator sets. Comparison and dominance only; who holds which
//! label is the gateway's business.

use crate::domain::valid_name;

/// The default order, lowest first.
pub const DEFAULT_LABELS: [&str; 4] = ["public", "internal", "private", "secret"];

/// An ordered chain of label names, lowest first. Names are unique and valid names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelChain(Vec<String>);

impl Default for LabelChain {
    fn default() -> Self {
        Self(DEFAULT_LABELS.iter().map(|l| l.to_string()).collect())
    }
}

impl LabelChain {
    /// A chain of at least two names, lowest first, each a valid name, none repeated.
    pub fn new<S: AsRef<str>>(names: &[S]) -> Result<Self, String> {
        let names: Vec<String> = names.iter().map(|n| n.as_ref().to_string()).collect();
        match check_chain(&names) {
            Ok(()) => Ok(Self(names)),
            Err(ChainError::TooFew) => Err("a label chain needs at least two levels".into()),
            Err(ChainError::BadName(i)) => {
                Err(format!("bad label name: {}", super::shown(&names[i])))
            }
            Err(ChainError::Repeat(i)) => {
                Err(format!("label {} appears twice", super::shown(&names[i])))
            }
        }
    }

    /// Read a chain written `a,b,c` (as `to_text` writes it).
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::new(&text.split(',').map(str::trim).collect::<Vec<_>>())
    }

    pub fn to_text(&self) -> String {
        self.0.join(",")
    }

    pub fn names(&self) -> &[String] {
        &self.0
    }

    pub fn lowest(&self) -> &str {
        &self.0[0]
    }

    pub fn highest(&self) -> &str {
        &self.0[self.0.len() - 1]
    }

    /// Position in the chain, 0 for the lowest; `None` for a name the chain does not have.
    pub fn rank(&self, label: &str) -> Option<usize> {
        rank_in(&self.0, label)
    }

    /// Whether `held` is at least `needed`. A name outside the chain is never enough and never satisfied: unknown
    /// labels fail closed.
    pub fn dominates(&self, held: &str, needed: &str) -> bool {
        dominates_in(&self.0, held, needed)
    }

    /// The lower of two labels, `None` if either is unknown.
    pub fn min<'a>(&'a self, a: &'a str, b: &'a str) -> Option<&'a str> {
        min_in(&self.0, a, b)
    }

    /// The default ceiling for an agent: `private` when the chain has it, else the level just below the top.
    pub fn default_cap(&self) -> &str {
        default_cap_in(&self.0)
    }
}

/// Why a list of names is not a chain. Carries an index, not text, so `check_chain` allocates nothing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChainError {
    /// Fewer than two names.
    TooFew,
    /// The name at this index is not a valid name (the first such).
    BadName(usize),
    /// The name at this index repeats an earlier one (the first such).
    Repeat(usize),
}

/// The rules `LabelChain::new` enforces, over any list of names and without allocating, so the Kani proofs (#658) can
/// run them on a stack array: at least two names, each a valid name, none repeated.
pub(crate) fn check_chain<S: AsRef<str>>(names: &[S]) -> Result<(), ChainError> {
    if names.len() < 2 {
        return Err(ChainError::TooFew);
    }
    if let Some(i) = names.iter().position(|n| !valid_name(n.as_ref())) {
        return Err(ChainError::BadName(i));
    }
    for (i, n) in names.iter().enumerate() {
        if names[..i].iter().any(|m| m.as_ref() == n.as_ref()) {
            return Err(ChainError::Repeat(i));
        }
    }
    Ok(())
}

/// `LabelChain::min` over any list of names.
pub(crate) fn min_in<'a, S: AsRef<str>>(names: &[S], a: &'a str, b: &'a str) -> Option<&'a str> {
    let (ra, rb) = (rank_in(names, a)?, rank_in(names, b)?);
    Some(if ra <= rb { a } else { b })
}

/// `LabelChain::default_cap` over a list of at least two names: `private` when the list has it, else the name just below
/// the top.
pub(crate) fn default_cap_in<S: AsRef<str>>(names: &[S]) -> &str {
    if rank_in(names, "private").is_some() {
        "private"
    } else {
        names[names.len() - 2].as_ref()
    }
}

/// `LabelChain::rank` over any list of names. Free functions on a slice, so the Kani proofs (#404) can run the real
/// logic on a stack array instead of a heap `Vec<String>`, which the solver cannot finish.
fn rank_in<S: AsRef<str>>(names: &[S], label: &str) -> Option<usize> {
    names.iter().position(|l| l.as_ref() == label)
}

/// `LabelChain::dominates` over any list of names.
pub(crate) fn dominates_in<S: AsRef<str>>(names: &[S], held: &str, needed: &str) -> bool {
    match (rank_in(names, held), rank_in(names, needed)) {
        (Some(h), Some(n)) => h >= n,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_chain_is_public_internal_private_secret() {
        let c = LabelChain::default();
        assert_eq!(c.names(), ["public", "internal", "private", "secret"]);
        assert_eq!((c.lowest(), c.highest()), ("public", "secret"));
    }

    #[test]
    fn dominance_follows_the_order_and_is_reflexive() {
        let c = LabelChain::default();
        for (held, needed, ok) in [
            ("secret", "public", true),
            ("private", "private", true),
            ("internal", "private", false),
            ("public", "internal", false),
            ("private", "internal", true),
        ] {
            assert_eq!(c.dominates(held, needed), ok, "{held} vs {needed}");
        }
    }

    #[test]
    fn a_label_outside_the_chain_dominates_nothing_and_is_dominated_by_nothing() {
        let c = LabelChain::default();
        assert!(!c.dominates("topsecret", "public"));
        assert!(!c.dominates("secret", "topsecret"));
        assert!(!c.dominates("topsecret", "topsecret"));
        assert_eq!(c.min("public", "topsecret"), None);
    }

    #[test]
    fn building_a_chain_names_the_first_fault_in_these_words() {
        // pinned before `new` was split into `check_chain` (#658): the words, and which fault comes first
        let few = "a label chain needs at least two levels";
        let cases: [(&[&str], &str); 6] = [
            (&["only"], few),
            (&[], few),
            (&["a", "b c", "d e"], "bad label name: b c"),
            (&["a", "b", "a", "b"], "label a appears twice"),
            (&["a", "b", "b", "a"], "label b appears twice"),
            // a bad name is reported before a repeat, even when the repeat comes first in the list
            (&["a", "a", "b c"], "bad label name: b c"),
        ];
        for (names, words) in cases {
            assert_eq!(LabelChain::new(names).unwrap_err(), words, "{names:?}");
        }
        assert!(LabelChain::new(&["a", "b"]).is_ok());
    }

    #[test]
    fn the_lower_of_two_labels_and_the_default_cap() {
        let c = LabelChain::default();
        let mins = [("secret", "internal"), ("public", "public")];
        for (a, b) in mins {
            assert_eq!(c.min(a, b), Some(if a == "secret" { b } else { a }));
        }
        let chains = [
            (LabelChain::default(), "private"),
            (LabelChain::new(&["open", "closed"]).unwrap(), "open"),
            (LabelChain::new(&["open", "team", "board"]).unwrap(), "team"),
        ];
        for (chain, cap) in &chains {
            assert_eq!(chain.default_cap(), *cap);
        }
    }

    #[test]
    fn a_custom_chain_orders_by_its_own_levels() {
        let c = LabelChain::new(&["open", "team", "board"]).unwrap();
        assert!(c.dominates("board", "open"));
        assert!(!c.dominates("open", "team"));
        assert!(
            !c.dominates("secret", "open"),
            "default names are not in it"
        );
        assert_eq!(c.min("board", "team"), Some("team"));
        assert_eq!(
            c.default_cap(),
            "team",
            "no private level: one below the top"
        );
    }

    #[test]
    fn min_picks_the_lower_level() {
        let c = LabelChain::default();
        assert_eq!(c.min("private", "internal"), Some("internal"));
        assert_eq!(c.min("public", "secret"), Some("public"));
        assert_eq!(c.min("internal", "internal"), Some("internal"));
    }

    #[test]
    fn a_chain_needs_two_distinct_valid_names() {
        assert!(LabelChain::new(&["only"]).is_err());
        assert!(LabelChain::new(&["a", "a"]).unwrap_err().contains("twice"));
        assert!(LabelChain::new(&["a", "b c"]).is_err());
        assert!(LabelChain::new(&["a", ""]).is_err());
        assert!(LabelChain::new::<&str>(&[]).is_err());
    }

    #[test]
    fn a_chain_round_trips_through_its_text() {
        let c = LabelChain::new(&["lo", "mid", "hi"]).unwrap();
        assert_eq!(LabelChain::parse(&c.to_text()).unwrap(), c);
        assert_eq!(LabelChain::parse("a, b").unwrap().names(), ["a", "b"]);
        assert!(LabelChain::parse("a").is_err());
    }

    #[test]
    fn the_default_cap_is_private() {
        assert_eq!(LabelChain::default().default_cap(), "private");
    }

    #[test]
    fn the_default_cap_is_one_below_the_top_for_chains_of_every_length_without_private() {
        for names in [
            vec!["a", "b", "c"],
            vec!["a", "b", "c", "d", "e"],
            vec!["a", "b", "c", "d", "e", "f"],
        ] {
            let c = LabelChain::new(&names).unwrap();
            assert_eq!(c.default_cap(), names[names.len() - 2], "{names:?}");
        }
        let c = LabelChain::new(&["a", "b"]).unwrap();
        assert_eq!(c.default_cap(), "a");
    }
}
