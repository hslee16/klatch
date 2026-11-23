//! `klatch consent` (#256): show the consents `klatch init` granted and switch each on or off after install. It edits
//! only klatch's own hook entries in the settings files init already writes, never adds klatch where init did not
//! install it, shows the diff first, and takes effect at the next session. The pure parts are testable here; the file
//! handling reuses init's.

use crate::init::{self, HookKind, Io, Places, Scope, Switch};
use anyhow::{Context, bail};
use serde_json::Value;
use std::path::PathBuf;

/// What the user asked for on the command line.
pub struct Options {
    pub scope: Option<Scope>,
    pub name: Option<String>,
    pub value: Option<String>,
    pub yes: bool,
}

/// One consent init grants, and the value to give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    /// The room whose board SessionStart shows before joining, or none.
    Board(Option<String>),
    /// Join every new session to the default lobby.
    Join(bool),
    SessionEnd(bool),
    Enforce(bool),
    Wake(bool),
    /// The machine checkpoint (#1408).
    Checkpoint(bool),
    /// Passive claims (#1401).
    PassiveClaims(bool),
}

/// Every consent as it stands in one settings value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub board: Option<String>,
    pub join: bool,
    pub session_end: bool,
    pub enforce: bool,
    pub wake: bool,
    pub checkpoint: bool,
    pub passive_claims: bool,
    /// The program klatch's hooks run, kept as it is when a consent changes.
    bin: String,
}

/// What each consent changes, shown once under the state.
const LEGEND: &str = "What each one changes (only klatch's own hook entries in the files above; other hooks and settings are never touched):
  board: SessionStart prints that room's board before the session joins anything (off: nothing is read)
  join: SessionStart joins the session to the default lobby (off: --no-join is added to that command)
  session-end: a SessionEnd hook marks a finished session done in klatch's store
  enforce: a PreToolUse hook blocks an edit of a path another member claimed
  wake: asyncRewake hooks on SessionStart and Stop wake an idle session for unread messages
  checkpoint: a Stop hook and a flag on PostToolUse and SessionEnd keep a machine checkpoint (tool classes and edited paths, never a command or content)
  passive-claims: a flag on PostToolUse, SessionEnd and Stop claims the repo-relative path of each file you edit for 5 minutes (renewed by each edit, released when the turn ends); it warns when another member holds the path and never blocks
A change takes effect at the next session; one already running keeps the hooks it started with.
";

fn words(command: &str) -> Vec<&str> {
    command.split(' ').collect()
}

/// The consents in a settings value, or none when klatch's hooks are not installed in it.
pub fn read_state(settings: &Value) -> Option<State> {
    let found = init::installed_hooks(settings);
    let (_, first, _) = found.iter().find(|(_, _, kind)| *kind == HookKind::Base)?;
    let start = found
        .iter()
        .find(|(event, _, kind)| event == "SessionStart" && *kind == HookKind::Base)
        .map(|(_, cmd, _)| words(cmd));
    let board = start.as_ref().and_then(|w| {
        w.windows(2)
            .find(|p| p[0] == "--board")
            .map(|p| p[1].to_string())
    });
    let join = start.is_none_or(|w| !w.contains(&"--no-join"));
    let has = |event: &str, kind: HookKind| found.iter().any(|(e, _, k)| e == event && *k == kind);
    Some(State {
        board,
        join,
        session_end: has("SessionEnd", HookKind::Base),
        enforce: has("PreToolUse", HookKind::Enforce),
        wake: found.iter().any(|(_, _, kind)| *kind == HookKind::Wake),
        checkpoint: stop_flags(&found).1,
        passive_claims: stop_flags(&found).0,
        bin: init::hook_program(first).to_string(),
    })
}

/// The consents the Stop hooks of klatch's carry, as `(passive claims, checkpoint)`.
fn stop_flags(found: &[(String, String, HookKind)]) -> (bool, bool) {
    let mut flags = (false, false);
    for (_, cmd, _) in found
        .iter()
        .filter(|(e, _, k)| e == "Stop" && *k == HookKind::Base)
    {
        let w = words(cmd);
        flags.0 |= w.contains(&"--passive-claims");
        flags.1 |= w.contains(&"--checkpoint");
    }
    flags
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

impl State {
    /// One line per consent, as `show` prints it.
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("board: {}", self.board.as_deref().unwrap_or("off")),
            format!("join: {}", on_off(self.join)),
            format!("session-end: {}", on_off(self.session_end)),
            format!("enforce: {}", on_off(self.enforce)),
            format!("wake: {}", on_off(self.wake)),
            format!("checkpoint: {}", on_off(self.checkpoint)),
            format!("passive-claims: {}", on_off(self.passive_claims)),
        ]
    }
}

