//! Golden text of `klatch init` and `klatch consent` (#1480): the hook commands every combination of the consents
//! writes (the Stop hook is three-way for passive claims, the checkpoint and the machine status, the flags always in
//! the fixed order `--passive-claims --checkpoint --machine-status`), the diff `init` shows, the `consent` listing
//! and the refusals. Fixture settings in a throwaway project and home only; a real settings file is never touched.
//! Regenerate on purpose only: `KLATCH_WRITE_GOLDEN=1`.
mod common;

use common::golden::*;
use common::*;

const FLAGS: [&str; 3] = ["--passive-claims", "--checkpoint", "--machine-status"];
const BASE: [&str; 3] = ["--scope", "project", "--yes"];

fn sheet(s: &Setup) -> Sheet {
    Sheet::new(
        Scrub::new()
            .path(&s.project(), "<PROJECT>")
            .path(&s.home(), "<HOME>")
            .binary(),
    )
}

/// Teach the sheet the paths of a new throwaway setup.
fn learn(sh: &mut Sheet, s: &Setup) {
    sh.scrub_path(&s.project(), "<PROJECT>");
    sh.scrub_path(&s.home(), "<HOME>");
}

/// What a run printed: standard output, then standard error if it printed any.
fn shown(o: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&o.stderr);
    if err.is_empty() {
        out(o)
    } else {
        format!("{}[stderr]\n{err}", out(o))
    }
}

/// The hook commands of the project settings, event by event, one line each (the binary path is scrubbed).
fn hooks(s: &Setup) -> String {
    let path = s.project_settings();
    if !path.exists() {
        return "(no settings file)\n".into();
    }
    let v = Setup::read(&path);
    let mut lines = String::new();
    let mut events: Vec<&String> = v["hooks"]
        .as_object()
        .map_or(vec![], |m| m.keys().collect());
    events.sort();
    for event in events {
        for c in hook_cmds(&v, event) {
            lines.push_str(&format!("{event}: {c}\n"));
        }
    }
    lines
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut v = BASE.to_vec();
    v.extend_from_slice(extra);
    v
}

#[test]
fn every_combination_of_the_three_flag_consents_writes_the_same_hooks_every_time() {
    let mut sheets = None;
    for mask in 0..8u32 {
        let flags: Vec<&str> = (0..3)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| FLAGS[i])
            .collect();
        let s = Setup::new();
        let sh = sheets.get_or_insert_with(|| sheet(&s));
        learn(sh, &s);
        let label = format!("klatch init {}", flags.join(" "));
        let o = s.init(&args(&flags), "");
        sh.block(
            &format!("{label}\n(the diff and the notes)"),
            o.status.success(),
            &shown(&o),
        );
        sh.note(&format!("{label}: the hook commands"), &hooks(&s));
        let o = s.klatch("consent", &["--scope", "project"], "");
        sh.block(
            &format!("{label}: klatch consent"),
            o.status.success(),
            &shown(&o),
        );
        // a plain rerun changes nothing, and the order of the flags on the command line does not matter
        let before = std::fs::read(s.project_settings()).unwrap();
        let mut reversed = flags.clone();
        reversed.reverse();
        s.init(&args(&reversed), "");
        assert_eq!(
            std::fs::read(s.project_settings()).unwrap(),
            before,
            "{label}"
        );
    }
    sheets.unwrap().check("consent_flag_combinations", &[]);
}

