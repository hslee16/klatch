use super::*;

fn parse(args: &[&str]) -> Result<Options, String> {
    Options::parse(args.iter().map(|s| s.to_string()))
}

#[test]
fn the_defaults_are_the_dev_split_all_items_and_a_one_percent_leak_target() {
    let o = parse(&["--judge", "null"]).unwrap();
    assert_eq!(o.judge, JudgeSpec::Null);
    assert_eq!(o.split, Split::Dev);
    assert_eq!(o.limit, None);
    assert_eq!(o.out, None);
    assert_eq!(o.target_leak, 0.01);
    assert_eq!(o.ceiling, "secret");
}

#[test]
fn every_option_is_read() {
    let o = parse(&[
        "--judge",
        "claude:opus",
        "--split",
        "test",
        "--limit",
        "20",
        "--out",
        "r.json",
        "--target-leak",
        "0.05",
        "--ceiling",
        "internal",
        "--budget-usd",
        "0.5",
        "--corpus",
        "c.jsonl",
        "--scrub",
        "s.txt",
        "--claude",
        "/opt/claude",
    ])
    .unwrap();
    assert_eq!(o.judge, JudgeSpec::Claude("opus".into()));
    assert_eq!(o.split, Split::Test);
    assert_eq!(o.limit, Some(20));
    assert_eq!(o.out.as_deref(), Some("r.json"));
    assert_eq!(o.target_leak, 0.05);
    assert_eq!(o.ceiling, "internal");
    assert_eq!(o.budget_usd, 0.5);
    assert_eq!(o.corpus.as_deref(), Some("c.jsonl"));
    assert_eq!(o.scrub.as_deref(), Some("s.txt"));
    assert_eq!(o.claude, "/opt/claude");
}

#[test]
fn the_judge_is_required_and_each_spelling_is_checked() {
    assert!(parse(&[]).unwrap_err().contains("--judge"));
    assert_eq!(
        parse(&["--judge", "rules"]).unwrap().judge,
        JudgeSpec::Rules
    );
    assert_eq!(
        parse(&["--judge", "jev"]).unwrap().judge,
        JudgeSpec::Jev {
            model: None,
            threshold: None
        }
    );
    assert_eq!(
        parse(&["--judge", "jev:jev-1.14.0:0.8"]).unwrap().judge,
        JudgeSpec::Jev {
            model: Some("jev-1.14.0".into()),
            threshold: Some(0.8)
        }
    );
    for bad in [
        "claude:",
        "claude",
        "gpt",
        "jev:x:y:z",
        "jev:jev-1.14.0:two",
    ] {
        assert!(parse(&["--judge", bad]).is_err(), "{bad}");
    }
    assert!(parse(&["--judge", "null", "--split", "train"]).is_err());
    assert!(parse(&["--judge", "null", "--limit", "x"]).is_err());
    assert!(parse(&["--judge", "null", "--limit", "0"]).is_err());
    assert!(parse(&["--judge", "null", "--target-leak", "2"]).is_err());
    assert!(parse(&["--judge", "null", "--bogus"]).is_err());
    assert!(parse(&["--judge"]).is_err());
}

#[test]
fn jev_needs_a_pinned_model_and_a_sane_threshold() {
    let e = build_judge(&parse(&["--judge", "jev"]).unwrap(), &[])
        .err()
        .unwrap();
    assert!(e.contains("pinned model"), "{e}");
    let e = build_judge(&parse(&["--judge", "jev:latest"]).unwrap(), &[])
        .err()
        .unwrap();
    assert!(e.contains("pinned version"), "{e}");
    let e = build_judge(&parse(&["--judge", "jev:jev-1.14.0:2"]).unwrap(), &[])
        .err()
        .unwrap();
    assert!(e.contains("threshold"), "{e}");
}

#[cfg(not(feature = "jev-http"))]
#[test]
fn without_the_feature_jev_says_to_build_with_it_and_sends_nothing() {
    let o = parse(&["--judge", "jev:jev-1.14.0:0.8"]).unwrap();
    let e = build_judge(&o, &[]).err().unwrap();
    assert!(e.contains("build with --features jev-http"), "{e}");
}

