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

fn refusal_targets(s: &SqliteStore) -> Vec<String> {
    s.audit_log(10_000)
        .unwrap()
        .into_iter()
        .filter(|a| a.action == "policy-reserved-refused")
        .map(|a| a.target)
        .collect()
}

#[test]
fn a_huge_key_gives_a_bounded_audit_row() {
    let s = SqliteStore::open_in_memory().unwrap();
    let k = format!("admin-passphrase-{}", "x".repeat(100 * 1024));
    assert!(s.policy_set(&k, Some("v"), 100).is_err());
    let t = refusal_targets(&s);
    assert_eq!(t.len(), 1);
    assert!(t[0].len() <= 256, "target of {} bytes", t[0].len());
    assert!(t[0].starts_with("admin-passphrase-xxx"), "{}", t[0]);
    assert!(t[0].ends_with(crate::store::area::TRUNCATED), "{}", t[0]);
}

#[test]
fn a_huge_key_of_wide_characters_is_bounded_too() {
    let s = SqliteStore::open_in_memory().unwrap();
    let k = format!("admin-unlock-{}", "\u{202e}".repeat(50_000));
    assert!(s.policy_set(&k, Some("v"), 100).is_err());
    for t in refusal_targets(&s) {
        assert!(t.len() <= 256, "target of {} bytes", t.len());
    }
}

#[test]
fn control_and_bidi_characters_never_reach_the_stored_target() {
    let s = SqliteStore::open_in_memory().unwrap();
    let bad = [
        "admin-passphrase-a\u{0}b",
        "admin-passphrase-a\nb",
        "admin-passphrase-a\x1b[31mb",
        "admin-passphrase-a\u{7f}b",
        "admin-passphrase-a\u{202e}b",
        "admin-passphrase-a\u{2066}b",
        "admin-passphrase-a\u{200b}b",
        "admin-passphrase-a\u{feff}b",
        "admin-passphrase-a\u{85}b",
        "admin-passphrase-a\u{2028}b",
    ];
    for (i, k) in bad.iter().enumerate() {
        // distinct minutes, so the throttle does not hide a row
        assert!(s.policy_set(k, Some("v"), 1000 * i as i64).is_err());
    }
    let t = refusal_targets(&s);
    assert_eq!(t.len(), bad.len());
    for t in t {
        assert!(
            t.chars().all(|c| c.is_ascii_graphic()),
            "unsafe target {t:?}"
        );
        assert!(t.starts_with("admin-passphrase-a"), "{t:?}");
    }
}

#[test]
fn sanitising_leaves_the_matching_key_alone() {
    use crate::store::area::normalize_key;
    assert_eq!(normalize_key(" ADMIN-x\u{202e}\n "), "admin-x\u{202e}");
    assert!(is_proof_key("admin-passphrase\u{202e}"));
    assert!(!is_proof_key("\u{202e}admin-passphrase"));
}

#[test]
fn a_plain_key_is_stored_as_is() {
    let s = SqliteStore::open_in_memory().unwrap();
    assert!(s.policy_set(" Admin-Unlock-Secs ", Some("v"), 100).is_err());
    assert_eq!(refusal_targets(&s), ["admin-unlock-secs"]);
}

// Throttle pins (#1755 item 1): the throttle must count the normalized key, not the raw one.

#[test]
fn repeated_real_key_variants_give_one_row_a_window() {
    let s = SqliteStore::open_in_memory().unwrap();
    for i in 0..5 {
        assert!(
            s.policy_set(" ADMIN-PASSPHRASE ", Some("v"), 100 + i)
                .is_err()
        );
    }
    for (i, k) in [
        crate::proof::VERIFIER_KEY.to_uppercase(),
        format!(" {} ", crate::proof::VERIFIER_KEY),
        crate::proof::VERIFIER_KEY.to_string(),
    ]
    .iter()
    .enumerate()
    {
        assert!(s.policy_set(k, Some("v"), 100 + i as i64).is_err());
    }
    let t = refusal_targets(&s);
    assert_eq!(
        t.iter().filter(|x| *x == "admin-passphrase").count(),
        1,
        "{t:?}"
    );
    assert_eq!(
        t.iter()
            .filter(|x| *x == crate::proof::VERIFIER_KEY)
            .count(),
        1,
        "{t:?}"
    );
}

#[test]
fn an_unlock_variant_after_a_passphrase_flood_is_still_recorded() {
    let s = SqliteStore::open_in_memory().unwrap();
    for i in 0..20 {
        assert!(
            s.policy_set(&format!("admin-passphrase-junk{i}"), None, 100)
                .is_err()
        );
    }
    assert!(s.policy_set(" Admin-Unlock-Secs", Some("v"), 101).is_err());
    let t = refusal_targets(&s);
    assert!(t.iter().any(|x| x == "admin-unlock-secs"), "{t:?}");
}

#[test]
fn a_plain_unlock_key_does_not_count_against_the_passphrase_family() {
    let s = SqliteStore::open_in_memory().unwrap();
    for i in 0..20 {
        assert!(
            s.policy_set(&format!("Admin-Unlock-junk{i}"), None, 100)
                .is_err()
        );
    }
    for i in 0..8 {
        assert!(
            s.policy_set(&format!("admin-passphrase-j{i}"), None, 101)
                .is_err()
        );
    }
    let t = refusal_targets(&s);
    let pass = t
        .iter()
        .filter(|x| x.starts_with("admin-passphrase"))
        .count();
    assert_eq!(pass, 8, "the passphrase family keeps its own cap: {t:?}");
}

#[test]
fn the_throttle_holds_inside_a_transaction_for_variants() {
    let s = SqliteStore::open_in_memory().unwrap();
    s.atomically(&mut || {
        for i in 0..5 {
            let _ = s.policy_set(" ADMIN-PASSPHRASE ", Some("v"), 100 + i);
        }
        Ok(())
    })
    .unwrap();
    let t = refusal_targets(&s);
    assert_eq!(
        t.iter().filter(|x| *x == "admin-passphrase").count(),
        1,
        "{t:?}"
    );
}

#[test]
fn the_cut_is_exact_and_never_splits_a_character_or_an_escape() {
    use crate::store::area::{MAX_AUDIT_TARGET, TRUNCATED, audit_target};
    let fits = "a".repeat(MAX_AUDIT_TARGET);
    assert_eq!(audit_target(&fits), fits);
    let over = "a".repeat(MAX_AUDIT_TARGET + 1);
    let t = audit_target(&over);
    assert_eq!(t.len(), MAX_AUDIT_TARGET);
    assert!(t.ends_with(TRUNCATED));
    for pad in 0..8 {
        for ch in ["\u{e9}", "\u{1f600}", "\u{202e}", "\0"] {
            let k = format!("{}{}", "a".repeat(100 + pad), ch.repeat(40));
            let t = audit_target(&k);
            assert!(t.len() <= MAX_AUDIT_TARGET, "{t:?}");
            assert!(t.ends_with(TRUNCATED), "{t:?}");
        }
    }
    assert_eq!(audit_target("a\u{202e}"), r"a\u{202e}");
    assert_eq!(audit_target(r"a\u{202e}"), r"a\u{5c}u{202e}");
}
