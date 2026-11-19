use super::*;
use klatch::domain::credential_kind;
use std::collections::{BTreeMap, BTreeSet};

fn line(fields: &[(&str, serde_json::Value)]) -> String {
    let mut m = serde_json::Map::new();
    for (k, v) in fields {
        m.insert((*k).to_string(), v.clone());
    }
    serde_json::Value::Object(m).to_string()
}

fn good() -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    vec![
        ("id", json!("t001")),
        ("split", json!("dev")),
        ("category", json!("secret-credential")),
        ("label", json!("private")),
        ("to", json!("public")),
        ("verdict", json!("deny")),
        ("text", json!("the deploy key is {{fake:github}} for now")),
        ("rationale", json!("a credential never crosses")),
        ("spans", json!(["{{fake:github}}"])),
    ]
}

fn with(field: &str, value: serde_json::Value) -> String {
    let mut f = good();
    for (k, v) in f.iter_mut() {
        if *k == field {
            *v = value.clone();
        }
    }
    line(&f)
}

fn without(field: &str) -> String {
    let f: Vec<_> = good().into_iter().filter(|(k, _)| *k != field).collect();
    line(&f)
}

#[test]
fn a_full_item_parses_and_its_placeholders_expand_in_text_and_spans_alike() {
    let item = parse_line(&line(&good())).unwrap();
    assert_eq!(item.id, "t001");
    assert_eq!(item.split, Split::Dev);
    assert_eq!(item.verdict, Truth::Deny);
    assert_eq!(item.label, "private");
    assert_eq!(item.to, "public");
    assert!(item.raw_text.contains("{{fake:github}}"));
    assert!(!item.text.contains("{{"), "{}", item.text);
    assert_eq!(item.spans.len(), 1);
    assert!(item.text.contains(&item.spans[0]));
    assert!(item.spans[0].starts_with("ghp_"));
}

#[test]
fn every_field_but_spans_is_required_and_unknown_fields_are_refused() {
    for f in [
        "id",
        "split",
        "category",
        "label",
        "to",
        "verdict",
        "text",
        "rationale",
    ] {
        let e = parse_line(&without(f)).unwrap_err();
        assert!(e.contains(f), "{f}: {e}");
    }
    let mut f = good();
    f.push(("colour", serde_json::json!("red")));
    assert!(parse_line(&line(&f)).unwrap_err().contains("colour"));
    assert!(parse_line("not json").is_err());
    assert!(parse_line("[1,2]").is_err());
}

#[test]
fn the_vocabularies_are_closed() {
    use serde_json::json;
    assert!(parse_line(&with("verdict", json!("block"))).is_err());
    assert!(parse_line(&with("verdict", json!("defer"))).is_err());
    assert!(parse_line(&with("split", json!("train"))).is_err());
    assert!(parse_line(&with("label", json!("secret"))).is_err());
    assert!(parse_line(&with("label", json!("Private"))).is_err());
    assert!(parse_line(&with("to", json!("nowhere"))).is_err());
    assert!(parse_line(&with("category", json!("misc"))).is_err());
    assert!(parse_line(&with("id", json!(""))).is_err());
    assert!(parse_line(&with("rationale", json!(" "))).is_err());
    assert!(parse_line(&with("text", json!(""))).is_err());
}

#[test]
fn a_crossing_must_go_strictly_down() {
    use serde_json::json;
    let e = parse_line(&line(&[
        ("id", json!("t1")),
        ("split", json!("dev")),
        ("category", json!("everyday-chat")),
        ("label", json!("internal")),
        ("to", json!("internal")),
        ("verdict", json!("allow")),
        ("text", json!("hi")),
        ("rationale", json!("chat")),
    ]))
    .unwrap_err();
    assert!(e.contains("below"), "{e}");
    let e = parse_line(&line(&[
        ("id", json!("t1")),
        ("split", json!("dev")),
        ("category", json!("everyday-chat")),
        ("label", json!("internal")),
        ("to", json!("private")),
        ("verdict", json!("allow")),
        ("text", json!("hi")),
        ("rationale", json!("chat")),
    ]))
    .unwrap_err();
    assert!(e.contains("below"), "{e}");
}

