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
fn jev_reports_that_its_transport_is_not_available_and_sends_nothing() {
    let o = parse(&["--judge", "jev"]).unwrap();
    let e = build_judge(&o, &[]).err().unwrap();
    assert!(e.contains("transport not available"), "{e}");
    assert!(e.contains("#871"), "{e}");
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
