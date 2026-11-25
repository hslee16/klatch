//! The consent flags carried on the PostToolUse and Stop hook commands, and the Stop hook derived from them (#1502).

use super::*;

/// The consents either of two flag sets carries.
pub(super) fn either(a: Flags, b: Flags) -> Flags {
    (a.0 || b.0, a.1 || b.1, a.2 || b.2, a.3 || b.3)
}

/// Whether any consent is on.
pub(super) fn any_on(f: Flags) -> bool {
    f.0 || f.1 || f.2 || f.3
}

/// The flag that is the machine checkpoint consent (#1408): a hook command carries it, or records nothing.
pub(super) const CHECKPOINT_FLAG: &str = " --checkpoint";

/// The flag that is the passive claims consent (#1401): a hook command carries it, or claims nothing.
pub(super) const PASSIVE_FLAG: &str = " --passive-claims";
/// The flag that is the machine status consent (#1403): a hook command carries it, or writes no status.
pub(super) const MACHINE_STATUS_FLAG: &str = " --machine-status";
/// The flag that is the stop warning consent (#1344): a hook command carries it, or prints no warning.
pub(super) const STOP_WARN_FLAG: &str = " --stop-warn";

/// The consents a hook command carries, as `(passive claims, checkpoint, machine status, stop warning)`. Read word by word, so the
/// order they are written in does not matter.
pub(super) type Flags = (bool, bool, bool, bool);

pub(super) fn flags_of(cmd: &str) -> Flags {
    let has = |flag: &str| cmd.split(' ').any(|w| w == flag.trim_start());
    (
        has(PASSIVE_FLAG),
        has(CHECKPOINT_FLAG),
        has(MACHINE_STATUS_FLAG),
        has(STOP_WARN_FLAG),
    )
}

