use super::*;

fn r(kind: &str, value: &str) -> AskRef {
    AskRef {
        kind: kind.into(),
        value: value.into(),
        ..Default::default()
    }
}

fn invalid(refs: &[AskRef]) -> String {
    match check_refs(refs) {
        Err(RefProblem::Invalid(why)) => why,
        other => panic!("expected an invalid refusal, got {other:?}"),
    }
}

fn credential(refs: &[AskRef]) -> String {
    match check_refs(refs) {
        Err(RefProblem::Credential(why)) => why,
        other => panic!("expected a credential refusal, got {other:?}"),
    }
}

#[test]
fn kinds_are_open_tokens() {
    for ok in [
        "ticket",
        "ci-run",
        "mcp-tool",
        "my.kind_2",
        "a",
        &"k".repeat(32),
    ] {
        assert!(valid_ref_kind(ok), "{ok}");
    }
    for bad in [
        "",
        "Ticket",
        "has space",
        "a:b",
        "a/b",
        &"k".repeat(33),
        "ünï",
        "a\nb",
    ] {
        assert!(!valid_ref_kind(bad), "{bad:?}");
    }
}

#[test]
fn an_unknown_kind_is_accepted_as_opaque_text() {
    assert_eq!(
        check_refs(&[r("figma-frame", "any text at all: 12 / \"x\"")]),
        Ok(())
    );
    assert_eq!(check_refs(&[r("zzz", "a b c")]), Ok(()));
}

#[test]
fn a_bad_kind_is_refused() {
    assert!(invalid(&[r("Bad Kind", "x")]).contains("kind"));
}

#[test]
fn known_kinds_check_their_value() {
    for (kind, value) in [
        ("pr", "#12"),
        ("pr", "owner/repo#12"),
        ("pr", "https://github.com/o/r/pull/12"),
        ("ticket", "#4"),
        ("ticket", "ENG-123"),
        ("ticket", "o/r#4"),
        ("commit", "0adbdbde"),
        ("commit", "o/r@0adbdbde"),
        ("path", "src/lib.rs"),
        ("path", "src/lib.rs:10-20"),
        ("path", "dir/"),
        ("url", "https://example.com/a?b=1"),
        ("url", "http://localhost:8080/x"),
        ("message", "m_01HXYZ-abc.def"),
        ("ask", "7"),
        ("ask", "lobby#7"),
        ("room", "lobby"),
        ("run", "run-12"),
        ("member", "alice"),
        ("tool", "grep"),
        ("command", "cargo test -p klatch"),
        ("branch", "alee/1318-ask-refs"),
    ] {
        assert_eq!(check_refs(&[r(kind, value)]), Ok(()), "{kind} {value}");
    }
    for (kind, value) in [
        ("pr", "twelve"),
        ("pr", "o/r#"),
        ("commit", "zzzz"),
        ("commit", "abc"),
        ("path", "/etc/passwd"),
        ("path", "../secret"),
        ("path", "a/../../b"),
        ("path", "a//b"),
        ("path", "./a"),
        ("url", "ftp://example.com/x"),
        ("url", "javascript:alert(1)"),
        ("url", "example.com"),
        ("message", "has space"),
        ("ask", "x"),
        ("ask", "BAD ROOM#7"),
        ("room", "has space"),
        ("member", "a b"),
    ] {
        let why = invalid(&[r(kind, value)]);
        assert!(why.contains(kind), "{kind} {value}: {why}");
    }
}

#[test]
fn relative_paths_and_secret_params_keep_their_exact_rules() {
    for ok in [
        "a.rs",
        "src/a.rs:10",
        "src/a.rs:10-20",
        "dir/",
        "a:b",
        "a.rs:-5",
        "a.rs:5-",
    ] {
        assert!(relative_path(ok), "{ok}");
    }
    for bad in [
        "", "/a", "a\\b", "a//b", "./a", "a/../b", "/", ":10", "a/./b",
    ] {
        assert!(!relative_path(bad), "{bad}");
    }
    for (q, why) in [
        ("a=1&token=xyz", Some("a query value named like a secret")),
        ("my_key=v", Some("a query value named like a secret")),
        ("sig=1;x=2", Some("a query value named like a secret")),
        ("page=2&q=hello", None),
        ("token=", None),
        ("novalue", None),
    ] {
        assert_eq!(secret_param(q), why, "{q}");
    }
    assert_eq!(
        credential_in_url("h.example/p?a=b#access_token=zz"),
        Some("a query value named like a secret")
    );
    assert_eq!(
        credential_in_url("u:p@h.example/p"),
        Some("a user name or password in the URL")
    );
    assert_eq!(credential_in_url("h.example/p?a=b#frag"), None);
}

#[test]
fn empty_and_control_values_and_labels_are_refused() {
    assert!(invalid(&[r("other", "")]).contains("value"));
    assert!(invalid(&[r("other", "  ")]).contains("value"));
    assert!(invalid(&[r("other", "a\nb")]).contains("control"));
    assert!(invalid(&[r("other", "a\x1b[31m")]).contains("control"));
    let mut with_label = r("other", "x");
    with_label.label = Some("bad\nlabel".into());
    assert!(invalid(&[with_label.clone()]).contains("control"));
    with_label.label = Some("".into());
    assert!(invalid(&[with_label]).contains("label"));
}

