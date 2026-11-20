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

fn workdir() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let d = std::env::temp_dir().join(format!(
        "klatch-claude-judge-test-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
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
    // the text goes on stdin, never on the command line
    assert!(inv.stdin.contains("merge when green"));
    assert!(!inv.args.iter().any(|a| a.contains("merge when green")));
    // the child sees no KLATCH_ or CLAUDE variable and holds no key from the repo
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
    assert_eq!(j.judge(&review("private", "x")), Verdict::Defer);
    assert_eq!(j.judge(&review("secret", "x")), Verdict::Defer);
    assert_eq!(j.judge(&review("no-such-label", "x")), Verdict::Defer);
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
    assert!(matches!(
        stack.judge(&review("public", "about ORCA")),
        Verdict::Deny(_)
    ));
    assert_eq!(r.calls(), 0);
    assert_eq!(stack.judge(&review("public", "hello")), Verdict::Allow);
    assert_eq!(r.calls(), 1);
}