impl Setting {
    /// Parse `<name> <value>`: a switch takes on or off, the board takes off or a room name.
    pub fn parse(name: &str, value: &str) -> Result<Self, String> {
        let switch = || match value {
            "on" => Ok(true),
            "off" => Ok(false),
            other => Err(format!("{name} takes on or off, not {other:?}")),
        };
        match name {
            "board" if value == "off" => Ok(Self::Board(None)),
            "board" if klatch::domain::valid_name(value) => Ok(Self::Board(Some(value.into()))),
            "board" => Err(format!("board takes off or a room name, not {value:?}")),
            "join" => switch().map(Self::Join),
            "session-end" => switch().map(Self::SessionEnd),
            "enforce" => switch().map(Self::Enforce),
            "wake" => switch().map(Self::Wake),
            "checkpoint" => switch().map(Self::Checkpoint),
            "passive-claims" => switch().map(Self::PassiveClaims),
            other => Err(format!(
                "unknown consent {other:?}: board, join, session-end, enforce, wake, checkpoint or passive-claims"
            )),
        }
    }
}

fn switch_of(on: bool) -> Switch {
    if on { Switch::On } else { Switch::Off }
}

/// Rewrite klatch's own SessionStart command to carry the board room and the join choice; nothing else changes.
fn rewrite_session_start(
    settings: &mut Value,
    bin: &str,
    board: Option<&str>,
    join: bool,
) -> Result<(), String> {
    let cmd = init::hook_command(bin, "SessionStart", board, join)?;
    let groups = settings["hooks"]["SessionStart"]
        .as_array_mut()
        .ok_or("hooks.SessionStart is not a list")?;
    for hook in groups
        .iter_mut()
        .filter_map(|g| g.get_mut("hooks").and_then(Value::as_array_mut))
        .flatten()
        .filter(|h| init::hook_kind("SessionStart", h) == Some(HookKind::Base))
    {
        hook["command"] = Value::String(cmd.clone());
    }
    Ok(())
}

/// Set one consent in a settings value. Fails when klatch's hooks are not installed in it (init adds them, this does
/// not) or the value is of the wrong shape; the value is then unchanged.
pub fn apply(settings: &mut Value, setting: &Setting) -> Result<(), String> {
    let state = read_state(settings)
        .ok_or("klatch's hooks are not installed here; run `klatch init` first")?;
    let mut next = settings.clone();
    let bin = state.bin.as_str();
    match setting {
        Setting::Board(room) => rewrite_session_start(&mut next, bin, room.as_deref(), state.join)?,
        Setting::Join(on) => rewrite_session_start(&mut next, bin, state.board.as_deref(), *on)?,
        Setting::SessionEnd(on) => init::set_session_end(&mut next, bin, *on)?,
        Setting::Enforce(on) => init::set_enforce(&mut next, bin, switch_of(*on))?,
        Setting::Wake(on) => init::set_wake(&mut next, bin, switch_of(*on))?,
        Setting::Checkpoint(on) => init::set_checkpoint(&mut next, bin, switch_of(*on))?,
        Setting::PassiveClaims(on) => init::set_passive_claims(&mut next, bin, switch_of(*on))?,
    }
    *settings = next;
    Ok(())
}

/// A settings file and what a consent change would make of it.
struct Change {
    label: &'static str,
    path: PathBuf,
    old: Value,
    new: Value,
}

/// Print the consents of each scope, read only.
fn show(scope: Scope, io: &mut Io, places: &Places) -> anyhow::Result<()> {
    for (label, path) in init::targets(scope, places)? {
        writeln!(io.out, "{label} ({}):", path.display())?;
        match init::read_settings(&path)?.as_ref().and_then(read_state) {
            Some(state) => {
                for line in state.lines() {
                    writeln!(io.out, "  {line}")?;
                }
            }
            None => writeln!(io.out, "  klatch's hooks are not installed here")?,
        }
    }
    write!(io.out, "{LEGEND}")?;
    Ok(())
}

/// Read every scope first; one that cannot be read, or none with klatch installed, stops the run before a write.
fn plan(
    scope: Scope,
    setting: &Setting,
    io: &mut Io,
    places: &Places,
) -> anyhow::Result<Vec<Change>> {
    let mut changes = Vec::new();
    for (label, path) in init::targets(scope, places)? {
        let Some(old) = init::read_settings(&path)?.filter(|v| read_state(v).is_some()) else {
            writeln!(
                io.out,
                "{} ({label}): klatch is not installed here, skipped",
                path.display()
            )?;
            continue;
        };
        let mut new = old.clone();
        apply(&mut new, setting)
            .map_err(|e| anyhow::anyhow!("{}: {e}; nothing was written", path.display()))?;
        changes.push(Change {
            label,
            path,
            old,
            new,
        });
    }
    if changes.is_empty() {
        bail!(
            "klatch's hooks are not installed in that scope; run `klatch init` first (nothing was written)"
        );
    }
    Ok(changes)
}

