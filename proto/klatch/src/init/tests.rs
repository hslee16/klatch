use super::*;
use serde_json::json;

const BIN: &str = "/opt/klatch/klatch";

fn commands(v: &Value, event: &str) -> Vec<String> {
    v["hooks"][event]
        .as_array()
        .map(|a| {
            a.iter()
                .flat_map(|m| m["hooks"].as_array().cloned().unwrap_or_default())
                .filter_map(|h| h["command"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_command_uses_the_absolute_path_discover_and_the_event() {
    let c = hook_command(BIN, "PostToolUse", None, true).unwrap();
    assert_eq!(c, "/opt/klatch/klatch --discover hook --event PostToolUse");
}

#[test]
fn the_board_flag_is_added_only_for_session_start() {
    let s = hook_command(BIN, "SessionStart", Some("lobby"), true).unwrap();
    assert!(s.ends_with("--event SessionStart --board lobby"), "{s}");
    let p = hook_command(BIN, "PostToolUse", Some("lobby"), true).unwrap();
    assert!(!p.contains("--board"), "{p}");
}

#[test]
fn a_path_with_spaces_is_quoted_and_a_dangerous_path_is_refused() {
    let c = hook_command("/Users/a b/klatch", "PostToolUse", None, true).unwrap();
    assert!(c.starts_with("'/Users/a b/klatch' --discover"), "{c}");
    for bad in [
        "$HOME/klatch",
        "/a/`x`/klatch",
        "/a/it's/klatch",
        "/a/\"q\"/klatch",
        "/a\n/klatch",
        "klatch",
    ] {
        assert!(
            hook_command(bad, "PostToolUse", None, true).is_err(),
            "{bad:?} was accepted"
        );
    }
}

#[test]
fn a_bad_board_room_name_is_refused() {
    assert!(hook_command(BIN, "SessionStart", Some("a b"), true).is_err());
    assert!(hook_command(BIN, "SessionStart", Some(""), true).is_err());
    assert!(hook_command(BIN, "SessionStart", Some("x;rm"), true).is_err());
}

#[test]
fn merging_into_nothing_adds_the_three_events() {
    let v = merge_hooks(None, BIN, None, true).unwrap();
    for e in EVENTS {
        assert_eq!(commands(&v, e).len(), 1, "{e}");
    }
    assert!(!commands(&v, "SessionStart")[0].contains("--board"));
}

#[test]
fn other_settings_and_other_hooks_are_left_alone() {
    let before = json!({
        "permissions": {"allow": ["Bash(ls)"]},
        "hooks": {"PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "/usr/bin/audit"}]}]}
    });
    let v = merge_hooks(Some(&before), BIN, None, true).unwrap();
    assert_eq!(v["permissions"], before["permissions"]);
    let cmds = commands(&v, "PostToolUse");
    assert!(cmds.contains(&"/usr/bin/audit".to_string()));
    assert!(cmds.iter().any(|c| c.contains("klatch")));
    assert_eq!(
        v["hooks"]["PostToolUse"][0],
        before["hooks"]["PostToolUse"][0]
    );
}

#[test]
fn merging_twice_changes_nothing_the_second_time() {
    let once = merge_hooks(None, BIN, Some("lobby"), true).unwrap();
    let twice = merge_hooks(Some(&once), BIN, Some("lobby"), true).unwrap();
    assert_eq!(once, twice);
    assert_eq!(commands(&twice, "PostToolUse").len(), 1);
}

#[test]
fn rerunning_with_other_answers_updates_the_entry_in_place() {
    let with = merge_hooks(None, BIN, Some("lobby"), true).unwrap();
    let without = merge_hooks(Some(&with), BIN, None, true).unwrap();
    assert_eq!(
        commands(&without, "SessionStart").len(),
        1,
        "added a duplicate"
    );
    assert!(!commands(&without, "SessionStart")[0].contains("--board"));
    let moved = merge_hooks(Some(&with), "/other/klatch", Some("lobby"), true).unwrap();
    assert_eq!(commands(&moved, "PostToolUse").len(), 1);
    assert!(commands(&moved, "PostToolUse")[0].starts_with("/other/klatch"));
}

#[test]
fn a_settings_file_of_the_wrong_shape_is_refused() {
    assert!(merge_hooks(Some(&json!([1, 2])), BIN, None, true).is_err());
    assert!(merge_hooks(Some(&json!({"hooks": "no"})), BIN, None, true).is_err());
    assert!(merge_hooks(Some(&json!({"hooks": {"PostToolUse": 3}})), BIN, None, true).is_err());
}

#[test]
fn the_diff_marks_added_removed_and_unchanged_lines() {
    let d = diff("a\nb\nc\n", "a\nx\nc\n");
    assert!(
        d.contains("  a") && d.contains("- b") && d.contains("+ x") && d.contains("  c"),
        "{d}"
    );
    assert_eq!(diff("same\n", "same\n").lines().count(), 1);
}

#[test]
fn a_session_start_command_opts_out_of_joining_only_when_joining_is_off() {
    let on = hook_command(BIN, "SessionStart", None, true).unwrap();
    assert!(!on.contains("--no-join"), "{on}");
    let off = hook_command(BIN, "SessionStart", None, false).unwrap();
    assert!(off.ends_with("--no-join"), "{off}");
    let other = hook_command(BIN, "PostToolUse", None, false).unwrap();
    assert!(!other.contains("--no-join"), "{other}");
}

#[test]
fn turning_joining_off_and_on_again_updates_the_entry_in_place() {
    let off = merge_hooks(None, BIN, None, false).unwrap();
    assert!(commands(&off, "SessionStart")[0].contains("--no-join"));
    let on = merge_hooks(Some(&off), BIN, None, true).unwrap();
    assert_eq!(commands(&on, "SessionStart").len(), 1);
    assert!(!commands(&on, "SessionStart")[0].contains("--no-join"));
}

#[test]
fn the_session_end_hook_is_added_and_is_idempotent() {
    let mut v = merge_hooks(None, BIN, None, true).unwrap();
    set_session_end(&mut v, BIN, true).unwrap();
    assert_eq!(
        commands(&v, "SessionEnd"),
        vec!["/opt/klatch/klatch --discover hook --event SessionEnd".to_string()]
    );
    let once = v.clone();
    set_session_end(&mut v, BIN, true).unwrap();
    assert_eq!(v, once);
    assert_eq!(
        commands(&v, "PostToolUse").len(),
        1,
        "the other events are untouched"
    );
}

#[test]
fn turning_the_session_end_hook_off_removes_only_klatchs_entry() {
    let mut v = json!({"hooks": {"SessionEnd": [{"hooks": [{"type": "command", "command": "/usr/bin/other"}]}]}});
    set_session_end(&mut v, BIN, true).unwrap();
    assert_eq!(commands(&v, "SessionEnd").len(), 2);
    set_session_end(&mut v, BIN, false).unwrap();
    assert_eq!(
        commands(&v, "SessionEnd"),
        vec!["/usr/bin/other".to_string()]
    );
    let mut only = merge_hooks(None, BIN, None, true).unwrap();
    set_session_end(&mut only, BIN, true).unwrap();
    set_session_end(&mut only, BIN, false).unwrap();
    assert!(
        only["hooks"].get("SessionEnd").is_none(),
        "an empty list was left behind"
    );
    set_session_end(&mut only, BIN, false).unwrap();
}

#[test]
fn a_session_end_in_a_settings_file_of_the_wrong_shape_is_refused() {
    assert!(set_session_end(&mut json!([1]), BIN, true).is_err());
    assert!(set_session_end(&mut json!({"hooks": {"SessionEnd": 3}}), BIN, true).is_err());
}
#[test]
fn the_claim_hook_is_opt_in_and_is_not_part_of_the_default_merge() {
    let v = merge_hooks(None, BIN, None, true).unwrap();
    assert!(v["hooks"].get("PreToolUse").is_none());
}

#[test]
fn turning_the_claim_hook_on_adds_one_matcher_entry_and_is_idempotent() {
    let mut v = merge_hooks(None, BIN, None, true).unwrap();
    set_enforce(&mut v, BIN, Switch::On).unwrap();
    let entry = &v["hooks"]["PreToolUse"][0];
    assert_eq!(entry["matcher"], "Edit|Write|MultiEdit|NotebookEdit|Bash");
    assert_eq!(
        commands(&v, "PreToolUse"),
        vec!["/opt/klatch/klatch --discover hook --event PreToolUse".to_string()]
    );
    let once = v.clone();
    set_enforce(&mut v, BIN, Switch::On).unwrap();
    assert_eq!(v, once);
}

#[test]
fn turning_the_claim_hook_off_removes_only_klatchs_entry() {
    let mut v = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "/usr/bin/guard"}]}]}});
    set_enforce(&mut v, BIN, Switch::On).unwrap();
    assert_eq!(commands(&v, "PreToolUse").len(), 2);
    set_enforce(&mut v, BIN, Switch::Off).unwrap();
    assert_eq!(
        commands(&v, "PreToolUse"),
        vec!["/usr/bin/guard".to_string()]
    );
    let mut only = json!({});
    set_enforce(&mut only, BIN, Switch::On).unwrap();
    set_enforce(&mut only, BIN, Switch::Off).unwrap();
    assert!(only["hooks"].get("PreToolUse").is_none());
}