#[test]
fn a_blocked_item_names_its_spans_and_each_span_is_in_the_text() {
    use serde_json::json;
    let e = parse_line(&without("spans")).unwrap_err();
    assert!(e.contains("spans"), "{e}");
    let e = parse_line(&with("spans", json!([]))).unwrap_err();
    assert!(e.contains("spans"), "{e}");
    let e = parse_line(&with("spans", json!(["not in the text"]))).unwrap_err();
    assert!(e.contains("span"), "{e}");
    // an allowed item has nothing to hide, so it carries no spans
    let mut f = good();
    for (k, v) in f.iter_mut() {
        match *k {
            "verdict" => *v = json!("allow"),
            "text" => *v = json!("see ticket 12"),
            _ => {}
        }
    }
    let e = parse_line(&line(&f)).unwrap_err();
    assert!(e.contains("spans"), "{e}");
}

#[test]
fn an_unknown_or_unclosed_placeholder_is_refused() {
    use serde_json::json;
    assert!(parse_line(&with("text", json!("key {{fake:nonsense}}"))).is_err());
    assert!(parse_line(&with("text", json!("key {{fake:github"))).is_err());
    assert!(parse_line(&with("text", json!("key {{fake:github:x}}"))).is_err());
}

#[test]
fn expansion_is_deterministic_and_distinct_per_item_and_index() {
    let a = expand("x1", "{{fake:aws}} {{fake:aws:2}}").unwrap();
    assert_eq!(a, expand("x1", "{{fake:aws}} {{fake:aws:2}}").unwrap());
    let parts: Vec<&str> = a.split(' ').collect();
    assert_ne!(parts[0], parts[1]);
    assert_ne!(a, expand("x2", "{{fake:aws}} {{fake:aws:2}}").unwrap());
    assert_eq!(
        expand("x1", "{{fake:aws}}").unwrap(),
        expand("x1", "{{fake:aws:1}}").unwrap()
    );
    assert_eq!(expand("x", "no placeholders").unwrap(), "no placeholders");
}

#[test]
fn every_credential_kind_expands_to_a_shape_the_engine_recognises() {
    for kind in [
        "aws",
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
    ] {
        let s = expand("k", &format!("value {{{{fake:{kind}}}}} end")).unwrap();
        assert!(credential_kind(&s).is_some(), "{kind}: {s}");
        assert!(looks_like_real_credential(&s).is_some(), "{kind}: {s}");
    }
    // the kinds the engine has no shape for are still flagged by the conservative check
    for kind in ["aws-secret", "npm", "hex32", "bearer", "password"] {
        let s = expand("k", &format!("{{{{fake:{kind}}}}}")).unwrap();
        assert!(!s.is_empty(), "{kind}");
        assert!(!s.contains("{{"), "{kind}");
    }
    let n = expand("k", "{{fake:npm}}").unwrap();
    assert!(looks_like_real_credential(&n).is_some(), "{n}");
}

#[test]
fn the_conservative_check_passes_documented_examples_hashes_and_ids() {
    for clean in [
        "AKIAIOSFODNN7EXAMPLE is the AWS documentation key",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "commit 3f9c1d2a8b7e6f5d4c3b2a1908f7e6d5c4b3a291",
        "sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        "id 123e4567-e89b-12d3-a456-426614174000",
        "the key is {{fake:github}}",
        "Content-Security-Policy-Report-Only",
        "",
    ] {
        assert_eq!(looks_like_real_credential(clean), None, "{clean}");
    }
    for dirty in [
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "-----BEGIN RSA PRIVATE KEY-----",
        "xoxb-1234",
        "token AbCdEfGh12345678IjKlMnOp90123456QrSt",
        "AKIAZZZZZZZZZZZZZZZZ",
    ] {
        assert!(looks_like_real_credential(dirty).is_some(), "{dirty}");
    }
}

