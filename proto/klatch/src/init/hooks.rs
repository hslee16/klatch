//! Building the hook commands, recognising ours in a settings file, and merging them in (#1502).

use super::*;

/// The Claude Code hook events klatch listens on.
pub const EVENTS: [&str; 3] = ["SessionStart", "UserPromptSubmit", "PostToolUse"];

/// A binary path a hook command can carry: absolute and free of anything a shell would act on.
pub(super) fn check_bin_path(bin: &str) -> Result<(), String> {
    if !bin.starts_with('/') {
        return Err(format!("the binary path must be absolute: {bin}"));
    }
    let unsafe_char = |c: char| matches!(c, '$' | '`' | '\'' | '"' | '\\') || c.is_control();
    if bin.chars().any(unsafe_char) {
        return Err(format!(
            "the binary path has a character a hook command cannot carry safely (a shell variable, quote, backslash or control character): {bin}"
        ));
    }
    Ok(())
}

pub(super) fn check_event_name(event: &str) -> Result<(), String> {
    if event.is_empty() || !event.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("bad hook event name: {event}"));
    }
    Ok(())
}

/// The binary path as it goes into a command: plain when it needs no quoting, else in single quotes.
pub(super) fn shell_path(bin: &str) -> String {
    let plain = bin
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_.-+:@%,=~".contains(c));
    if plain {
        bin.to_string()
    } else {
        format!("'{bin}'")
    }
}

/// The hook command for one event: the binary by absolute path, never a shell variable (permission rules refuse
/// those, #174). `board` adds the session-start board for that room (only on SessionStart; the consent to read a
/// room before joining it, DESIGN section 25.6).
pub fn hook_command(
    bin: &str,
    event: &str,
    board: Option<&str>,
    join: bool,
) -> Result<String, String> {
    command_with(bin, "--discover", event, (board, join))
}

/// `hook_command` for a harness: `mode` is the flag that tells klatch who the caller is (`--discover` for Claude Code,
/// `--codex` for Codex); `(board, join)` are the SessionStart choices.
pub(crate) fn command_with(
    bin: &str,
    mode: &str,
    event: &str,
    (board, join): (Option<&str>, bool),
) -> Result<String, String> {
    check_bin_path(bin)?;
    check_event_name(event)?;
    let mut cmd = format!("{} {mode} hook --event {event}", shell_path(bin));
    if let Some(room) = board {
        if !klatch::domain::valid_name(room) {
            return Err(format!("bad board room name: {room:?}"));
        }
        if event == "SessionStart" {
            cmd.push_str(&format!(" --board {room}"));
        }
    }
    if !join && event == "SessionStart" {
        cmd.push_str(" --no-join");
    }
    Ok(cmd)
}

/// A hook command klatch wrote (or a hand-written one of the same form).
pub(crate) fn is_klatch_command(cmd: &str) -> bool {
    klatch::harness::hooks_pointer::command_is_klatch(cmd)
}

pub(super) fn is_ours(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(is_klatch_command)
}

/// Which klatch hook a settings entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// SessionStart, UserPromptSubmit, PostToolUse and SessionEnd.
    Base,
    /// The PreToolUse hook that blocks an edit of a claimed path (`--enforce`).
    Enforce,
    /// The asyncRewake watcher (`--wake`).
    Wake,
    /// The PreToolUse hook that warns before an edit of a path another member holds (`--pre-edit-warn`).
    PreEdit,
}

/// Every hook klatch installed in a settings value: its event, its command and which one it is. Read only.
pub fn installed_hooks(settings: &Value) -> Vec<(String, String, HookKind)> {
    let Some(events) = settings.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (event, list) in events {
        for matcher in list.as_array().into_iter().flatten() {
            found.extend(klatch_hooks_in(event, matcher));
        }
    }
    found
}

