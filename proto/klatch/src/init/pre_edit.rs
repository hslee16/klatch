//! The pre-edit hook (#1402, #1458): one PreToolUse entry of its own for the Edit tools, carrying `--pre-edit-warn`
//! (tell the member when another member holds the path it is about to edit) and, as a second consent, `--pre-edit-claims`
//! (also take the passive claim before the first edit). It is separate from the blocking `--enforce` entry: each is
//! installed, refreshed and removed without touching the other, and `is_enforce` is what tells them apart.

use super::*;

/// The flag that is the pre-edit warning consent: the entry's command carries it, or the event is the blocking hook.
pub(super) const PRE_EDIT_WARN_FLAG: &str = "--pre-edit-warn";
/// The flag that is the first-edit claim consent: the entry's command carries it, or it claims nothing.
pub(super) const PRE_EDIT_CLAIMS_FLAG: &str = "--pre-edit-claims";
/// The tools the pre-edit hook looks at: the ones that write a file (never a shell command).
pub(super) const PRE_EDIT_MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit";

fn carries(hook: &Value, flag: &str) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.split(' ').any(|w| w == flag))
}

/// Whether a hook is klatch's pre-edit one.
pub(super) fn is_pre_edit(hook: &Value) -> bool {
    is_ours(hook) && carries(hook, PRE_EDIT_WARN_FLAG)
}

/// Whether a hook is klatch's blocking `--enforce` one: ours, and not the pre-edit one.
pub(super) fn is_enforce(hook: &Value) -> bool {
    is_ours(hook) && !carries(hook, PRE_EDIT_WARN_FLAG)
}

/// `(installed, takes the first-edit claim)` for the pre-edit hook in a settings value. Read only.
pub fn pre_edit_state(settings: &Value) -> (bool, bool) {
    let hooks = settings
        .pointer("/hooks/PreToolUse")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|h| is_pre_edit(h));
    hooks.fold((false, false), |(_, claims), h| {
        (true, claims || carries(h, PRE_EDIT_CLAIMS_FLAG))
    })
}

/// Put the pre-edit entry in the state asked for: `Some(claims)` installs or updates it, `None` removes it.
fn write(settings: &mut Value, bin: &str, level: Option<bool>) -> Result<(), String> {
    if level.is_none() && event_absent(settings, "PreToolUse") {
        return Ok(());
    }
    let mut cmd = format!(
        "{} {PRE_EDIT_WARN_FLAG}",
        hook_command(bin, "PreToolUse", None, true)?
    );
    if level == Some(true) {
        cmd.push_str(&format!(" {PRE_EDIT_CLAIMS_FLAG}"));
    }
    let hook = json!({"type": "command", "command": cmd, "timeout": HOOK_TIMEOUT_SECS});
    let hooks = hooks_of(settings)?;
    let list = event_list(hooks, "PreToolUse")?;
    let on = level.is_some();
    if !sync_owned(list, &cmd, on, is_pre_edit) && on {
        list.push(json!({"matcher": PRE_EDIT_MATCHER, "hooks": [hook]}));
    } else if on {
        refresh_owned(list, is_pre_edit, &hook);
    }
    if list.is_empty() {
        hooks.remove("PreToolUse");
    }
    Ok(())
}

/// The pre-edit warning (#1402): an own PreToolUse entry for the Edit tools that prints one bounded line when another
/// member holds the path about to be edited. The flag is the consent; `Switch::Keep` leaves things as they are, and off
/// removes the entry, the first-edit claim with it.
pub fn set_pre_edit_warn(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    match how {
        Switch::Keep => Ok(()),
        Switch::Off => write(settings, bin, None),
        Switch::On => write(settings, bin, Some(pre_edit_state(settings).1)),
    }
}

/// The first-edit passive claim (#1458): the same entry also takes the passive claim before an edit. It needs passive
/// claims on (the Stop and SessionEnd hooks that release it carry that consent), and it implies the warning.
pub fn set_pre_edit_claims(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    match how {
        Switch::Keep => Ok(()),
        Switch::Off if pre_edit_state(settings).0 => write(settings, bin, Some(false)),
        Switch::Off => Ok(()),
        Switch::On if !post_tool_use_flags(settings)?.0 => Err(
            "--pre-edit-claims needs --passive-claims (the claim a first edit takes is released by the passive claims hooks at the end of the turn)"
                .into(),
        ),
        Switch::On => write(settings, bin, Some(true)),
    }
}

/// Passive claims went off: the first-edit claim has nothing to release it, so it goes too and the warning stays.
pub fn drop_orphan_pre_edit_claims(settings: &mut Value, bin: &str) -> Result<(), String> {
    if pre_edit_state(settings).1 && !post_tool_use_flags(settings)?.0 {
        write(settings, bin, Some(false))?;
    }
    Ok(())
}
