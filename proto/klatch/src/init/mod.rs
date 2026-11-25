//! `klatch init`: writes the Claude Code hook entries that call this binary, with consent per scope.
//! The pure parts (building the hook command, merging into a settings file, showing a diff) live here so they are
//! testable; the prompting and file writing are in `run`.

use anyhow::{Context, bail};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

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
    /// Do not join new sessions to the default lobby at session start.
    pub no_lobby: bool,
    /// Do not install the SessionEnd hook that marks a finished session done.
    pub no_session_end: bool,
    /// The PreToolUse claim hook (#33) and the wake hook (#37): opt in, opt out, or leave as is.
    pub enforce: Switch,
    pub wake: Switch,
    /// The machine checkpoint (#1408): the Stop hook, and a flag on the PostToolUse and SessionEnd hooks that records
    /// tool classes and edited paths and writes the checkpoint. Opt in, opt out, or leave as is.
    pub checkpoint: Switch,
    /// Passive claims (#1401): a flag on the PostToolUse, SessionEnd and Stop hooks that claims the path of each file a
    /// member edits and releases it at the end of the turn. Opt in, opt out, or leave as is.
    pub passive_claims: Switch,
    /// The machine status (#1403): the same flag on the PostToolUse and SessionEnd hooks and the shared Stop hook, which
    /// set `working (auto)`, `idle (auto)` or `parked (auto)` from activity. Opt in, opt out, or leave as is.
    pub machine_status: Switch,
    /// The stop warning (#1344): a flag on the PostToolUse and SessionEnd hooks and the shared Stop hook, which prints one
    /// warn-only line when a turn ends with background tasks or crons still running. Opt in, opt out, or leave as is.
    pub stop_warn: Switch,
    /// Remove every hook klatch wrote instead of installing (#436); nothing else in the settings is touched.
    pub uninstall: bool,
    /// Install into Codex's hooks.json instead of Claude Code's settings (#782).
    pub codex: bool,
}

/// Where `init` reads answers from and writes its questions and results to.
pub struct Io<'a> {
    pub input: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
}

/// The places `init` works with: the project it was run in, the home directory (if known) and this binary.
pub struct Places<'a> {
    pub project_dir: &'a Path,
    pub home: Option<&'a Path>,
    pub bin: &'a Path,
    /// Where Codex keeps its files (`CODEX_HOME`, else `.codex` under the home), for `init --codex`.
    pub codex_home: Option<&'a Path>,
}

/// What the user decided, by flag or by answering.
struct Choices {
    scope: Scope,
    board: Option<String>,
    join: bool,
    session_end: bool,
    enforce: Switch,
    wake: Switch,
    checkpoint: Switch,
    passive_claims: Switch,
    machine_status: Switch,
    stop_warn: Switch,
}

/// A settings file and what it would become.
pub(crate) struct Plan {
    pub(crate) label: &'static str,
    pub(crate) path: PathBuf,
    pub(crate) existing: Option<Value>,
    pub(crate) merged: Value,
}

