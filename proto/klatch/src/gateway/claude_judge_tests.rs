use super::*;
use crate::domain::{Breaker, Narrowed, ReviewItem, RulesJudge, Verdict};
use std::cell::RefCell;

/// Records every invocation and answers with a canned output. No process is ever started.
struct FakeRunner {
    seen: RefCell<Vec<Invocation>>,
    reply: Result<ProcessOutput, String>,
}
impl FakeRunner {
    fn ok(stdout: &str) -> Self {
        FakeRunner {
            seen: RefCell::new(vec![]),
            reply: Ok(ProcessOutput {
                code: Some(0),
                stdout: stdout.into(),
                stderr: String::new(),
            }),
        }
    }
    fn failing(reply: Result<ProcessOutput, String>) -> Self {
        FakeRunner {
            seen: RefCell::new(vec![]),
            reply,
        }
    }
    fn calls(&self) -> usize {
        self.seen.borrow().len()
    }
}
impl ProcessRunner for &FakeRunner {
    fn run(&self, inv: &Invocation) -> Result<ProcessOutput, String> {
        self.seen.borrow_mut().push(inv.clone());
        self.reply.clone()
    }
}

fn review(label: &str, text: &str) -> ReviewItem {
    ReviewItem {
        id: 7,
        room: "r".into(),
        seq: 7,
        to_label: "public".into(),
        from_label: label.into(),
        reason: "crossing".into(),
        text: text.into(),
        to_room: String::new(),
    }
}

fn result_json(structured: serde_json::Value) -> String {
    serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "result": "",
        "structured_output": structured})
    .to_string()
}

fn allow() -> String {
    result_json(serde_json::json!({"verdict": "allow", "confidence": 0.9}))
}

/// The judge only writes its settings file there when built, so the directory may go once the judge exists.
fn workdir() -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new("claude-judge")
}

fn config(ceiling: &str) -> ClaudeConfig {
    ClaudeConfig {
        model: "sonnet".into(),
        program: "claude".into(),
        budget_usd: 0.25,
        ceiling: ceiling.into(),
    }
}

fn env(name: &str) -> Option<String> {
    match name {
        "PATH" => Some("/usr/bin".into()),
        "KLATCH_IDENTITY_FILE" => Some("/x/bundle".into()),
        "CLAUDECODE" => Some("1".into()),
        _ => None,
    }
}

fn judge<'a>(r: &'a FakeRunner, ceiling: &str) -> ClaudeJudge<&'a FakeRunner> {
    ClaudeJudge::new(config(ceiling), r, &workdir(), &env).unwrap()
}

#[test]
fn text_below_the_ceiling_is_sent_in_a_quarantined_session() {
    let r = FakeRunner::ok(&allow());
    let j = judge(&r, "private");
    assert_eq!(
        j.judge(&review("internal", "merge when green")),
        Verdict::Allow
    );
    assert_eq!(r.calls(), 1);
    let inv = r.seen.borrow()[0].clone();
    assert_quarantine_flags(&inv);
    assert_merge_text_on_stdin_only(&inv);
    assert_child_env_is_clean(&inv);
}

fn assert_quarantine_flags(inv: &Invocation) {
    for flag in [
        "--strict-mcp-config",
        "--no-session-persistence",
        "--disable-slash-commands",
    ] {
        assert!(inv.args.contains(&flag.to_string()), "{flag}");
    }
    let at = |f: &str| inv.args.iter().position(|a| a == f).unwrap();
    assert_eq!(inv.args[at("--tools") + 1], "");
    assert_eq!(inv.args[at("--setting-sources") + 1], "");
}

/// The text goes on stdin, never on the command line.
fn assert_merge_text_on_stdin_only(inv: &Invocation) {
    assert!(inv.stdin.contains("merge when green"));
    assert!(!inv.args.iter().any(|a| a.contains("merge when green")));
}

/// The child sees no KLATCH_ or CLAUDE variable and holds no key from the repo.
fn assert_child_env_is_clean(inv: &Invocation) {
    assert!(
        inv.env
            .iter()
            .all(|(k, _)| !k.starts_with("KLATCH_") && !k.starts_with("CLAUDE"))
    );
}

#[test]
fn text_at_or_above_the_ceiling_or_with_an_unknown_label_is_never_sent() {
    let r = FakeRunner::ok(&allow());
    let j = judge(&r, "private");
    for label in ["private", "secret", "no-such-label"] {
        assert_eq!(j.judge(&review(label, "x")), Verdict::Defer, "{label}");
    }
    assert_eq!(r.calls(), 0);
}

