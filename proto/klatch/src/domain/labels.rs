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
        if names.len() < 2 {
            return Err("a label chain needs at least two levels".into());
        }
        if let Some(bad) = names.iter().find(|n| !valid_name(n)) {
            return Err(format!("bad label name: {bad}"));
        }
        for (i, n) in names.iter().enumerate() {
            if names[..i].contains(n) {
                return Err(format!("label {n} appears twice"));
            }
        }
        Ok(Self(names))
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
        self.0.iter().position(|l| l == label)
    }

    /// Whether `held` is at least `needed`. A name outside the chain is never enough and never satisfied: unknown
    /// labels fail closed.
    pub fn dominates(&self, held: &str, needed: &str) -> bool {
        match (self.rank(held), self.rank(needed)) {
            (Some(h), Some(n)) => h >= n,
            _ => false,
        }
    }

    /// The lower of two labels, `None` if either is unknown.
    pub fn min<'a>(&'a self, a: &'a str, b: &'a str) -> Option<&'a str> {
        let (ra, rb) = (self.rank(a)?, self.rank(b)?);
        Some(if ra <= rb { a } else { b })
    }

    /// The default ceiling for an agent: `private` when the chain has it, else the level just below the top.
    pub fn default_cap(&self) -> &str {
        if self.rank("private").is_some() {
            "private"
        } else {
            &self.0[self.0.len() - 2]
        }
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
}
