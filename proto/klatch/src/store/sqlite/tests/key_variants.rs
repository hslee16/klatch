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

// Review of #1758.

#[test]
fn every_invisible_or_non_ascii_character_is_escaped() {
    use crate::store::area::audit_target;
    let samples = [
        '\u{600}',
        '\u{6dd}',
        '\u{70f}',
        '\u{13430}',
        '\u{1bca0}',
        '\u{1d173}',
        '\u{e0001}',
        '\u{e0041}',
        '\u{e007f}',
        '\u{fe0f}',
        '\u{e0100}',
        '\u{34f}',
        '\u{301}',
        '\u{3164}',
        '\u{115f}',
        '\u{2800}',
        '\u{a0}',
        '\u{2003}',
        '\u{3000}',
        '\u{e000}',
        '\u{f8ff}',
        '\u{378}',
        '\u{fffe}',
        '\u{10ffff}',
        '\u{212a}',
        '\u{ff21}',
        '\u{e9}',
        '\u{1f600}',
        '\u{7f}',
        '\u{0}',
        '\u{85}',
    ];
    for c in samples {
        let t = audit_target(&format!("admin-passphrase{c}"));
        assert!(
            t.chars().all(|c| c.is_ascii_graphic() || c == ' '),
            "{c:?} -> {t:?}"
        );
        assert_ne!(
            t, "admin-passphrase",
            "{c:?} must not display as the real key"
        );
    }
    let s = SqliteStore::open_in_memory().unwrap();
    assert!(
        s.policy_set("admin-passphrase\u{e0001}", Some("v"), 100)
            .is_err()
    );
    let t = refusal_targets(&s);
    assert_eq!(t, [r"admin-passphrase\u{e0001}"]);
}

#[test]
fn a_key_ending_with_the_marker_is_not_the_cut_form_of_a_longer_key() {
    use crate::store::area::{MAX_AUDIT_TARGET, TRUNCATED, audit_target};
    // while the escaped literal fits, it is stored uncut and must not look cut
    for n in 0..=(MAX_AUDIT_TARGET - r"\u{5c}...[truncated]".len()) {
        // the marker is pure ASCII and starts with a bare backslash
        assert!(TRUNCATED.is_ascii() && TRUNCATED.starts_with('\\'));
        let literal = format!("{}{TRUNCATED}", "a".repeat(n));
        let t = audit_target(&literal);
        let cut = audit_target(&format!("{}{}", "a".repeat(MAX_AUDIT_TARGET * 2), "b"));
        assert!(!t.ends_with(TRUNCATED), "{t:?}");
        assert_ne!(t, cut);
    }
}

#[test]
fn the_cap_is_128_bytes_exactly() {
    use crate::store::area::{MAX_AUDIT_TARGET, TRUNCATED, audit_target};
    assert_eq!(MAX_AUDIT_TARGET, 128);
    assert_eq!(audit_target(&"a".repeat(128)).len(), 128);
    assert!(!audit_target(&"a".repeat(128)).ends_with(TRUNCATED));
    let t = audit_target(&"a".repeat(129));
    assert_eq!(t.len(), 128);
    assert_eq!(
        t,
        format!("{}{TRUNCATED}", "a".repeat(128 - TRUNCATED.len()))
    );
}