#[test]
fn the_null_and_rules_judges_run_end_to_end_on_the_committed_corpus() {
    let null = run(&parse(&["--judge", "null", "--limit", "15"]).unwrap()).unwrap();
    assert_eq!(null.0.n, 15);
    assert_eq!(null.0.defers, 15);
    let rules = run(&parse(&["--judge", "rules"]).unwrap()).unwrap();
    assert_eq!(rules.0.defers, 0);
    assert!(rules.0.n > 100);
    // the rules catch the engine-recognised credential shapes, so they leak fewer items than always-allow would
    let blocked =
        rules.0.confusion[1].iter().sum::<usize>() + rules.0.confusion[2].iter().sum::<usize>();
    assert!(rules.0.leaks < blocked, "{} of {blocked}", rules.0.leaks);
}

#[test]
fn origin_selects_design_or_search_items_and_is_checked() {
    assert_eq!(parse(&["--judge", "null"]).unwrap().origin, None);
    assert_eq!(
        parse(&["--judge", "null", "--origin", "design"])
            .unwrap()
            .origin,
        Some(corpus::Origin::Design)
    );
    assert_eq!(
        parse(&["--judge", "null", "--origin", "search"])
            .unwrap()
            .origin,
        Some(corpus::Origin::Search)
    );
    assert!(
        parse(&["--judge", "null", "--origin", "all"])
            .unwrap_err()
            .contains("--origin")
    );
}

#[test]
fn the_rules_judge_runs_on_the_hard_corpus_by_origin() {
    let path = corpus::default_hard_path();
    let path = path.to_str().unwrap();
    let all = run(&parse(&["--judge", "rules", "--corpus", path]).unwrap()).unwrap();
    let design =
        run(&parse(&["--judge", "rules", "--corpus", path, "--origin", "design"]).unwrap())
            .unwrap();
    let search =
        run(&parse(&["--judge", "rules", "--corpus", path, "--origin", "search"]).unwrap())
            .unwrap();
    assert!(design.0.n > 0 && search.0.n > 0);
    assert_eq!(design.0.n + search.0.n, all.0.n);
    assert!(
        search.1.iter().all(|r| r.id.starts_with('x')),
        "search ids start with x"
    );
    assert!(
        design.1.iter().all(|r| r.id.starts_with('h')),
        "design ids start with h"
    );
}

// the cascade (#936)

#[test]
fn a_cascade_names_two_stages_and_only_judges_that_report_a_pick() {
    let o = parse(&["--judge", "cascade:jev:jev-1.14.0:0.9,claude:sonnet"]).unwrap();
    assert_eq!(
        o.judge,
        JudgeSpec::Cascade {
            first: Box::new(JudgeSpec::Jev {
                model: Some("jev-1.14.0".into()),
                threshold: Some(0.9)
            }),
            second: Box::new(JudgeSpec::Claude("sonnet".into())),
        }
    );
    for bad in [
        "cascade:",
        "cascade:claude:sonnet",
        "cascade:claude:sonnet,",
        "cascade:,claude:sonnet",
        "cascade:rules,claude:sonnet",
        "cascade:null,claude:sonnet",
        "cascade:claude:haiku,rules",
        "cascade:claude:a,claude:b,claude:c",
        "cascade:cascade:claude:a,claude:b,claude:c",
        "cascade:gpt,claude:sonnet",
    ] {
        assert!(parse(&["--judge", bad]).is_err(), "{bad}");
    }
}

#[test]
fn the_policy_defaults_are_conservative_and_each_knob_can_be_set() {
    let o = parse(&["--judge", "cascade:claude:haiku,claude:sonnet"]).unwrap();
    let p = o.cascade_policy().unwrap();
    assert_eq!(p, klatch::domain::CascadePolicy::default());
    assert_eq!(
        p.redact_threshold, None,
        "a redact is escalated unless asked"
    );
    let o = parse(&[
        "--judge",
        "cascade:claude:haiku,claude:sonnet",
        "--deny-threshold",
        "0.8",
        "--allow-threshold",
        "0.99",
        "--redact-threshold",
        "0.95",
    ])
    .unwrap();
    let p = o.cascade_policy().unwrap();
    assert_eq!(
        (p.deny_threshold, p.allow_threshold, p.redact_threshold),
        (Some(0.8), Some(0.99), Some(0.95))
    );
}