#[test]
fn keeping_the_claim_hook_updates_the_path_but_never_adds_it() {
    let mut absent = merge_hooks(None, BIN, None, true).unwrap();
    let before = absent.clone();
    set_enforce(&mut absent, BIN, Switch::Keep).unwrap();
    assert_eq!(absent, before);
    let mut on = merge_hooks(None, BIN, None, true).unwrap();
    set_enforce(&mut on, BIN, Switch::On).unwrap();
    set_enforce(&mut on, "/other/klatch", Switch::Keep).unwrap();
    assert!(commands(&on, "PreToolUse")[0].starts_with("/other/klatch"));
    assert_eq!(commands(&on, "PreToolUse").len(), 1);
}

#[test]
fn every_installed_hook_entry_has_a_timeout_longer_than_the_keychain_read_limit() {
    let mut v = merge_hooks(None, BIN, None, true).unwrap();
    set_session_end(&mut v, BIN, true).unwrap();
    set_enforce(&mut v, BIN, Switch::On).unwrap();
    let mut seen = 0;
    for (event, list) in v["hooks"].as_object().unwrap() {
        for hook in list
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["hooks"].as_array().unwrap())
        {
            let t = hook["timeout"]
                .as_u64()
                .unwrap_or_else(|| panic!("{event} has no timeout"));
            assert!(t > klatch::keys::KEYCHAIN_READ_LIMIT_SECS, "{event}: {t}");
            seen += 1;
        }
    }
    assert!(seen >= 5, "{seen}");
}