/// Show each planned change and write it on a yes (or `--yes`).
fn write_changes(changes: Vec<Change>, yes: bool, io: &mut Io) -> anyhow::Result<()> {
    let mut wrote = false;
    for Change {
        label,
        path,
        old,
        new,
    } in changes
    {
        if old == new {
            writeln!(
                io.out,
                "{} ({label}): already set, nothing to change",
                path.display()
            )?;
            continue;
        }
        writeln!(io.out, "{} ({label}):", path.display())?;
        let diff = init::diff(
            &serde_json::to_string_pretty(&old)?,
            &serde_json::to_string_pretty(&new)?,
        );
        write!(io.out, "{diff}")?;
        if !yes && !io.ask(&format!("Write {}?", path.display()), false)? {
            writeln!(io.out, "skipped {}", path.display())?;
            continue;
        }
        init::write_settings(&path, &serde_json::to_string_pretty(&new)?, io)?;
        wrote = true;
    }
    if wrote {
        writeln!(
            io.out,
            "This takes effect at the next session; one already running keeps the hooks it started with."
        )?;
    }
    Ok(())
}

/// Run `klatch consent`: with no name, show; with a name and a value, change that consent in `--scope`.
pub fn run(options: &Options, io: &mut Io, places: &Places) -> anyhow::Result<()> {
    let (name, value) = match (&options.name, &options.value) {
        (None, None) => return show(options.scope.unwrap_or(Scope::Both), io, places),
        (Some(name), Some(value)) => (name, value),
        _ => bail!(
            "a change needs a consent and a value, for example `klatch consent join off --scope project`"
        ),
    };
    let setting = Setting::parse(name, value).map_err(|e| anyhow::anyhow!(e))?;
    let scope = options
        .scope
        .context("a change needs --scope project, user or both (nothing is changed by default)")?;
    let changes = plan(scope, &setting, io, places)?;
    write_changes(changes, options.yes, io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BIN: &str = "/opt/klatch/klatch";

    fn installed() -> Value {
        let mut v = init::merge_hooks(None, BIN, None, true).unwrap();
        init::set_session_end(&mut v, BIN, true).unwrap();
        v
    }

    #[test]
    fn nothing_is_read_from_a_settings_value_without_klatch() {
        assert!(read_state(&json!({})).is_none());
        let foreign =
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/usr/bin/x"}]}]}});
        assert!(read_state(&foreign).is_none());
    }

    #[test]
    fn the_state_reads_back_what_was_set() {
        let mut v = installed();
        let s = read_state(&v).unwrap();
        assert_eq!(
            (s.board, s.join, s.session_end, s.enforce, s.wake),
            (None, true, true, false, false)
        );
        apply(&mut v, &Setting::Board(Some("lobby".into()))).unwrap();
        apply(&mut v, &Setting::Join(false)).unwrap();
        apply(&mut v, &Setting::Enforce(true)).unwrap();
        apply(&mut v, &Setting::Wake(true)).unwrap();
        let s = read_state(&v).unwrap();
        assert_eq!(
            (s.board.as_deref(), s.join, s.enforce, s.wake),
            (Some("lobby"), false, true, true)
        );
    }

    #[test]
    fn board_and_join_are_independent() {
        let mut v = installed();
        apply(&mut v, &Setting::Join(false)).unwrap();
        apply(&mut v, &Setting::Board(Some("ops".into()))).unwrap();
        let cmd = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(
            cmd.contains("--board ops") && cmd.contains("--no-join"),
            "{cmd}"
        );
        apply(&mut v, &Setting::Board(None)).unwrap();
        let s = read_state(&v).unwrap();
        assert_eq!((s.board, s.join), (None, false));
    }

    #[test]
    fn a_change_keeps_the_binary_path_the_hooks_already_run() {
        let mut v = installed();
        apply(&mut v, &Setting::Join(false)).unwrap();
        let cmd = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.starts_with(BIN), "{cmd}");
    }

    #[test]
    fn applying_without_klatch_installed_fails_and_changes_nothing() {
        let mut v = json!({"hooks": {}});
        assert!(apply(&mut v, &Setting::Wake(true)).is_err());
        assert_eq!(v, json!({"hooks": {}}));
    }

    #[test]
    fn names_and_values_are_checked() {
        assert_eq!(Setting::parse("join", "off"), Ok(Setting::Join(false)));
        assert_eq!(
            Setting::parse("board", "lobby"),
            Ok(Setting::Board(Some("lobby".into())))
        );
        assert_eq!(Setting::parse("board", "off"), Ok(Setting::Board(None)));
        for (n, v) in [
            ("join", "lobby"),
            ("board", "a b"),
            ("x", "on"),
            ("wake", ""),
        ] {
            assert!(Setting::parse(n, v).is_err(), "{n} {v}");
        }
    }
}