/// `cmd` without either consent flag.
pub(super) fn bare(cmd: &str) -> String {
    cmd.split(' ')
        .filter(|w| {
            ![
                PASSIVE_FLAG,
                CHECKPOINT_FLAG,
                MACHINE_STATUS_FLAG,
                STOP_WARN_FLAG,
            ]
            .iter()
            .any(|f| *w == f.trim_start())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `cmd` carrying exactly the flags given, in one fixed order (passive claims, checkpoint, machine status, stop warning), so a command always
/// reads the same for the same consents.
pub(super) fn with_flags(cmd: &str, (passive, checkpoint, status, stop_warn): Flags) -> String {
    let mut out = bare(cmd);
    if passive {
        out.push_str(PASSIVE_FLAG);
    }
    if checkpoint {
        out.push_str(CHECKPOINT_FLAG);
    }
    if status {
        out.push_str(MACHINE_STATUS_FLAG);
    }
    if stop_warn {
        out.push_str(STOP_WARN_FLAG);
    }
    out
}

/// The consents any klatch hook in `list` carries.
pub(super) fn carried_flags(list: &[Value]) -> Flags {
    list.iter()
        .filter_map(|m| m.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|h| is_ours(h))
        .filter_map(|h| h.get("command").and_then(Value::as_str))
        .map(flags_of)
        .fold((false, false, false, false), either)
}

/// The Stop entry that carries the consents: klatch's own Stop hook, never the wake watcher.
pub(super) fn is_flag_stop(hook: &Value) -> bool {
    is_ours(hook)
        && hook
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|c| c.contains(" hook --event Stop"))
}

/// Make the Stop entry match the consents the PostToolUse hook carries: one entry that carries them, or none when
/// neither is on. Everything else under Stop (the wake watcher, other tools' hooks) is left as it was.
pub(super) fn sync_stop(settings: &mut Value, bin: &str) -> Result<(), String> {
    let flags = post_tool_use_flags(settings)?;
    let on = any_on(flags);
    if !on && event_absent(settings, "Stop") {
        return Ok(());
    }
    let cmd = with_flags(&hook_command(bin, "Stop", None, true)?, flags);
    let entry = json!({"type": "command", "command": cmd, "timeout": HOOK_TIMEOUT_SECS});
    let hooks = hooks_of(settings)?;
    let list = event_list(hooks, "Stop")?;
    if !sync_owned(list, &cmd, on, is_flag_stop) && on {
        list.push(json!({"hooks": [entry]}));
    } else if on {
        refresh_owned(list, is_flag_stop, &entry);
    }
    if list.is_empty() {
        hooks.remove("Stop");
    }
    Ok(())
}

/// The consents the PostToolUse hook carries, none when it has no entry.
pub(super) fn post_tool_use_flags(settings: &mut Value) -> Result<Flags, String> {
    if event_absent(settings, "PostToolUse") {
        return Ok((false, false, false, false));
    }
    Ok(carried_flags(event_list(
        hooks_of(settings)?,
        "PostToolUse",
    )?))
}

/// Put one consent flag on, or take it off, every klatch command of `event` in the settings.
pub(super) fn flag_event(
    settings: &mut Value,
    event: &str,
    flag: &str,
    on: bool,
) -> Result<(), String> {
    if event_absent(settings, event) {
        return Ok(());
    }
    let list = event_list(hooks_of(settings)?, event)?;
    for hook in list
        .iter_mut()
        .filter_map(|m| m.get_mut("hooks").and_then(Value::as_array_mut))
        .flatten()
        .filter(|h| is_ours(h))
    {
        let Some(cmd) = hook.get("command").and_then(Value::as_str) else {
            continue;
        };
        let (passive, checkpoint, status, warn) = flags_of(cmd);
        let flags = match flag {
            PASSIVE_FLAG => (on, checkpoint, status, warn),
            CHECKPOINT_FLAG => (passive, on, status, warn),
            MACHINE_STATUS_FLAG => (passive, checkpoint, on, warn),
            _ => (passive, checkpoint, status, on),
        };
        hook["command"] = json!(with_flags(cmd, flags));
    }
    Ok(())
}

/// The machine checkpoint (#1408): the same flag on klatch's PostToolUse and SessionEnd commands and on its Stop hook,
/// so the trail is kept, the checkpoint is written at each turn's end and a last one at the end of the session.
/// `Switch::Keep` leaves things as they are. The flag is the consent: a hook command without it records nothing.
pub fn set_checkpoint(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    set_flag(settings, bin, CHECKPOINT_FLAG, how)
}

/// Passive claims (#1401): the `--passive-claims` flag on klatch's PostToolUse and SessionEnd commands and on its Stop
/// hook, so a file a member edits is claimed for a short lease and released when the turn or session ends. The flag is
/// the consent: a hook command without it claims nothing. `Switch::Keep` leaves things as they are.
pub fn set_passive_claims(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    set_flag(settings, bin, PASSIVE_FLAG, how)
}

/// The machine status (#1403): the `--machine-status` flag on klatch's PostToolUse and SessionEnd commands and on its
/// Stop hook, so the member's status follows its activity (working at the first tool call after idle, idle or parked
/// at a turn's end). The flag is the consent: a hook command without it writes no status. SessionEnd carries it as a
/// record of the consent only (it marks the session done as it always did). `Switch::Keep` leaves things as they are.
pub fn set_machine_status(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    set_flag(settings, bin, MACHINE_STATUS_FLAG, how)
}

/// The stop warning (#1344): the `--stop-warn` flag on klatch's PostToolUse and SessionEnd commands and on its Stop hook,
/// so a turn that ends with background tasks or crons still running prints one warn-only line (counts and kinds, never
/// a task's text). The flag is the consent: a hook command without it prints nothing. `Switch::Keep` leaves things as
/// they are.
pub fn set_stop_warn(settings: &mut Value, bin: &str, how: Switch) -> Result<(), String> {
    set_flag(settings, bin, STOP_WARN_FLAG, how)
}

pub(super) fn set_flag(
    settings: &mut Value,
    bin: &str,
    flag: &str,
    how: Switch,
) -> Result<(), String> {
    if how == Switch::Keep {
        return Ok(());
    }
    let on = how == Switch::On;
    for event in ["PostToolUse", "SessionEnd"] {
        flag_event(settings, event, flag, on)?;
    }
    sync_stop(settings, bin)
}

/// What to do with an optional hook: add it, remove it, or leave things as they are (update an entry already there,
/// add none). Optional hooks are opt-in, so nothing here runs unless a flag asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    On,
    Off,
    Keep,
}

impl Switch {
    /// The switch two flags (`--x` and `--no-x`) set; neither leaves things as they are.
    pub fn from_flags(on: bool, off: bool) -> Self {
        match (on, off) {
            (true, _) => Self::On,
            (_, true) => Self::Off,
            _ => Self::Keep,
        }
    }
}