#[test]
fn a_snippet_may_hold_any_text_including_escape_sequences_and_is_escaped_on_display() {
    let mut a = r("log", "ci.log");
    a.snippet = Some("line one\nline two\x1b[31mred\x1b[0m\ttab".into());
    assert_eq!(check_refs(&[a.clone()]), Ok(()));
    let shown = ref_lines(&[a], true).join("\n");
    assert!(!shown.contains('\x1b'), "{shown:?}");
    assert!(shown.contains("\\u{1b}[31mred"), "{shown:?}");
    assert!(shown.contains("| line one"), "{shown:?}");
    assert!(shown.contains("| line two"), "{shown:?}");
}

#[test]
fn caps_are_refused_never_truncated() {
    let many: Vec<AskRef> = (0..=MAX_REFS)
        .map(|i| r("other", &format!("v{i}")))
        .collect();
    assert!(invalid(&many).contains("32"));
    assert_eq!(check_refs(&many[..MAX_REFS]), Ok(()));
    assert!(invalid(&[r("other", &"v".repeat(MAX_VALUE_BYTES + 1))]).contains("value"));
    assert_eq!(
        check_refs(&[r("other", &"v".repeat(MAX_VALUE_BYTES))]),
        Ok(())
    );
    let mut long_label = r("other", "x");
    long_label.label = Some("l".repeat(MAX_LABEL_BYTES + 1));
    assert!(invalid(&[long_label]).contains("label"));
    let mut long_snippet = r("log", "x");
    long_snippet.snippet = Some("s".repeat(MAX_SNIPPET_BYTES + 1));
    assert!(invalid(&[long_snippet]).contains("snippet"));
    let mut full = r("log", "x");
    full.snippet = Some("s".repeat(MAX_SNIPPET_BYTES));
    assert_eq!(check_refs(&[full.clone()]), Ok(()));
    // each ref is within its caps but together they pass the total
    let total: Vec<AskRef> = (0..MAX_REFS).map(|_| full.clone()).collect();
    assert!(invalid(&total).contains("total"));
}

#[test]
fn credentials_in_a_url_are_refused_without_echoing_them() {
    for url in [
        "https://user:hunter2@example.com/x",
        "https://token@example.com/x",
        "https://example.com/x?access_token=abcdef123456",
        "https://example.com/x?api_key=short",
        "https://example.com/x?x=1&password=pw",
        "https://example.com/x?sig=AbCdEf0123456789AbCdEf0123456789",
        "https://example.com/x?v=0123456789abcdef0123456789abcdef",
        "https://example.com/x#access_token=zzz",
    ] {
        let why = credential(&[r("url", url)]);
        assert!(
            !why.contains("hunter2") && !why.contains("abcdef123456"),
            "{why}"
        );
    }
    // the same shapes under a kind klatch does not know
    credential(&[r("whatever", "see https://u:p@h.example/x")]);
    credential(&[r("ticket", "https://u:p@h.example/x")]);
}

#[test]
fn plain_urls_with_ordinary_queries_pass() {
    for url in [
        "https://github.com/o/r/pull/12",
        "https://example.com/search?q=rust&page=2",
        "https://example.com/x?tab=files#diff-1",
        "https://example.com/mail@home",
    ] {
        assert_eq!(check_refs(&[r("url", url)]), Ok(()), "{url}");
    }
}

#[test]
fn display_is_one_line_inert_text() {
    let mut a = r("pr", "o/r#12");
    a.label = Some("the fix".into());
    assert_eq!(a.line(), "pr o/r#12 (the fix)");
    // stored data from an older or hostile writer is still shown inert
    let hostile = AskRef {
        kind: "x".into(),
        value: "a\nb\x1b[2Jc\u{202e}d".into(),
        label: Some("l\r\nm".into()),
        snippet: None,
    };
    let line = hostile.line();
    assert!(
        !line.contains('\n') && !line.contains('\r') && !line.contains('\x1b'),
        "{line:?}"
    );
    assert!(!line.contains('\u{202e}'), "{line:?}");
    assert!(line.contains("\\n") && line.contains("\\u{1b}") && line.contains("\\u{202e}"));
}

#[test]
fn the_row_mark_lists_a_few_refs_and_counts_the_rest() {
    assert_eq!(refs_mark(&[]), "");
    assert_eq!(refs_mark(&[r("pr", "#1")]), " [refs pr #1]");
    let five: Vec<AskRef> = (1..=5).map(|i| r("ticket", &format!("#{i}"))).collect();
    assert_eq!(
        refs_mark(&five),
        " [refs ticket #1, ticket #2, ticket #3 +2]"
    );
}

#[test]
fn ref_lines_show_snippets_only_when_asked() {
    let mut a = r("log", "ci.log");
    a.snippet = Some("boom\nbang".into());
    let without = ref_lines(&[a.clone()], false);
    assert_eq!(
        without,
        vec!["  ref log ci.log [snippet 9 bytes]".to_string()]
    );
    let with = ref_lines(&[a], true);
    assert_eq!(
        with,
        vec![
            "  ref log ci.log".to_string(),
            "    | boom".to_string(),
            "    | bang".to_string()
        ]
    );
}

#[test]
fn searchable_text_covers_every_field() {
    let a = AskRef {
        kind: "log".into(),
        value: "ci.log".into(),
        label: Some("nightly".into()),
        snippet: Some("segfault in parser".into()),
    };
    let s = a.searchable();
    for want in ["log", "ci.log", "nightly", "segfault in parser"] {
        assert!(s.contains(want), "{s}");
    }
}