#[test]
fn the_whole_settings_file_for_no_consent_for_every_consent_and_for_the_board_enforce_and_wake() {
    let s = Setup::new();
    let mut sh = sheet(&s);
    let cases: [(&str, Vec<&str>); 3] = [
        ("no consent beyond the default", vec![]),
        (
            "every consent on",
            vec![
                "--board",
                "lobby",
                "--enforce",
                "--wake",
                "--passive-claims",
                "--checkpoint",
                "--machine-status",
                "--no-lobby",
                "--no-session-end",
            ],
        ),
        (
            "board, enforce and wake only",
            vec!["--board", "lobby", "--enforce", "--wake"],
        ),
    ];
    for (label, extra) in cases {
        let s = Setup::new();
        learn(&mut sh, &s);
        let o = s.init(&args(&extra), "");
        sh.block(
            &format!("klatch init: {label}"),
            o.status.success(),
            &shown(&o),
        );
        let text = std::fs::read_to_string(s.project_settings()).unwrap_or_default();
        sh.note(&format!("{label}: the settings file"), &text);
        let o = s.klatch("consent", &["--scope", "project"], "");
        sh.block(
            &format!("{label}: klatch consent"),
            o.status.success(),
            &shown(&o),
        );
    }
    sh.check("consent_whole_settings", &[]);
}

#[test]
fn consent_switches_each_consent_off_and_on_and_says_what_it_did() {
    let s = Setup::new();
    let mut sh = sheet(&s);
    let o = s.init(
        &args(&["--passive-claims", "--checkpoint", "--machine-status"]),
        "",
    );
    assert!(o.status.success());
    for name in [
        "board",
        "join",
        "session-end",
        "enforce",
        "wake",
        "checkpoint",
        "passive-claims",
        "machine-status",
    ] {
        for value in ["off", "on"] {
            let o = s.klatch("consent", &[name, value, "--scope", "project", "--yes"], "");
            sh.block(
                &format!("klatch consent {name} {value}"),
                o.status.success(),
                &shown(&o),
            );
            sh.note(
                &format!("after consent {name} {value}: the hook commands"),
                &hooks(&s),
            );
        }
    }
    sh.check("consent_switches", &[]);
}

/// One run of a refusal scenario: the subcommand, what it shows, its arguments and what is typed.
struct Run {
    sub: &'static str,
    label: &'static str,
    args: Vec<&'static str>,
    stdin: &'static str,
}

fn run(sub: &'static str, label: &'static str, args: &[&'static str], stdin: &'static str) -> Run {
    Run {
        sub,
        label,
        args: args.to_vec(),
        stdin,
    }
}

fn refusal_runs() -> Vec<Run> {
    let scope = ["--scope", "project"];
    vec![
        run(
            "consent",
            "a change without a scope",
            &["checkpoint", "off", "--yes"],
            "",
        ),
        run(
            "consent",
            "a bad name",
            &["nonsense", "on", "--scope", "project", "--yes"],
            "",
        ),
        run(
            "consent",
            "a bad value",
            &["checkpoint", "maybe", "--scope", "project", "--yes"],
            "",
        ),
        run(
            "consent",
            "without --yes, answering no",
            &["checkpoint", "off", scope[0], scope[1]],
            "n\n",
        ),
        run(
            "init",
            "codex with passive claims",
            &["--codex", "--passive-claims", "--yes"],
            "",
        ),
        run(
            "init",
            "codex with machine status",
            &["--codex", "--machine-status", "--yes"],
            "",
        ),
        run(
            "init",
            "codex with a checkpoint",
            &["--codex", "--checkpoint", "--yes"],
            "",
        ),
        run("init", "without a scope and with --yes", &["--yes"], ""),
        run(
            "init",
            "uninstall",
            &["--uninstall", "--scope", "project", "--yes"],
            "",
        ),
    ]
}

#[test]
fn consent_and_init_refuse_in_the_same_words() {
    let s = Setup::new();
    let mut sh = sheet(&s);
    let o = s.klatch("consent", &[], "");
    sh.block(
        "klatch consent, nothing installed",
        o.status.success(),
        &shown(&o),
    );
    assert!(s.init(&args(&["--checkpoint"]), "").status.success());
    for r in refusal_runs() {
        let o = s.klatch(r.sub, &r.args, r.stdin);
        sh.block(
            &format!("klatch {}: {}", r.sub, r.label),
            o.status.success(),
            &shown(&o),
        );
        sh.note("the hook commands after it", &hooks(&s));
    }
    sh.check("consent_refusals", &[]);
}
