//! The feature declaration (#485): a store that does not declare what the gateway relies on is refused at wiring.

use super::*;
use crate::store::memory_spike::MemoryStore;
use crate::store::{Feature, Features};

#[test]
fn a_store_with_only_the_required_methods_declares_nothing_and_is_refused() {
    let s = MemoryStore::new();
    assert_eq!(s.features(), Features::none());
    let err = crate::store::require_features(&s).unwrap_err().to_string();
    for f in Feature::ALL {
        assert!(err.contains(f.name()), "{} missing from: {err}", f.name());
    }
}

#[test]
fn a_partial_declaration_names_only_what_is_missing() {
    let f = Features::none().with(Feature::Mute).with(Feature::Asks);
    let missing = f.missing();
    assert!(!missing.contains(&Feature::Mute) && !missing.contains(&Feature::Asks));
    assert_eq!(missing.len(), Feature::ALL.len() - 2);
    assert!(Features::all().missing().is_empty());
}

#[test]
fn the_memory_spike_declares_nothing_so_no_declaration_goes_unbacked() {
    crate::store::conformance::features::every_declared_feature_works(MemoryStore::new);
}

#[test]
#[should_panic(expected = "does not declare")]
fn the_gateway_refuses_a_store_that_skips_a_feature() {
    use crate::gateway::{AllowAll, FixedClock, Gateway};
    let _ = Gateway::new(MemoryStore::new(), FixedClock(1), AllowAll);
}
