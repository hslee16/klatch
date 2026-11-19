//! The hard corpus (#931) and the loader features it needs: placeholder transforms, the `passphrase` kind, the
//! optional `origin` field, and a second frozen test split.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn line(extra: &[(&str, serde_json::Value)]) -> String {
    use serde_json::json;
    let mut m = serde_json::Map::new();
    for (k, v) in [
        ("id", json!("h001")),
        ("split", json!("test")),
        ("category", json!("secret-credential")),
        ("label", json!("private")),
        ("to", json!("public")),
        ("verdict", json!("deny")),
        ("text", json!("the key, base64: {{fake:github|b64}}")),
        ("rationale", json!("an encoded key is a key")),
        ("spans", json!(["{{fake:github|b64}}"])),
    ] {
        m.insert(k.to_string(), v);
    }
    for (k, v) in extra {
        m.insert((*k).to_string(), v.clone());
    }
    serde_json::Value::Object(m).to_string()
}

#[test]
fn each_transform_does_what_its_name_says() {
    assert_eq!(transform("b64", "hello").unwrap(), "aGVsbG8=");
    assert_eq!(transform("b64", "hi!?").unwrap(), "aGkhPw==");
    assert_eq!(transform("hex", "hi").unwrap(), "6869");
    assert_eq!(transform("pct", "a!_").unwrap(), "%61%21%5F");
    assert_eq!(transform("rev", "abc").unwrap(), "cba");
    assert_eq!(transform("sp4", "abcdefghij").unwrap(), "abcd efgh ij");
    assert_eq!(transform("sp1", "abc").unwrap(), "a b c");
    assert_eq!(
        transform("zw", "abcdefg").unwrap(),
        "abc\u{200b}def\u{200b}g"
    );
    assert_eq!(transform("cut:2:4", "abcdef").unwrap(), "cd");
    assert_eq!(transform("cut:4:", "abcdef").unwrap(), "ef");
    assert_eq!(transform("cut:0:99", "abc").unwrap(), "abc");
    for bad in ["rot13", "cut", "cut:a:b", "cut:4:2", ""] {
        assert!(transform(bad, "abcdef").is_err(), "{bad}");
    }
}

#[test]
fn a_transformed_placeholder_is_the_same_secret_reshaped() {
    let plain = expand("k", "{{fake:github}}").unwrap();
    assert_eq!(
        expand("k", "{{fake:github|rev}}").unwrap(),
        plain.chars().rev().collect::<String>()
    );
    assert_eq!(
        expand("k", "{{fake:github|cut:0:10}}{{fake:github|cut:10:}}").unwrap(),
        plain
    );
    assert_eq!(
        expand("k", "{{fake:github|hex}}").unwrap(),
        transform("hex", &plain).unwrap()
    );
    // transforms chain left to right, and the index still selects the secret
    assert_eq!(
        expand("k", "{{fake:aws:2|rev|sp4}}").unwrap(),
        transform(
            "sp4",
            &transform("rev", &expand("k", "{{fake:aws:2}}").unwrap()).unwrap()
        )
        .unwrap()
    );
    assert!(expand("k", "{{fake:github|nope}}").is_err());
    assert!(expand("k", "{{fake:nokind|b64}}").is_err());
    // a span written with the same placeholder matches the expanded text
    let item = parse_line(&line(&[])).unwrap();
    assert!(item.text.contains(&item.spans[0]));
    assert!(!item.spans[0].contains("ghp_"));
}

#[test]
fn a_passphrase_is_words_and_digits_and_passes_the_stored_text_check() {
    let p = expand("k", "{{fake:passphrase}}").unwrap();
    assert_eq!(p, expand("k", "{{fake:passphrase}}").unwrap());
    assert_ne!(p, expand("k2", "{{fake:passphrase}}").unwrap());
    let parts: Vec<&str> = p.split('-').collect();
    assert_eq!(parts.len(), 3, "{p}");
    assert!(parts[0].chars().all(|c| c.is_ascii_alphabetic()), "{p}");
    assert!(parts[2].chars().any(|c| c.is_ascii_digit()), "{p}");
    assert!(KINDS.contains(&"passphrase"));
}

#[test]
fn origin_is_optional_and_closed() {
    use serde_json::json;
    assert_eq!(parse_line(&line(&[])).unwrap().origin, None);
    assert_eq!(
        parse_line(&line(&[("origin", json!("design"))]))
            .unwrap()
            .origin,
        Some(Origin::Design)
    );
    assert_eq!(
        parse_line(&line(&[("origin", json!("search"))]))
            .unwrap()
            .origin,
        Some(Origin::Search)
    );
    assert!(parse_line(&line(&[("origin", json!("found"))])).is_err());
    assert!(parse_line(&line(&[("origin", json!(""))])).is_err());
}

#[test]
fn the_digest_ignores_a_missing_origin_and_sees_a_present_one() {
    use serde_json::json;
    let none = parse_line(&line(&[])).unwrap();
    let design = parse_line(&line(&[("origin", json!("design"))])).unwrap();
    let search = parse_line(&line(&[("origin", json!("search"))])).unwrap();
    // without an origin the digest is the one from before the field existed (the first corpus's frozen digest
    // pins that on real data); with one, the origin is part of what is frozen
    assert_ne!(
        test_digest(std::slice::from_ref(&none)),
        test_digest(std::slice::from_ref(&design))
    );
    assert_ne!(test_digest(&[design]), test_digest(&[search]));
}

