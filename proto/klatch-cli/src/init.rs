//! `klatch init`: writes the Claude Code hook entries that call this binary, with consent per scope.
//! The pure parts (building the hook command, merging into a settings file, showing a diff) live here so they are
//! testable; the prompting and file writing are in `run`.

use serde_json::{Value, json};

/// Where `init` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Scope {
    Project,
    User,
    Both,
}

/// What the user asked for on the command line.
pub struct Options {
    pub scope: Option<Scope>,
    pub board: Option<String>,
    pub yes: bool,
}

/// Ask a yes or no question; an empty answer, end of input and anything but yes mean `default`.
fn ask(
    input: &mut dyn std::io::BufRead,
    out: &mut dyn std::io::Write,
    question: &str,
    default_yes: bool,
) -> anyhow::Result<bool> {
    write!(
        out,
        "{question} [{}] ",
        if default_yes { "Y/n" } else { "y/N" }
    )?;
    out.flush()?;
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(match line.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    })
}

/// Run `init`: choose scope and board (asking unless `--yes`), show each diff, write only on a yes.
pub fn run(
    options: &Options,
    input: &mut dyn std::io::BufRead,
    out: &mut dyn std::io::Write,
    project_dir: &std::path::Path,
    home: Option<&std::path::Path>,
    bin: &std::path::Path,
) -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    let bin = bin
        .to_str()
        .context("the path of this binary is not valid text")?;
    let scope = match options.scope {
        Some(s) => s,
        None if options.yes => {
            bail!("init --yes asks nothing, so it needs --scope project, user or both")
        }
        None => {
            write!(
                out,
                "Where should klatch install its hooks? project (.claude/settings.local.json here), user (~/.claude/settings.json) or both [project]: "
            )?;
            out.flush()?;
            let mut line = String::new();
            input.read_line(&mut line)?;
            match line.trim().to_ascii_lowercase().as_str() {
                "" | "project" => Scope::Project,
                "user" => Scope::User,
                "both" => Scope::Both,
                other => bail!("unknown scope {other:?}: say project, user or both"),
            }
        }
    };
    let board = match &options.board {
        Some(room) => Some(room.clone()),
        None if options.yes => None,
        None => ask(
            input,
            out,
            "Show the lobby board when a session starts? This reads the lobby before the session has joined it.",
            false,
        )?
        .then(|| "lobby".to_string()),
    };
    let mut targets: Vec<(&str, std::path::PathBuf)> = Vec::new();
    if matches!(scope, Scope::Project | Scope::Both) {
        targets.push((
            "project",
            project_dir.join(".claude").join("settings.local.json"),
        ));
    }
    if matches!(scope, Scope::User | Scope::Both) {
        let home = home.context("HOME is not set, so the user settings file cannot be found")?;
        targets.push(("user", home.join(".claude").join("settings.json")));
    }
    // Read and merge everything first: a file that cannot be read stops the whole run before anything is written.
    let mut plans = Vec::new();
    for (label, path) in targets {
        let existing: Option<Value> = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            Some(serde_json::from_str(&text).with_context(|| {
                format!("{} is not valid JSON; nothing was written", path.display())
            })?)
        } else {
            None
        };
        let merged = merge_hooks(existing.as_ref(), bin, board.as_deref())
            .map_err(|e| anyhow::anyhow!("{}: {e}; nothing was written", path.display()))?;
        plans.push((label, path, existing, merged));
    }
    for (label, path, existing, merged) in plans {
        if existing.as_ref() == Some(&merged) {
            writeln!(out, "{} ({label}): already up to date", path.display())?;
            continue;
        }
        let old = existing
            .as_ref()
            .map(serde_json::to_string_pretty)
            .transpose()?
            .unwrap_or_default();
        let new = serde_json::to_string_pretty(&merged)?;
        writeln!(out, "{} ({label}):", path.display())?;
        write!(out, "{}", diff(&old, &new))?;
        if !options.yes && !ask(input, out, &format!("Write {}?", path.display()), false)? {
            writeln!(out, "skipped {}", path.display())?;
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.klatch-tmp");
        std::fs::write(&tmp, format!("{new}\n"))?;
        std::fs::rename(&tmp, &path)?;
        writeln!(out, "wrote {}", path.display())?;
    }
    Ok(())
}

/// The Claude Code hook events klatch listens on.
pub const EVENTS: [&str; 3] = ["SessionStart", "UserPromptSubmit", "PostToolUse"];