impl Io<'_> {
    fn read_line(&mut self) -> anyhow::Result<String> {
        let mut line = String::new();
        self.input.read_line(&mut line)?;
        Ok(line.trim().to_ascii_lowercase())
    }

    /// Ask a yes or no question; an empty answer, end of input and anything but yes mean `default`.
    pub(crate) fn ask(&mut self, question: &str, default_yes: bool) -> anyhow::Result<bool> {
        write!(
            self.out,
            "{question} [{}] ",
            if default_yes { "Y/n" } else { "y/N" }
        )?;
        self.out.flush()?;
        Ok(match self.read_line()?.as_str() {
            "" => default_yes,
            "y" | "yes" => true,
            _ => false,
        })
    }

    /// A setting that is on unless switched off by a flag, and is not asked about under `--yes`.
    pub(crate) fn on_unless(
        &mut self,
        off: bool,
        yes: bool,
        question: &str,
    ) -> anyhow::Result<bool> {
        if off {
            return Ok(false);
        }
        if yes {
            return Ok(true);
        }
        self.ask(question, true)
    }

    fn choose_scope(&mut self, options: &Options) -> anyhow::Result<Scope> {
        if let Some(scope) = options.scope {
            return Ok(scope);
        }
        if options.yes {
            bail!("init --yes asks nothing, so it needs --scope project, user or both");
        }
        write!(
            self.out,
            "Where should klatch install its hooks? project (.claude/settings.local.json here), user (~/.claude/settings.json) or both [project]: "
        )?;
        self.out.flush()?;
        match self.read_line()?.as_str() {
            "" | "project" => Ok(Scope::Project),
            "user" => Ok(Scope::User),
            "both" => Ok(Scope::Both),
            other => bail!("unknown scope {other:?}: say project, user or both"),
        }
    }

    pub(crate) fn choose_board(&mut self, options: &Options) -> anyhow::Result<Option<String>> {
        if let Some(room) = &options.board {
            return Ok(Some(room.clone()));
        }
        if options.yes {
            return Ok(None);
        }
        let show = self.ask(
            "Show the lobby board when a session starts? This reads the lobby before the session has joined it.",
            false,
        )?;
        Ok(show.then(|| "lobby".to_string()))
    }

    fn choose(&mut self, options: &Options) -> anyhow::Result<Choices> {
        let scope = self.choose_scope(options)?;
        let board = self.choose_board(options)?;
        // Joining the default lobby at session start is on: installing the hooks is the consent (DESIGN section 24,
        // 25.10).
        let join = self.on_unless(
            options.no_lobby,
            options.yes,
            "Join every new session to the default lobby automatically?",
        )?;
        // The SessionEnd hook is on by default: it only marks the session done in klatch's own store.
        let session_end = self.on_unless(
            options.no_session_end,
            options.yes,
            "Mark a session done when it ends, so it leaves the lobby board at once?",
        )?;
        if options.checkpoint == Switch::On && !session_end {
            bail!("--checkpoint needs the SessionEnd hook, which --no-session-end leaves out");
        }
        if options.machine_status == Switch::On && !session_end {
            bail!(
                "--machine-status needs the SessionEnd hook (it marks the session done), which --no-session-end leaves out"
            );
        }
        Ok(Choices {
            scope,
            board,
            join,
            session_end,
            enforce: options.enforce,
            wake: options.wake,
            checkpoint: options.checkpoint,
            passive_claims: options.passive_claims,
            machine_status: options.machine_status,
            stop_warn: options.stop_warn,
        })
    }
}

/// The settings files a scope writes to, with a label for each.
pub(crate) fn targets(
    scope: Scope,
    places: &Places,
) -> anyhow::Result<Vec<(&'static str, PathBuf)>> {
    let mut targets = Vec::new();
    if matches!(scope, Scope::Project | Scope::Both) {
        targets.push((
            "project",
            places
                .project_dir
                .join(".claude")
                .join("settings.local.json"),
        ));
    }
    if matches!(scope, Scope::User | Scope::Both) {
        let home = places
            .home
            .context("HOME is not set, so the user settings file cannot be found")?;
        targets.push(("user", home.join(".claude").join("settings.json")));
    }
    Ok(targets)
}

/// The current content of a settings file, if there is one.
pub(crate) fn read_settings(path: &Path) -> anyhow::Result<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON; nothing was written", path.display()))?;
    Ok(Some(value))
}