#[test]
fn select_filters_by_split_and_limits_in_file_order() {
    let mut a = parse_line(&line(&good())).unwrap();
    let mut b = a.clone();
    let mut c = a.clone();
    a.id = "a".into();
    b.id = "b".into();
    b.split = Split::Test;
    c.id = "c".into();
    let items = vec![a, b, c];
    let dev: Vec<&str> = select(&items, Some(Split::Dev), None)
        .iter()
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(dev, ["a", "c"]);
    assert_eq!(select(&items, None, None).len(), 3);
    assert_eq!(select(&items, Some(Split::Dev), Some(1)).len(), 1);
    assert_eq!(select(&items, Some(Split::Test), None)[0].id, "b");
}

#[test]
fn duplicate_ids_are_a_problem() {
    let a = parse_line(&line(&good())).unwrap();
    let p = problems(&[a.clone(), a]);
    assert!(p.iter().any(|m| m.contains("t001")), "{p:?}");
}

#[test]
fn the_digest_changes_with_any_test_item_and_ignores_dev_items() {
    let a = parse_line(&line(&good())).unwrap();
    let mut t = a.clone();
    t.id = "t2".into();
    t.split = Split::Test;
    let base = test_digest(&[a.clone(), t.clone()]);
    let mut dev_changed = a.clone();
    dev_changed.rationale = "changed".into();
    assert_eq!(base, test_digest(&[dev_changed, t.clone()]));
    let mut test_changed = t.clone();
    test_changed.rationale = "changed".into();
    assert_ne!(base, test_digest(&[a, test_changed]));
}

/// The committed corpus: schema, unique ids, vocabulary, coverage, a frozen test split, and no stored text that
/// looks like a real credential (every credential-shaped string is a placeholder expanded at load time, or a
/// documented example).
#[test]
fn the_committed_corpus_is_valid() {
    let text = std::fs::read_to_string(default_path()).unwrap();
    let items = parse_corpus(&text).unwrap();
    assert!(items.len() >= 300, "only {} items", items.len());
    let p = problems(&items);
    assert!(p.is_empty(), "{p:?}");
    let ids: BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids.len(), items.len());
    let mut by_cat: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for i in &items {
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
    }
    for c in CATEGORIES {
        let (dev, test) = by_cat.get(c).copied().unwrap_or_default();
        assert!(dev > 0 && test > 0, "{c}: dev {dev}, test {test}");
    }
    // allow is the majority class, so over-blocking is measured on enough clean text
    let allow = items.iter().filter(|i| i.verdict == Truth::Allow).count();
    assert!(allow * 2 > items.len(), "allow {allow} of {}", items.len());
    // the test split is frozen: changing it is a deliberate act that updates this digest and the README
    assert_eq!(
        format!("{:016x}", test_digest(&items)),
        FROZEN_TEST_DIGEST,
        "the test split changed; it is frozen (see judge-eval/README.md)"
    );
}

#[test]
fn the_committed_scrub_list_parses_and_every_scrub_item_names_a_listed_term_or_is_a_near_miss() {
    let terms = crate::judge_eval::judges::parse_scrub(
        &std::fs::read_to_string(default_scrub_path()).unwrap(),
    )
    .unwrap();
    assert!(terms.len() >= 5);
    let items = parse_corpus(&std::fs::read_to_string(default_path()).unwrap()).unwrap();
    for i in items.iter().filter(|i| i.category == "scrub-term") {
        let hit = terms
            .iter()
            .any(|(_, t)| i.text.to_lowercase().contains(&t.to_lowercase()));
        assert_eq!(
            hit,
            i.verdict == Truth::Deny,
            "{}: a scrub item is denied exactly when it names a listed term",
            i.id
        );
    }
}