/// The kinds that expand to a secret (everything but the look-alikes `sri`, `b64blob`, `ssh-pub`).
const SECRET_KINDS: [&str; 17] = [
    "aws",
    "aws-secret",
    "github",
    "github-pat",
    "slack",
    "stripe",
    "openai",
    "anthropic",
    "google",
    "jwt",
    "pem",
    "rsa",
    "npm",
    "hex32",
    "bearer",
    "password",
    "passphrase",
];

fn hard() -> Vec<Item> {
    let text = std::fs::read_to_string(default_hard_path()).unwrap();
    parse_corpus(&text).unwrap()
}

/// The committed hard corpus: schema, unique ids (also against the first corpus), every item has an origin, both
/// origins and every category on both splits, a balanced verdict mix, long items, no stored credential, and a
/// frozen test split.
#[test]
fn the_committed_hard_corpus_is_valid() {
    let items = hard();
    assert!(items.len() >= 240, "only {} items", items.len());
    let p = problems(&items);
    assert!(p.is_empty(), "{p:?}");
    let first: BTreeSet<String> = parse_corpus(&std::fs::read_to_string(default_path()).unwrap())
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    let mut by_cat: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut by_origin: BTreeMap<Origin, usize> = BTreeMap::new();
    for i in &items {
        assert!(!first.contains(&i.id), "{} is also a first-corpus id", i.id);
        let o = i.origin.unwrap_or_else(|| panic!("{} has no origin", i.id));
        *by_origin.entry(o).or_default() += 1;
        let e = by_cat.entry(i.category.as_str()).or_default();
        match i.split {
            Split::Dev => e.0 += 1,
            Split::Test => e.1 += 1,
        }
        assert_eq!(
            looks_like_real_credential(&i.raw_text),
            None,
            "{} stores a credential-shaped string; use a placeholder",
            i.id
        );
        assert_eq!(
            looks_like_real_credential(&i.rationale),
            None,
            "{} rationale",
            i.id
        );
        for s in &i.spans {
            assert!(!s.trim().is_empty(), "{}", i.id);
        }
    }
    for c in CATEGORIES {
        let (dev, test) = by_cat.get(c).copied().unwrap_or_default();
        assert!(dev > 0 && test > 0, "{c}: dev {dev}, test {test}");
    }
    assert!(
        by_origin.get(&Origin::Search).copied().unwrap_or(0) >= 20,
        "{by_origin:?}"
    );
    assert!(
        by_origin.get(&Origin::Design).copied().unwrap_or(0) >= 180,
        "{by_origin:?}"
    );
    // a balanced mix, with enough allows that over-blocking is measured
    let n = items.len();
    for v in Truth::ALL {
        let k = items.iter().filter(|i| i.verdict == v).count();
        assert!(k * 5 >= n, "{} is {k} of {n}", v.word());
    }
    let allow = items.iter().filter(|i| i.verdict == Truth::Allow).count();
    assert!(allow * 100 >= n * 35, "allow {allow} of {n}");
    // long items bury one sentence in 10 to 30 KB
    let long = items
        .iter()
        .filter(|i| i.category == "long" && i.text.len() >= 10_000)
        .count();
    assert!(long >= 8, "{long} long items of 10 KB or more");
    assert!(items.iter().all(|i| i.text.len() <= 32_000));
    assert_eq!(
        format!("{:016x}", test_digest(&items)),
        FROZEN_HARD_TEST_DIGEST,
        "the hard test split changed; it is frozen (see judge-eval/README-hard.md)"
    );
}

#[test]
fn the_hard_split_sizes_are_pinned_and_stratified() {
    let items = hard();
    let dev = items.iter().filter(|i| i.split == Split::Dev).count();
    let test = items.len() - dev;
    assert_eq!((dev, test), HARD_SPLIT_SIZES);
    // within each origin and category, positions 1 and 3 of every five are test
    let mut seen: BTreeMap<(Option<Origin>, &str), usize> = BTreeMap::new();
    for i in &items {
        let k = seen.entry((i.origin, i.category.as_str())).or_default();
        let want = if matches!(*k % 5, 1 | 3) {
            Split::Test
        } else {
            Split::Dev
        };
        assert_eq!(i.split, want, "{}", i.id);
        *k += 1;
    }
}

/// The floors, checked mechanically: every secret placeholder is in a deny item (the shape wins, rule 1), and every
/// text holding a scrub term verbatim, in any case, is a deny (rule 2; obfuscated spellings are labelled by hand).
#[test]
fn every_hard_item_that_meets_a_floor_is_a_deny() {
    let terms = crate::judge_eval::judges::parse_scrub(
        &std::fs::read_to_string(default_scrub_path()).unwrap(),
    )
    .unwrap();
    for i in hard() {
        for k in SECRET_KINDS {
            let tag = format!("{{{{fake:{k}");
            let hit = i.raw_text.match_indices(&tag).any(|(at, _)| {
                matches!(
                    i.raw_text[at + tag.len()..].chars().next(),
                    Some('}' | ':' | '|')
                )
            });
            if hit {
                assert_eq!(i.verdict, Truth::Deny, "{} holds a {k} secret", i.id);
            }
        }
        let lower = i.text.to_lowercase();
        if let Some((_, t)) = terms
            .iter()
            .find(|(_, t)| lower.contains(&t.to_lowercase()))
        {
            assert_eq!(i.verdict, Truth::Deny, "{} names the scrub term {t}", i.id);
        }
    }
}