/// Read and merge one settings file; any failure stops the whole run before anything is written.
fn plan(label: &'static str, path: PathBuf, bin: &str, choices: &Choices) -> anyhow::Result<Plan> {
    let existing = read_settings(&path)?;
    let refuse = |e: String| anyhow::anyhow!("{}: {e}; nothing was written", path.display());
    let mut merged = merge_hooks(
        existing.as_ref(),
        bin,
        choices.board.as_deref(),
        choices.join,
    )
    .map_err(refuse)?;
    set_session_end(&mut merged, bin, choices.session_end).map_err(refuse)?;
    set_enforce(&mut merged, bin, choices.enforce).map_err(refuse)?;
    set_wake(&mut merged, bin, choices.wake).map_err(refuse)?;
    set_checkpoint(&mut merged, bin, choices.checkpoint).map_err(refuse)?;
    set_passive_claims(&mut merged, bin, choices.passive_claims).map_err(refuse)?;
    set_machine_status(&mut merged, bin, choices.machine_status).map_err(refuse)?;
    set_stop_warn(&mut merged, bin, choices.stop_warn).map_err(refuse)?;
    Ok(Plan {
        label,
        path,
        existing,
        merged,
    })
}

/// Show the diff for one plan and write it, unless it is already in place or the user says no.
pub(crate) fn apply(plan: Plan, yes: bool, io: &mut Io) -> anyhow::Result<()> {
    let Plan {
        label,
        path,
        existing,
        merged,
    } = plan;
    if existing.as_ref() == Some(&merged) {
        writeln!(io.out, "{} ({label}): already up to date", path.display())?;
        return Ok(());
    }
    let old = existing
        .as_ref()
        .map(serde_json::to_string_pretty)
        .transpose()?
        .unwrap_or_default();
    let new = serde_json::to_string_pretty(&merged)?;
    writeln!(io.out, "{} ({label}):", path.display())?;
    write!(io.out, "{}", diff(&old, &new))?;
    if !yes && !io.ask(&format!("Write {}?", path.display()), false)? {
        writeln!(io.out, "skipped {}", path.display())?;
        return Ok(());
    }
    write_settings(&path, &new, io)
}

/// Write a settings file whole or not at all: beside it, then a rename.
pub(crate) fn write_settings(path: &Path, text: &str, io: &mut Io) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.klatch-tmp");
    std::fs::write(&tmp, format!("{text}\n"))?;
    std::fs::rename(&tmp, path)?;
    writeln!(io.out, "wrote {}", path.display())?;
    Ok(())
}

/// What `init --uninstall` would do to one settings file.
struct Removal {
    label: &'static str,
    path: PathBuf,
    /// The file as it is, and as it would be; `None` when there is no file or no klatch hook in it.
    change: Option<(Value, Value)>,
}

/// Remove every klatch hook from a settings value. Every other hook, and every other setting, stays as it was; an
/// event klatch emptied is dropped, and so is `hooks` itself if klatch emptied it.
pub fn remove_hooks(settings: &mut Value) -> Result<(), String> {
    let Some(hooks) = settings.get_mut("hooks") else {
        return Ok(());
    };
    let map = hooks.as_object_mut().ok_or("\"hooks\" is not an object")?;
    let emptied: Vec<String> = map
        .iter_mut()
        .filter_map(|(event, list)| {
            let list = list.as_array_mut()?;
            strip_klatch(list).then(|| event.clone())
        })
        .collect();
    for event in &emptied {
        map.remove(event);
    }
    let klatch_emptied_hooks = !emptied.is_empty() && map.is_empty();
    if klatch_emptied_hooks && let Some(obj) = settings.as_object_mut() {
        obj.remove("hooks");
    }
    Ok(())
}

/// Take klatch's entries out of an event's list; whether that left a list that had entries with none.
fn strip_klatch(list: &mut Vec<Value>) -> bool {
    let had_entries = !list.is_empty();
    sync_owned(list, "", false, is_ours);
    sync_owned(list, "", false, is_wake);
    had_entries && list.is_empty()
}

fn plan_removal(label: &'static str, path: PathBuf) -> anyhow::Result<Removal> {
    let existing = read_settings(&path)?;
    let change = match existing {
        None => None,
        Some(old) => {
            let mut new = old.clone();
            remove_hooks(&mut new)
                .map_err(|e| anyhow::anyhow!("{}: {e}; nothing was written", path.display()))?;
            (new != old).then_some((old, new))
        }
    };
    Ok(Removal {
        label,
        path,
        change,
    })
}

