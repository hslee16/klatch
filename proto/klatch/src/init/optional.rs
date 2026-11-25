//! The optional hooks (enforce, wake): installed or removed on their own consent (#1502).

use super::*;

/// One optional hook klatch can install.
pub(super) struct Optional {
    event: &'static str,
    entry: fn(&str) -> Result<Value, String>,
    owns: fn(&Value) -> bool,
    /// Replace an existing entry whole, not only its command.
    refresh: bool,
}

/// The tools the claim hook looks at.
pub(super) const ENFORCE_MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit|Bash";

/// How long the wake watcher may run, and the timeout Claude Code gets for it (a little more).
pub(super) const WAKE_WATCH_SECS: u64 = 3500;
pub(super) const WAKE_HOOK_TIMEOUT_SECS: u64 = 3600;

pub(super) fn is_wake_command(cmd: &str) -> bool {
    cmd.contains("klatch") && cmd.contains(" --discover wake")
}

pub(super) fn is_wake(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(is_wake_command)
}

pub(super) fn enforce_entry(bin: &str) -> Result<Value, String> {
    let cmd = hook_command(bin, "PreToolUse", None, true)?;
    Ok(
        json!({"matcher": ENFORCE_MATCHER, "hooks": [{"type": "command", "command": cmd, "timeout": HOOK_TIMEOUT_SECS}]}),
    )
}

pub(super) fn wake_entry(bin: &str) -> Result<Value, String> {
    check_bin_path(bin)?;
    let cmd = format!(
        "{} --discover wake --timeout {WAKE_WATCH_SECS}",
        shell_path(bin)
    );
    Ok(json!({"hooks": [{
        "type": "command", "command": cmd, "async": true, "asyncRewake": true,
        "timeout": WAKE_HOOK_TIMEOUT_SECS
    }]}))
}

pub(super) fn set_optional(
    settings: &mut Value,
    bin: &str,
    (spec, how): (&Optional, Switch),
) -> Result<(), String> {
    let entry = (spec.entry)(bin)?;
    let cmd = entry["hooks"][0]["command"]
        .as_str()
        .ok_or("no hook command")?
        .to_string();
    if how != Switch::On && event_absent(settings, spec.event) {
        return Ok(());
    }
    let hooks = hooks_of(settings)?;
    let list = event_list(hooks, spec.event)?;
    sync_optional(list, spec, (entry, &cmd), how);
    if list.is_empty() {
        hooks.remove(spec.event);
    }
    Ok(())
}

/// Bring the entries of an optional hook in line with `how`: add, refresh or drop them.
pub(super) fn sync_optional(
    list: &mut Vec<Value>,
    spec: &Optional,
    (entry, cmd): (Value, &str),
    how: Switch,
) {
    if !sync_owned(list, cmd, how != Switch::Off, spec.owns) && how == Switch::On {
        list.push(entry);
    } else if how != Switch::Off && spec.refresh {
        refresh_owned(list, spec.owns, &entry["hooks"][0]);
    } else if how != Switch::Off && spec.event == "PreToolUse" {
        ensure_timeout(list, is_ours);
    }
}

/// Whether a settings object has no entry for the event (a settings value that is not an object counts as present).
pub(super) fn event_absent(settings: &Value, event: &str) -> bool {
    let has_event =
        |r: &serde_json::Map<String, Value>| r.get("hooks").is_some_and(|h| h.get(event).is_some());
    settings.as_object().is_some_and(|r| !has_event(r))
}

/// An entry written by an older klatch: bring every field of the hooks `owns` claims up to date, not just the command.
pub(super) fn refresh_owned(list: &mut [Value], owns: fn(&Value) -> bool, fresh: &Value) {
    for hook in list
        .iter_mut()
        .filter_map(|m| m.get_mut("hooks").and_then(Value::as_array_mut))
        .flatten()
        .filter(|h| owns(h))
    {
        *hook = fresh.clone();
    }
}

/// The PreToolUse claim hook (#33): blocks an edit of a path another member claimed. Opt in with `--enforce`.
pub fn set_enforce(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    let spec = Optional {
        event: "PreToolUse",
        entry: enforce_entry,
        owns: is_ours,
        refresh: false,
    };
    set_optional(settings, bin, (&spec, how))
}

/// The wake watcher (#37): an `asyncRewake` hook that wakes an idle session for unread messages. Opt in with
/// `--wake`.
pub fn set_wake(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    // At session start, and again whenever a turn ends: a watcher is one-shot, so each turn starts the next one.
    for event in ["SessionStart", "Stop"] {
        let spec = Optional {
            event,
            entry: wake_entry,
            owns: is_wake,
            refresh: true,
        };
        set_optional(settings, bin, (&spec, how))?;
    }
    Ok(())
}