/// The hook command for one event: the binary by absolute path, never a shell variable (permission rules refuse
/// those, #174). `board` adds the session-start board for that room (only on SessionStart; the consent to read a
/// room before joining it, DESIGN section 25.6).
pub fn hook_command(bin: &str, event: &str, board: Option<&str>) -> Result<String, String> {
    if !bin.starts_with('/') {
        return Err(format!("the binary path must be absolute: {bin}"));
    }
    if bin
        .chars()
        .any(|c| matches!(c, '$' | '`' | '\'' | '"' | '\\') || c.is_control())
    {
        return Err(format!(
            "the binary path has a character a hook command cannot carry safely (a shell variable, quote, backslash or control character): {bin}"
        ));
    }
    if event.is_empty() || !event.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("bad hook event name: {event}"));
    }
    let plain = bin
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_.-+:@%,=~".contains(c));
    let path = if plain {
        bin.to_string()
    } else {
        format!("'{bin}'")
    };
    let mut cmd = format!("{path} --discover hook --event {event}");
    if let Some(room) = board {
        if !klatch_core::valid_name(room) {
            return Err(format!("bad board room name: {room:?}"));
        }
        if event == "SessionStart" {
            cmd.push_str(&format!(" --board {room}"));
        }
    }
    Ok(cmd)
}

/// A hook command klatch wrote (or a hand-written one of the same form).
fn is_klatch_command(cmd: &str) -> bool {
    cmd.contains(" hook --event ") && cmd.contains("klatch")
}

/// `settings` with klatch's hook entries set to exactly what is asked: added when missing, updated in place when
/// present, everything else left as it was. Fails (writing nothing) when the file is not a JSON object or its
/// `hooks` is not an object.
pub fn merge_hooks(
    existing: Option<&Value>,
    bin: &str,
    board: Option<&str>,
) -> Result<Value, String> {
    let mut settings = existing.cloned().unwrap_or_else(|| json!({}));
    let root = settings
        .as_object_mut()
        .ok_or("the settings file is not a JSON object")?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or("\"hooks\" in the settings file is not an object")?;
    for event in EVENTS {
        let cmd = hook_command(bin, event, board)?;
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let list = list
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} in the settings file is not a list"))?;
        let mut found = false;
        for matcher in list.iter_mut() {
            let Some(inner) = matcher.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            inner.retain_mut(|h| {
                let is_ours = h
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_klatch_command);
                if !is_ours {
                    return true;
                }
                if found {
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
        if !found {
            list.push(json!({"hooks": [{"type": "command", "command": cmd}]}));
        }
    }
    Ok(settings)
}

/// A line diff of two texts: removed lines with `-`, added with `+`, unchanged lines with two spaces.
pub fn diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push_str(&format!("  {}\n", a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            out.push_str(&format!("+ {}\n", b[j]));
            j += 1;
        } else {
            out.push_str(&format!("- {}\n", a[i]));
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
        let c = hook_command(BIN, "PostToolUse", None).unwrap();
        assert_eq!(c, "/opt/klatch/klatch --discover hook --event PostToolUse");
    }

    #[test]
    fn the_board_flag_is_added_only_for_session_start() {
        let s = hook_command(BIN, "SessionStart", Some("lobby")).unwrap();
        assert!(s.ends_with("--event SessionStart --board lobby"), "{s}");
        let p = hook_command(BIN, "PostToolUse", Some("lobby")).unwrap();
        assert!(!p.contains("--board"), "{p}");
    }

    #[test]
    fn a_path_with_spaces_is_quoted_and_a_dangerous_path_is_refused() {
        let c = hook_command("/Users/a b/klatch", "PostToolUse", None).unwrap();
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
                hook_command(bad, "PostToolUse", None).is_err(),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn a_bad_board_room_name_is_refused() {
        assert!(hook_command(BIN, "SessionStart", Some("a b")).is_err());
        assert!(hook_command(BIN, "SessionStart", Some("")).is_err());
        assert!(hook_command(BIN, "SessionStart", Some("x;rm")).is_err());
    }

    #[test]
    fn merging_into_nothing_adds_the_three_events() {
        let v = merge_hooks(None, BIN, None).unwrap();
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
        let v = merge_hooks(Some(&before), BIN, None).unwrap();
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
        let once = merge_hooks(None, BIN, Some("lobby")).unwrap();
        let twice = merge_hooks(Some(&once), BIN, Some("lobby")).unwrap();
        assert_eq!(once, twice);
        assert_eq!(commands(&twice, "PostToolUse").len(), 1);
    }

    #[test]
    fn rerunning_with_other_answers_updates_the_entry_in_place() {
        let with = merge_hooks(None, BIN, Some("lobby")).unwrap();
        let without = merge_hooks(Some(&with), BIN, None).unwrap();
        assert_eq!(
            commands(&without, "SessionStart").len(),
            1,
            "added a duplicate"
        );
        assert!(!commands(&without, "SessionStart")[0].contains("--board"));
        let moved = merge_hooks(Some(&with), "/other/klatch", Some("lobby")).unwrap();
        assert_eq!(commands(&moved, "PostToolUse").len(), 1);
        assert!(commands(&moved, "PostToolUse")[0].starts_with("/other/klatch"));
    }

    #[test]
    fn a_settings_file_of_the_wrong_shape_is_refused() {
        assert!(merge_hooks(Some(&json!([1, 2])), BIN, None).is_err());
        assert!(merge_hooks(Some(&json!({"hooks": "no"})), BIN, None).is_err());
        assert!(merge_hooks(Some(&json!({"hooks": {"PostToolUse": 3}})), BIN, None).is_err());
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
}