#[test]
fn a_cut_never_ends_inside_an_escape_or_a_character() {
    use crate::store::area::{MAX_AUDIT_TARGET, TRUNCATED, audit_target};
    // a plain reference: escape the whole key, then keep the longest prefix of whole pieces that fits with the marker
    fn reference(k: &str) -> String {
        let pieces: Vec<String> = k
            .chars()
            .map(|c| {
                if c.is_ascii_graphic() && c != '\\' || c == ' ' {
                    c.to_string()
                } else {
                    format!("\\u{{{:x}}}", c as u32)
                }
            })
            .collect();
        let whole: String = pieces.concat();
        if whole.len() <= MAX_AUDIT_TARGET {
            return whole;
        }
        let mut out = String::new();
        for p in &pieces {
            if out.len() + p.len() + TRUNCATED.len() > MAX_AUDIT_TARGET {
                break;
            }
            out.push_str(p);
        }
        out + TRUNCATED
    }
    for pad in 90..140 {
        for ch in ["\u{202e}", "\u{1f600}", "\u{e9}", "\0", "\\", "a"] {
            for tail in [1, 2, 3, 30] {
                let k = format!("{}{}", "a".repeat(pad), ch.repeat(tail));
                let t = audit_target(&k);
                assert_eq!(t, reference(&k), "pad {pad} {ch:?} x{tail}");
                assert!(t.is_ascii());
                let body = t.strip_suffix(TRUNCATED).unwrap_or(&t);
                // every `\u{` in the body is closed, and every backslash starts a whole escape
                let mut rest = body;
                while let Some(i) = rest.find('\\') {
                    let tail = &rest[i + 1..];
                    if let Some(esc) = tail.strip_prefix("u{") {
                        let close = esc
                            .find('}')
                            .unwrap_or_else(|| panic!("partial escape in {t:?}"));
                        assert!(esc[..close].chars().all(|c| c.is_ascii_hexdigit()), "{t:?}");
                        rest = &esc[close + 1..];
                    } else {
                        panic!("bare backslash in {t:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn a_cut_target_is_pure_ascii() {
    use crate::store::area::audit_target;
    for k in [
        "x".repeat(300),
        format!("a{}", "\u{202e}".repeat(100)),
        format!("a{}", "\u{e0041}".repeat(100)),
    ] {
        let t = audit_target(&k);
        assert!(t.chars().all(|c| c.is_ascii_graphic() || c == ' '), "{t:?}");
    }
}

// #1763: the text a refused caller reads is capped and escaped like the audit target.

fn refusal_text(k: &str) -> String {
    let s = SqliteStore::open_in_memory().unwrap();
    match s.policy_set(k, Some("v"), 100) {
        Err(e) => format!("{e}"),
        Ok(()) => panic!("{k:?} was not refused"),
    }
}

#[test]
fn the_refusal_text_of_a_huge_key_is_bounded_and_ascii() {
    let t = refusal_text(&format!("admin-passphrase-{}", "x".repeat(100 * 1024)));
    assert!(t.len() < 512, "{} bytes", t.len());
    assert!(t.is_ascii());
    assert!(t.contains(crate::store::area::TRUNCATED), "{t}");
}

#[test]
fn the_refusal_text_escapes_control_bidi_and_tag_characters() {
    for c in [
        '\u{1b}',
        '\n',
        '\u{202e}',
        '\u{2066}',
        '\u{e0041}',
        '\u{e0001}',
        '\u{200b}',
    ] {
        let t = refusal_text(&format!("admin-passphrase{c}x"));
        assert!(
            t.chars().all(|c| c.is_ascii_graphic() || c == ' '),
            "{c:?} {t:?}"
        );
        assert!(t.contains(&format!("\\u{{{:x}}}", c as u32)), "{c:?} {t:?}");
    }
}

#[test]
fn the_refusal_text_of_a_plain_key_is_unchanged() {
    assert_eq!(
        refusal_text("admin-passphrase-hash"),
        "refused (admin-passphrase-rows): the policy key admin-passphrase-hash belongs to the admin passphrase and is changed only by `klatch admin` passphrase commands"
    );
    // the key is shown as typed (case kept), not the normalized form
    assert!(refusal_text("Admin-Unlock-Secs").contains("key Admin-Unlock-Secs belongs"));
}

#[test]
fn multibyte_keys_of_every_length_stay_whole() {
    use crate::store::area::{MAX_AUDIT_TARGET, audit_target};
    for n in 0..200 {
        for ch in ['\u{e9}', '\u{20ac}', '\u{1f600}'] {
            let t = audit_target(&format!("a{}", ch.to_string().repeat(n)));
            assert!(t.len() <= MAX_AUDIT_TARGET, "{n} {ch:?}: {}", t.len());
            assert!(t.starts_with('a'));
        }
    }
}

#[test]
fn the_same_escaped_key_refused_again_gives_one_row() {
    for inside in [false, true] {
        for k in [
            "admin-passphrase-\u{202e}x\n".to_string(),
            "ADMIN-PASSPHRASE-\u{e0001}".to_string(),
            format!(" Admin-Unlock-{} ", "y".repeat(100 * 1024)),
        ] {
            let s = SqliteStore::open_in_memory().unwrap();
            let go = || {
                for i in 0..5 {
                    assert!(s.policy_set(&k, Some("v"), 100 + i).is_err());
                }
            };
            if inside {
                s.atomically(&mut || {
                    go();
                    Ok(())
                })
                .unwrap();
            } else {
                go();
            }
            let t = refusal_targets(&s);
            assert_eq!(t.len(), 1, "inside={inside} {:?}", &k[..20]);
        }
    }
}
