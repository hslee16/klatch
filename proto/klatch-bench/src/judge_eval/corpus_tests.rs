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
    assert_eq!(
        (
            item.id.as_str(),
            item.split,
            item.verdict,
            item.label.as_str(),
            item.to.as_str()
        ),
        ("t001", Split::Dev, Truth::Deny, "private", "public")
    );
    assert!(item.raw_text.contains("{{fake:github}}"));
    assert!(!item.text.contains("{{"), "{}", item.text);
    let span_shape = (
        item.spans.len(),
        item.text.contains(&item.spans[0]),
        item.spans[0].starts_with("ghp_"),
    );
    assert_eq!(span_shape, (1, true, true));
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
fn contested_and_name_spans_are_optional_and_default_to_off() {
    let item = parse_line(&line(&good())).unwrap();
    assert!(!item.contested);
    assert!(item.name_spans.is_empty());
}

#[test]
fn contested_is_a_boolean() {
    use serde_json::json;
    let mut f = good();
    f.push(("contested", json!(true)));
    assert!(parse_line(&line(&f)).unwrap().contested);
    let mut f = good();
    f.push(("contested", json!(false)));
    assert!(!parse_line(&line(&f)).unwrap().contested);
    let mut f = good();
    f.push(("contested", json!("yes")));
    assert!(parse_line(&line(&f)).unwrap_err().contains("contested"));
}

fn redact_with_names(spans: serde_json::Value, names: serde_json::Value) -> String {
    use serde_json::json;
    line(&[
        ("id", json!("t9")),
        ("split", json!("dev")),
        ("category", json!("personal-data")),
        ("label", json!("private")),
        ("to", json!("public")),
        ("verdict", json!("redact")),
        ("text", json!("ask Ana Kell, ana@example.com, about it")),
        ("rationale", json!("a customer and her email")),
        ("spans", spans),
        ("name_spans", names),
    ])
}

#[test]
fn name_spans_are_a_subset_of_spans_and_leave_at_least_one_data_span() {
    use serde_json::json;
    let ok = parse_line(&redact_with_names(
        json!(["ana@example.com", "Ana Kell"]),
        json!(["Ana Kell"]),
    ))
    .unwrap();
    assert_eq!(ok.name_spans, ["Ana Kell"]);
    assert_eq!(ok.spans.len(), 2);
    // a name span that is not one of the spans
    let e = parse_line(&redact_with_names(
        json!(["ana@example.com", "Ana Kell"]),
        json!(["Ana"]),
    ))
    .unwrap_err();
    assert!(e.contains("name_spans"), "{e}");
    // every span a name: nothing but a name would be left to leak as data
    let e = parse_line(&redact_with_names(json!(["Ana Kell"]), json!(["Ana Kell"]))).unwrap_err();
    assert!(e.contains("name_spans"), "{e}");
    // not an array of strings
    assert!(parse_line(&redact_with_names(json!(["Ana Kell"]), json!("Ana Kell"))).is_err());
    // only a redact item carries name spans
    let mut f = good();
    f.push(("name_spans", json!(["{{fake:github}}"])));
    assert!(parse_line(&line(&f)).unwrap_err().contains("name_spans"));
}

/// Standard base64 decode, enough to check the PNG fixture without a dependency.
fn b64_decode(s: &str) -> Vec<u8> {
    let table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = vec![];
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.trim_end_matches('=').chars() {
        acc = (acc << 6) | table.find(c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[test]
fn the_committed_contested_and_name_labels_are_as_documented() {
    let items = parse_corpus(&std::fs::read_to_string(default_path()).unwrap()).unwrap();
    let by = |id: &str| items.iter().find(|i| i.id == id).unwrap();
    let contested: Vec<&str> = items
        .iter()
        .filter(|i| i.contested)
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(contested, ["short-007", "short-008"]);
    for id in contested {
        assert!(by(id).rationale.contains("contested"), "{id}");
    }
    // every name span is the name of a redact item that also has data to remove
    let named: Vec<&str> = items
        .iter()
        .filter(|i| !i.name_spans.is_empty())
        .map(|i| i.id.as_str())
        .collect();
    for id in [
        "pii-001", "pii-004", "pii-008", "pii-011", "pii-016", "pii-018", "pii-019", "pii-020",
        "pii-022", "pii-023", "ml-017",
    ] {
        assert!(named.contains(&id), "{id} has no name span");
    }
    assert_eq!(by("pii-016").name_spans, ["Wen Zhao"]);
    assert!(by("pii-002").name_spans.is_empty());
}

#[test]
fn look_014_carries_the_literal_base64_of_a_valid_one_pixel_png() {
    let items = parse_corpus(&std::fs::read_to_string(default_path()).unwrap()).unwrap();
    let i = items.iter().find(|i| i.id == "look-014").unwrap();
    assert_eq!(i.verdict, Truth::Allow);
    assert!(!i.raw_text.contains("{{"), "no random placeholder");
    let blob = i
        .text
        .split_whitespace()
        .find(|w| w.starts_with("iVBOR"))
        .expect("the PNG base64")
        .trim_end_matches(';');
    let bytes = b64_decode(blob);
    let end = bytes.len();
    // signature, IHDR, 1x1 pixels, IEND
    assert_eq!(
        (
            &bytes[..8],
            &bytes[12..16],
            &bytes[16..24],
            &bytes[end - 8..end - 4]
        ),
        (
            &b"\x89PNG\r\n\x1a\n"[..],
            &b"IHDR"[..],
            &[0u8, 0, 0, 1, 0, 0, 0, 1][..],
            &b"IEND"[..]
        )
    );
    // the literal is exempt from the credential scan only because it is exactly this public fixture
    assert_eq!(looks_like_real_credential(&i.raw_text), None);
    assert!(looks_like_real_credential("aB3dE5gH7jK9mN1pQ3sT5vW7yZ9bD1fG3hJ5kL7n").is_some());
}

#[test]
fn the_vocabularies_are_closed() {
    use serde_json::json;
    let refused = [
        ("verdict", "block"),
        ("verdict", "defer"),
        ("split", "train"),
        ("label", "secret"),
        ("label", "Private"),
        ("to", "nowhere"),
        ("category", "misc"),
        ("id", ""),
        ("rationale", " "),
        ("text", ""),
    ];
    for (name, value) in refused {
        assert!(
            parse_line(&with(name, json!(value))).is_err(),
            "{name}: {value:?}"
        );
    }
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
    let ex = |id: &str, raw: &str| expand(id, raw).unwrap();
    let two = "{{fake:aws}} {{fake:aws:2}}";
    let a = ex("x1", two);
    let parts: Vec<&str> = a.split(' ').collect();
    assert_ne!(parts[0], parts[1]);
    assert_eq!(
        (a == ex("x1", two), a == ex("x2", two)),
        (true, false),
        "same item same text, another item another text"
    );
    assert_eq!(ex("x1", "{{fake:aws}}"), ex("x1", "{{fake:aws:1}}"));
    assert_eq!(ex("x", "no placeholders"), "no placeholders");
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
    let ids = |split: Option<Split>, limit: Option<usize>| -> Vec<String> {
        select(&items, split, limit)
            .iter()
            .map(|i| i.id.clone())
            .collect()
    };
    assert_eq!(
        [
            ids(Some(Split::Dev), None),
            ids(None, None),
            ids(Some(Split::Dev), Some(1)),
            ids(Some(Split::Test), None)
        ],
        [vec!["a", "c"], vec!["a", "b", "c"], vec!["a"], vec!["b"]]
    );
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