#[test]
fn an_unknown_ceiling_or_bad_model_or_budget_is_refused() {
    let r = FakeRunner::ok(&allow());
    assert!(ClaudeJudge::new(config("nope"), &r, &workdir(), &env).is_err());
    let mut c = config("private");
    c.model = "so net; rm".into();
    assert!(ClaudeJudge::new(c, &r, &workdir(), &env).is_err());
    let mut c = config("private");
    c.budget_usd = 0.0;
    assert!(ClaudeJudge::new(c, &r, &workdir(), &env).is_err());
}

#[test]
fn the_verdicts_map_and_every_failure_defers() {
    let cases = [
        (
            result_json(serde_json::json!({"verdict": "deny", "confidence": 0.9, "reason": "pii"})),
            Verdict::Deny("pii".into()),
        ),
        (
            result_json(
                serde_json::json!({"verdict": "redact", "confidence": 0.9, "redacted_text": "a [redacted]"}),
            ),
            Verdict::Redact("a [redacted]".into()),
        ),
        ("not json".to_string(), Verdict::Defer),
        (
            result_json(serde_json::json!({"verdict": "maybe", "confidence": 0.9})),
            Verdict::Defer,
        ),
        (
            result_json(serde_json::json!({"verdict": "allow", "confidence": 7})),
            Verdict::Defer,
        ),
        (
            result_json(serde_json::json!({"verdict": "redact", "confidence": 0.9})),
            Verdict::Defer,
        ),
    ];
    for (out, want) in cases {
        let r = FakeRunner::ok(&out);
        assert_eq!(judge(&r, "private").judge(&review("public", "x")), want);
    }
    let r = FakeRunner::failing(Err("no such program".into()));
    assert_eq!(
        judge(&r, "private").judge(&review("public", "x")),
        Verdict::Defer
    );
    let r = FakeRunner::failing(Ok(ProcessOutput {
        code: Some(1),
        stdout: allow(),
        stderr: String::new(),
    }));
    assert_eq!(
        judge(&r, "private").judge(&review("public", "x")),
        Verdict::Defer
    );
}

#[test]
fn the_scrub_list_is_never_part_of_the_prompt() {
    let r = FakeRunner::ok(&allow());
    let j = judge(&r, "private");
    j.judge(&review("public", "x"));
    let inv = r.seen.borrow()[0].clone();
    let all = format!("{} {}", inv.args.join(" "), inv.stdin);
    assert!(!all.contains("Scrub list"));
}

#[test]
fn the_gate_defers_above_the_ceiling_without_costing_the_breaker() {
    let r = FakeRunner::failing(Err("down".into()));
    let gated = BelowCeiling::new(
        Breaker::new(judge(&r, "private"), 2),
        crate::domain::LabelChain::default(),
        "private".into(),
    );
    // many items above the ceiling: the breaker must not open on them
    for _ in 0..5 {
        assert_eq!(gated.judge(&review("private", "x")), Verdict::Defer);
    }
    assert_eq!(r.calls(), 0);
    // two failures below the ceiling open it, then nothing more is sent
    for _ in 0..4 {
        assert_eq!(gated.judge(&review("public", "x")), Verdict::Defer);
    }
    assert_eq!(r.calls(), 2);
}

#[test]
fn rules_deny_first_and_the_hosted_judge_never_sees_that_text() {
    let r = FakeRunner::ok(&allow());
    let rules = RulesJudge::new(
        vec![("proj".into(), "orca".into())],
        crate::domain::LabelChain::default(),
    );
    let stack = Narrowed::new(rules, judge(&r, "private"));
    assert_is_deny(stack.judge(&review("public", "about ORCA")));
    assert_eq!(r.calls(), 0);
    assert_eq!(stack.judge(&review("public", "hello")), Verdict::Allow);
    assert_eq!(r.calls(), 1);
}

fn assert_is_deny(v: Verdict) {
    assert!(matches!(v, Verdict::Deny(_)), "{v:?}");
}

#[test]
fn a_session_sends_the_scrub_list_only_when_the_caller_names_it_and_keeps_the_gate_and_budget() {
    let dir = workdir();
    let terms = vec!["orca-build-07".to_string()];
    let item = review("public", "x");
    let plain = Session::new(config("private"), None, &dir, &env).unwrap();
    let with = Session::new(config("private"), Some(&terms), &dir, &env).unwrap();
    assert_scrub_prompts(&plain, &with, &item);
    for s in [&plain, &with] {
        assert_gate_and_budget(s, &item);
    }
    let mut bad = config("private");
    bad.budget_usd = f64::NAN;
    assert!(Session::new(bad, None, &dir, &env).is_err());
}