/// `init --uninstall`: ask the scope, read every file first, then show each diff and remove on a yes.
fn uninstall(options: &Options, io: &mut Io, places: &Places) -> anyhow::Result<()> {
    let scope = io.choose_scope(options)?;
    remove_from(options.yes, io, targets(scope, places)?)
}

/// Read every file first, then show each diff and remove klatch's hooks on a yes (or `yes`).
pub(crate) fn remove_from(
    yes: bool,
    io: &mut Io,
    files: Vec<(&'static str, PathBuf)>,
) -> anyhow::Result<()> {
    let removals = files
        .into_iter()
        .map(|(label, path)| plan_removal(label, path))
        .collect::<anyhow::Result<Vec<_>>>()?;
    for Removal {
        label,
        path,
        change,
    } in removals
    {
        let Some((old, new)) = change else {
            writeln!(io.out, "{} ({label}): nothing to remove", path.display())?;
            continue;
        };
        writeln!(io.out, "{} ({label}):", path.display())?;
        let (old, new) = (
            serde_json::to_string_pretty(&old)?,
            serde_json::to_string_pretty(&new)?,
        );
        write!(io.out, "{}", diff(&old, &new))?;
        if !yes
            && !io.ask(
                &format!("Remove klatch's hooks from {}?", path.display()),
                false,
            )?
        {
            writeln!(io.out, "skipped {}", path.display())?;
            continue;
        }
        write_settings(&path, &new, io)?;
    }
    writeln!(
        io.out,
        "Your rooms, messages and keys are not touched; see UNINSTALL.md to remove them."
    )?;
    Ok(())
}

/// Run `init`: choose scope and board (asking unless `--yes`), show each diff, write only on a yes.
pub fn run(options: &Options, io: &mut Io, places: &Places) -> anyhow::Result<()> {
    if options.codex {
        return crate::init_codex::run(options, io, places);
    }
    if options.uninstall {
        return uninstall(options, io, places);
    }
    let bin = places
        .bin
        .to_str()
        .context("the path of this binary is not valid text")?;
    let choices = io.choose(options)?;
    // Read and merge everything first: a file that cannot be read stops the whole run before anything is written.
    let plans = targets(choices.scope, places)?
        .into_iter()
        .map(|(label, path)| plan(label, path, bin, &choices))
        .collect::<anyhow::Result<Vec<_>>>()?;
    for plan in plans {
        apply(plan, options.yes, io)?;
    }
    write!(
        io.out,
        "{}",
        next_steps(std::env::var_os("KLATCH_HOME").as_deref(), places.home)
    )?;
    Ok(())
}

/// What `init` ends with (DESIGN section 21): where klatch keeps its data, shown as data and never created or
/// changed here, and the check to run next. A set `KLATCH_HOME` wins over `~/.klatch`, as it does for the store.
pub(crate) fn next_steps(klatch_home: Option<&std::ffi::OsStr>, home: Option<&Path>) -> String {
    let data = match (klatch_home, home) {
        (Some(dir), _) => format!("{}", Path::new(dir).display()),
        (None, Some(home)) => format!("{}", home.join(".klatch").display()),
        (None, None) => "~/.klatch".to_string(),
    };
    format!(
        "klatch keeps its data in {data} (set KLATCH_HOME to move it; init neither creates nor changes it).\nNext: run `klatch doctor` to check the install; it only reads, and it warns while no admin passphrase is set.\nSet that passphrase yourself, in a terminal: `klatch admin init-passphrase` (init never sets it).\n"
    )
}

mod diff;
mod flags;
mod hooks;
mod optional;
#[cfg(test)]
mod tests;
pub use diff::*;
pub use flags::*;
pub use hooks::*;
pub use optional::*;