/// The klatch hooks of one matcher entry under `event`: the event, the command and which hook it is.
pub(super) fn klatch_hooks_in<'a>(
    event: &'a str,
    matcher: &'a Value,
) -> impl Iterator<Item = (String, String, HookKind)> + 'a {
    let hooks = matcher.get("hooks").and_then(Value::as_array);
    hooks.into_iter().flatten().filter_map(move |hook| {
        let cmd = hook.get("command").and_then(Value::as_str)?;
        let kind = hook_kind(event, hook)?;
        Some((event.to_string(), cmd.to_string(), kind))
    })
}

/// Which klatch hook a settings entry under `event` is, or none when it is not klatch's.
pub(crate) fn hook_kind(event: &str, hook: &Value) -> Option<HookKind> {
    if is_wake(hook) {
        Some(HookKind::Wake)
    } else if !is_ours(hook) {
        None
    } else if event == "PreToolUse" && is_pre_edit(hook) {
        Some(HookKind::PreEdit)
    } else if event == "PreToolUse" {
        Some(HookKind::Enforce)
    } else {
        Some(HookKind::Base)
    }
}

/// The program a hook command runs: its first word, unquoted when `init` quoted it.
pub fn hook_program(command: &str) -> &str {
    match command.strip_prefix('\'') {
        Some(rest) => rest.split('\'').next().unwrap_or(rest),
        None => command.split(' ').next().unwrap_or(command),
    }
}

/// The klatch binary the hooks in the project and user settings run: the first program that exists. Read only.
pub fn installed_binary(project_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut files = vec![project_dir.join(".claude").join("settings.local.json")];
    files.extend(home.map(|h| h.join(".claude").join("settings.json")));
    files.iter().find_map(|file| {
        let value: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
        installed_hooks(&value)
            .iter()
            .map(|(_, cmd, _)| PathBuf::from(hook_program(cmd)))
            .find(|program| program.is_file())
    })
}

/// A line telling a caller which klatch is the installed one when the binary that ran (`exe`) is another, for a
/// child agent that was handed the wrong path (#513). None when they are the same file or nothing is installed.
pub fn other_binary_note(exe: &Path, project_dir: &Path, home: Option<&Path>) -> Option<String> {
    let installed = installed_binary(project_dir, home)?;
    if std::fs::canonicalize(&installed).ok() == std::fs::canonicalize(exe).ok() {
        return None;
    }
    Some(format!(
        "klatch: this is {}, but the installed klatch is {}; use the installed one (`{} convention` prints the commands for a child agent)",
        exe.display(),
        installed.display(),
        installed.display()
    ))
}

/// Make the list of hook matchers hold exactly one klatch entry running `cmd` (`keep`) or none (`!keep`), leaving
/// every other entry as it was. A matcher left with no hooks is dropped. Returns whether an entry of klatch's
/// stayed in place, so the caller knows whether to add one.
pub(super) fn sync_entries(list: &mut Vec<Value>, cmd: &str, keep: bool) -> bool {
    sync_owned(list, cmd, keep, is_ours)
}

/// `sync_entries` for the entries `owns` says are klatch's.
pub(super) fn sync_owned(
    list: &mut Vec<Value>,
    cmd: &str,
    keep: bool,
    owns: fn(&Value) -> bool,
) -> bool {
    let mut found = false;
    for matcher in list.iter_mut() {
        let Some(inner) = matcher.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        inner.retain_mut(|h| {
            if !owns(h) {
                return true;
            }
            if !keep || found {
                return false;
            }
            found = true;
            h["command"] = json!(cmd);
            true
        });
    }
    list.retain(|m| {
        m.get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|a| !a.is_empty())
    });
    found
}

/// The `hooks` object of a settings file, created if missing.
pub(super) fn hooks_of(
    settings: &mut Value,
) -> Result<&mut serde_json::Map<String, Value>, String> {
    let root = settings
        .as_object_mut()
        .ok_or("the settings file is not a JSON object")?;
    root.entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "\"hooks\" in the settings file is not an object".to_string())
}

/// The list of matchers for one event, created if missing.
pub(super) fn event_list<'a>(
    hooks: &'a mut serde_json::Map<String, Value>,
    event: &str,
) -> Result<&'a mut Vec<Value>, String> {
    hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| format!("hooks.{event} in the settings file is not a list"))
}