/// The value after `flag` on the command line of the session's invocation for `item`.
fn flag_value(s: &Session, item: &ReviewItem, flag: &str) -> String {
    let a = s.invocation(item).args;
    a[a.iter().position(|x| x == flag).unwrap() + 1].clone()
}

fn assert_scrub_prompts(plain: &Session, with: &Session, item: &ReviewItem) {
    assert_eq!(flag_value(plain, item, "--system-prompt"), SYSTEM_PROMPT);
    let prompt = flag_value(with, item, "--system-prompt");
    assert!(prompt.starts_with(SYSTEM_PROMPT));
    assert!(prompt.contains("Scrub list (private terms):\n- orca-build-07\n"));
}

fn assert_gate_and_budget(s: &Session, item: &ReviewItem) {
    assert!(s.admits(item));
    assert!(!s.admits(&review("private", "x")));
    assert!(!s.admits(&review("nonsense", "x")));
    assert_eq!(flag_value(s, item, "--max-budget-usd"), "0.25");
}

#[test]
fn a_reply_reports_usage_and_a_failed_run_says_why_without_quoting_the_reply() {
    let stdout = serde_json::json!({
        "is_error": false, "total_cost_usd": 0.5,
        "structured_output": {"verdict": "deny", "confidence": 0.8, "reason": "a key"},
        "usage": {"input_tokens": 1, "cache_read_input_tokens": 2, "output_tokens": 3}
    })
    .to_string();
    let p = parse_reply(&stdout).unwrap();
    assert_eq!(p.pick, Pick::Deny);
    assert_eq!(
        (p.tokens_in, p.tokens_out, p.cost_usd),
        (Some(3), Some(3), Some(0.5))
    );
    let dir = workdir();
    let s = Session::new(config("private"), None, &dir, &env).unwrap();
    let r = FakeRunner::failing(Ok(ProcessOutput {
        code: Some(2),
        stdout: "SECRET-ECHO".into(),
        stderr: String::new(),
    }));
    let e = s.run(&&r, &review("public", "x")).unwrap_err();
    assert!(e.starts_with("claude exited with status 2"), "{e}");
    assert!(!e.contains("SECRET-ECHO"));
}

fn deny_at(confidence: f64) -> String {
    result_json(
        serde_json::json!({"verdict": "deny", "confidence": confidence, "reason": "looks private"}),
    )
}

#[test]
fn a_verdict_below_the_confidence_band_is_deferred_with_a_note() {
    let r = FakeRunner::ok(&deny_at(0.5));
    let j = judge(&r, "private");
    let (verdict, note) = j.judge_noted(&review("public", "x"));
    assert_eq!(verdict, Verdict::Defer);
    let note = note.expect("the deferral is explained");
    assert!(
        note.contains("deny") && note.contains("0.50") && note.contains("0.70"),
        "{note}"
    );
    assert!(
        !note.contains("looks private"),
        "the note carries no model text: {note}"
    );
    assert_eq!(j.judge(&review("public", "x")), Verdict::Defer);
}

#[test]
fn a_verdict_at_or_above_the_band_stands_and_the_band_is_configurable() {
    let at = FakeRunner::ok(&deny_at(0.7));
    assert!(matches!(
        judge(&at, "private").judge_noted(&review("public", "x")),
        (Verdict::Deny(_), None)
    ));
    let mid = FakeRunner::ok(&deny_at(0.9));
    let strict = judge(&mid, "private").with_band(0.95).unwrap();
    assert_eq!(strict.judge(&review("public", "x")), Verdict::Defer);
    let loose = judge(&mid, "private").with_band(0.0).unwrap();
    assert!(matches!(
        loose.judge(&review("public", "x")),
        Verdict::Deny(_)
    ));
    for bad in [-0.1, 1.5, f64::NAN] {
        assert!(judge(&mid, "private").with_band(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_failure_is_a_defer_with_no_note() {
    let r = FakeRunner::failing(Err("down".into()));
    assert_eq!(
        judge(&r, "private").judge_noted(&review("public", "x")),
        (Verdict::Defer, None)
    );
}

#[test]
fn a_low_confidence_deferral_is_not_a_failure_for_the_breaker() {
    let r = FakeRunner::ok(&deny_at(0.2));
    let gated = Breaker::new(judge(&r, "private"), 2);
    for _ in 0..5 {
        let (v, note) = gated.judge_noted(&review("public", "x"));
        assert_eq!(v, Verdict::Defer);
        assert!(note.is_some());
    }
    assert_eq!(r.calls(), 5, "the breaker never opened");
}