#[test]
fn the_first_judges_threshold_in_the_spec_is_the_deny_threshold_unless_a_flag_says_otherwise() {
    let o = parse(&["--judge", "cascade:jev:jev-1.14.0:0.9,claude:sonnet"]).unwrap();
    let p = o.cascade_policy().unwrap();
    assert_eq!(p.deny_threshold, Some(0.9));
    assert!(
        p.allow_threshold >= p.deny_threshold,
        "an allow is never easier than a deny"
    );
    let o = parse(&[
        "--judge",
        "cascade:jev:jev-1.14.0:0.9,claude:sonnet",
        "--deny-threshold",
        "0.7",
    ])
    .unwrap();
    assert_eq!(o.cascade_policy().unwrap().deny_threshold, Some(0.7));
    // an allow default never falls below the deny threshold taken from the spec
    let o = parse(&["--judge", "cascade:jev:jev-1.14.0:0.99,claude:sonnet"]).unwrap();
    assert_eq!(o.cascade_policy().unwrap().allow_threshold, Some(0.99));
}

#[test]
fn bad_thresholds_and_thresholds_on_a_single_judge_are_refused() {
    assert!(
        parse(&[
            "--judge",
            "cascade:claude:a,claude:b",
            "--deny-threshold",
            "1.5"
        ])
        .and_then(|o| o.cascade_policy().map(|_| ()))
        .is_err()
    );
    assert!(
        parse(&[
            "--judge",
            "cascade:claude:a,claude:b",
            "--allow-threshold",
            "x"
        ])
        .is_err()
    );
    let e = parse(&["--judge", "rules", "--deny-threshold", "0.9"]).unwrap_err();
    assert!(e.contains("cascade"), "{e}");
}

#[cfg(unix)]
#[test]
fn a_cascade_runs_end_to_end_with_a_fake_claude_and_records_the_stage_and_the_summed_cost() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("klatch-cascade-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("claude");
    let reply = serde_json::json!({
        "type": "result", "subtype": "success", "is_error": false, "result": "",
        "structured_output": {"verdict": "deny", "confidence": 0.97, "reason": "x"},
        "total_cost_usd": 0.01,
        "usage": {"input_tokens": 10, "output_tokens": 4}
    });
    std::fs::write(
        &script,
        format!("#!/bin/sh\ncat >/dev/null\nprintf '%s' '{reply}'\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = parse(&[
        "--judge",
        "cascade:claude:haiku,claude:sonnet",
        "--limit",
        "5",
        "--claude",
        script.to_str().unwrap(),
    ])
    .unwrap();
    let (summary, records) = run(&o).unwrap();
    assert_eq!(summary.n, 5);
    // the fake denies with 0.97 every time, which the default policy takes at the first stage
    assert!(records.iter().all(|r| r.stage.as_deref() == Some("first")));
    assert_eq!((summary.staged, summary.escalated), (5, 0));
    assert!(
        (summary.cost_usd.unwrap() - 0.05).abs() < 1e-9,
        "{:?}",
        summary.cost_usd
    );
    // with a deny threshold above 0.97 every item is escalated and both stages are paid for
    let mut o2 = o.clone();
    o2.deny_threshold = Some(0.99);
    let (summary, records) = run(&o2).unwrap();
    assert!(records.iter().all(|r| r.stage.as_deref() == Some("second")));
    assert_eq!(summary.escalated, 5);
    assert!(
        (summary.cost_usd.unwrap() - 0.10).abs() < 1e-9,
        "{:?}",
        summary.cost_usd
    );
    let _ = std::fs::remove_dir_all(&dir);
}