pub(super) fn new_entry(cmd: &str) -> Value {
    json!({"hooks": [{"type": "command", "command": cmd}]})
}

/// The timeout, in seconds, written into every ordinary hook entry `init` installs (#1246). The first read by a new
/// binary may wait on the Keychain for up to `klatch::keys::KEYCHAIN_READ_LIMIT_SECS` before it fails with a message
/// that says what to do; the Claude Code default was not verified, so the entry says it outright, with room to spare.
pub(super) const HOOK_TIMEOUT_SECS: u64 = 30;

/// Give every hook of klatch's in `list` a timeout of at least `HOOK_TIMEOUT_SECS`: add it where it is missing,
/// raise a lower one, keep a longer one the user chose.
pub(super) fn ensure_timeout(list: &mut [Value], owns: fn(&Value) -> bool) {
    for hook in list
        .iter_mut()
        .filter_map(|m| m.get_mut("hooks").and_then(Value::as_array_mut))
        .flatten()
        .filter(|h| owns(h))
    {
        if hook.get("timeout").and_then(Value::as_u64) < Some(HOOK_TIMEOUT_SECS) {
            hook["timeout"] = json!(HOOK_TIMEOUT_SECS);
        }
    }
}

/// `settings` with klatch's hook entries set to exactly what is asked: added when missing, updated in place when
/// present, everything else left as it was. Fails (writing nothing) when the file is not a JSON object or its
/// `hooks` is not an object.
pub fn merge_hooks(
    existing: Option<&Value>,
    bin: &str,
    board: Option<&str>,
    join: bool,
) -> Result<Value, String> {
    let mut settings = merge_commands(existing, |event| hook_command(bin, event, board, join))?;
    for event in EVENTS {
        ensure_timeout(event_list(hooks_of(&mut settings)?, event)?, is_ours);
    }
    Ok(settings)
}

/// `merge_hooks` with the hook command of each event given by `command` (Codex's differs from Claude Code's).
pub(crate) fn merge_commands(
    existing: Option<&Value>,
    command: impl Fn(&str) -> Result<String, String>,
) -> Result<Value, String> {
    let mut settings = existing.cloned().unwrap_or_else(|| json!({}));
    let hooks = hooks_of(&mut settings)?;
    for event in EVENTS {
        let mut cmd = command(event)?;
        let list = event_list(hooks, event)?;
        // a re-run of init without --checkpoint, --passive-claims or --machine-status keeps the consents the hook
        // already carries (#1408, #1401, #1403)
        if event == "PostToolUse" {
            cmd = with_flags(&cmd, carried_flags(list));
        }
        if !sync_entries(list, &cmd, true) {
            list.push(new_entry(&cmd));
        }
    }
    Ok(settings)
}

/// Install (`on`) or remove (`!on`) klatch's SessionEnd hook, which marks a finished session done so it leaves the
/// board at once. Everything else in the settings is left as it was.
pub fn set_session_end(settings: &mut Value, bin: &str, on: bool) -> Result<(), String> {
    const EVENT: &str = "SessionEnd";
    let mut cmd = hook_command(bin, EVENT, None, true)?;
    if !on
        && settings
            .as_object()
            .is_some_and(|r| !r.contains_key("hooks"))
    {
        return Ok(());
    }
    let hooks = hooks_of(settings)?;
    if !on && !hooks.contains_key(EVENT) {
        return Ok(());
    }
    // a SessionEnd hook put back later carries the consents PostToolUse still carries, so switching session-end off
    // and on never drops the checkpoint, the passive claims release or the machine status consent
    let post = hooks
        .get("PostToolUse")
        .and_then(Value::as_array)
        .map_or((false, false, false, false), |l| carried_flags(l));
    let list = event_list(hooks, EVENT)?;
    cmd = with_flags(&cmd, either(carried_flags(list), post));
    if !sync_entries(list, &cmd, on) && on {
        list.push(new_entry(&cmd));
    }
    ensure_timeout(list, is_ours);
    if list.is_empty() {
        hooks.remove(EVENT);
    }
    Ok(())
}