#[test]
fn rerunning_init_adds_a_timeout_to_an_old_entry_and_keeps_a_longer_one() {
    let old = json!({"hooks": {
        "SessionStart": [{"hooks": [{"type": "command", "command": "/opt/klatch/klatch --discover hook --event SessionStart"}]}],
        "Stop": [{"hooks": [{"type": "command", "command": "/opt/klatch/klatch --discover hook --event Stop", "timeout": 120}]}],
        "PostToolUse": [{"hooks": [{"type": "command", "command": "/opt/klatch/klatch --discover hook --event PostToolUse", "timeout": 2}]}]
    }});
    let v = merge_hooks(Some(&old), BIN, None, true).unwrap();
    assert_eq!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["timeout"],
        HOOK_TIMEOUT_SECS
    );
    assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["timeout"], 120);
    assert_eq!(
        v["hooks"]["PostToolUse"][0]["hooks"][0]["timeout"],
        HOOK_TIMEOUT_SECS
    );
}

#[test]
fn the_wake_hook_is_an_async_rewake_entry_beside_the_session_start_hook() {
    let mut v = merge_hooks(None, BIN, None, true).unwrap();
    set_wake(&mut v, BIN, Switch::On).unwrap();
    let list = v["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(list.len(), 2, "the session start hook stays");
    let wake = &list[1]["hooks"][0];
    assert_eq!(
        wake["command"],
        "/opt/klatch/klatch --discover wake --timeout 3500"
    );
    assert_eq!(wake["async"], true);
    assert_eq!(wake["asyncRewake"], true);
    assert_eq!(wake["timeout"], 3600);
    let once = v.clone();
    set_wake(&mut v, BIN, Switch::On).unwrap();
    assert_eq!(v, once);
}

#[test]
fn the_default_merge_leaves_a_wake_entry_alone_and_wake_leaves_the_session_start_entry_alone() {
    let mut v = merge_hooks(None, BIN, None, true).unwrap();
    set_wake(&mut v, BIN, Switch::On).unwrap();
    let again = merge_hooks(Some(&v), BIN, Some("lobby"), true).unwrap();
    assert!(
        again["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["hooks"][0]["asyncRewake"] == true)
    );
    let mut off = again.clone();
    set_wake(&mut off, BIN, Switch::Off).unwrap();
    assert_eq!(off["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    assert!(commands(&off, "SessionStart")[0].contains("--board lobby"));
}

#[test]
fn the_new_hooks_refuse_a_settings_file_of_the_wrong_shape() {
    assert!(set_enforce(&mut json!([1]), BIN, Switch::On).is_err());
    assert!(set_wake(&mut json!({"hooks": {"SessionStart": 3}}), BIN, Switch::On).is_err());
}
#[test]
fn the_wake_hook_is_also_started_again_when_a_turn_ends() {
    let mut v = merge_hooks(None, BIN, None, true).unwrap();
    set_wake(&mut v, BIN, Switch::On).unwrap();
    let stop = &v["hooks"]["Stop"][0]["hooks"][0];
    assert_eq!(
        stop["command"],
        "/opt/klatch/klatch --discover wake --timeout 3500"
    );
    assert_eq!(
        (stop["async"].as_bool(), stop["asyncRewake"].as_bool()),
        (Some(true), Some(true))
    );
    let once = v.clone();
    set_wake(&mut v, BIN, Switch::On).unwrap();
    assert_eq!(v, once);
    set_wake(&mut v, BIN, Switch::Off).unwrap();
    assert!(
        v["hooks"].get("Stop").is_none(),
        "an empty Stop list was left behind"
    );
    let mut other =
        json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/usr/bin/mine"}]}]}});
    set_wake(&mut other, BIN, Switch::On).unwrap();
    set_wake(&mut other, BIN, Switch::Off).unwrap();
    assert_eq!(commands(&other, "Stop"), vec!["/usr/bin/mine".to_string()]);
}
#[test]
fn rerunning_init_brings_an_old_wake_entry_up_to_date_including_its_timeout() {
    let mut v = json!({"hooks": {
        "SessionStart": [{"hooks": [{"type": "command", "command": "/old/klatch --discover wake --timeout 540", "async": true, "asyncRewake": true, "timeout": 600}]}],
        "Stop": [{"hooks": [{"type": "command", "command": "/old/klatch --discover wake --timeout 540", "async": true, "asyncRewake": true, "timeout": 600}]}]
    }});
    set_wake(&mut v, BIN, Switch::Keep).unwrap();
    for event in ["SessionStart", "Stop"] {
        let h = &v["hooks"][event][0]["hooks"][0];
        assert_eq!(
            h["command"], "/opt/klatch/klatch --discover wake --timeout 3500",
            "{event}"
        );
        assert_eq!(h["timeout"], 3600, "{event}");
    }
}

#[test]
fn the_data_location_is_klatch_home_when_set_else_dot_klatch_under_home() {
    let set = next_steps(Some("/data/k".as_ref()), Some(Path::new("/h")));
    assert!(set.contains("in /data/k "), "{set}");
    let default = next_steps(None, Some(Path::new("/h")));
    assert!(default.contains("in /h/.klatch "), "{default}");
    assert!(next_steps(None, None).contains("~/.klatch"));
    assert!(default.contains("klatch doctor") && !default.contains('\u{2014}'));
}

#[test]
fn the_next_steps_tell_the_human_to_set_the_admin_passphrase_themselves() {
    let t = next_steps(None, Some(Path::new("/h")));
    assert!(
        t.contains("klatch admin init-passphrase") && t.contains("yourself"),
        "{t}"
    );
    assert!(t.contains("doctor") && !t.contains('\u{2014}'), "{t}");
}
