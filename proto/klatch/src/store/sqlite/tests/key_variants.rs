//! Case and whitespace variants of a reserved admin key are refused like the key itself (#1588).

use super::*;
use crate::store::PolicyStore;
use crate::store::area::is_proof_key;

const VARIANTS: [&str; 6] = [
    "ADMIN-PASSPHRASE",
    "Admin-Unlock-Secs",
    " admin-passphrase-verifier",
    "admin-unlock-secs\t",
    "\u{a0}Admin-Passphrase-Fails\n",
    "\u{3000}ADMIN-UNLOCK-x ",
];

#[test]
fn variants_are_proof_keys_and_other_keys_are_not() {
    for k in VARIANTS {
        assert!(is_proof_key(k), "{k:?} must be reserved");
    }
    for k in [
        "label:admin-passphrase",
        "admin",
        "admin-unlock",
        "sealed:x",
    ] {
        assert!(!is_proof_key(k), "{k:?} must stay writable");
    }
}

#[test]
fn a_generic_write_of_a_variant_is_refused_and_stores_nothing() {
    let s = SqliteStore::open_in_memory().unwrap();
    for (i, k) in VARIANTS.iter().enumerate() {
        assert!(s.policy_set(k, Some("x"), 100 + i as i64).is_err(), "{k:?}");
        assert_eq!(s.policy_get(k).unwrap(), None, "{k:?}");
    }
}

#[test]
fn a_flood_of_variants_cannot_fill_the_audit() {
    let s = SqliteStore::open_in_memory().unwrap();
    for i in 0..200 {
        let k = format!(" ADMIN-PASSPHRASE-junk{i} ");
        assert!(s.policy_set(&k, Some("x"), 100).is_err());
    }
    let rows = s
        .audit_log(1000)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "policy-reserved-refused")
        .count();
    assert!(rows <= 8, "{rows} rows from one family flood");
}
