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
        Ok(Choices {
            scope,
            board,
            join,
            session_end,
            enforce: options.enforce,
            wake: options.wake,
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
        "klatch keeps its data in {data} (set KLATCH_HOME to move it; init neither creates nor changes it).\nNext: run `klatch doctor` to check the install; it only reads.\n"
    )
}

/// The Claude Code hook events klatch listens on.
pub const EVENTS: [&str; 3] = ["SessionStart", "UserPromptSubmit", "PostToolUse"];

/// A binary path a hook command can carry: absolute and free of anything a shell would act on.
fn check_bin_path(bin: &str) -> Result<(), String> {
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

fn check_event_name(event: &str) -> Result<(), String> {
    if event.is_empty() || !event.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("bad hook event name: {event}"));
    }
    Ok(())
}

/// The binary path as it goes into a command: plain when it needs no quoting, else in single quotes.
fn shell_path(bin: &str) -> String {
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
    cmd.contains(" hook --event ") && cmd.contains("klatch")
}

fn is_ours(hook: &Value) -> bool {
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
fn klatch_hooks_in<'a>(
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
fn sync_entries(list: &mut Vec<Value>, cmd: &str, keep: bool) -> bool {
    sync_owned(list, cmd, keep, is_ours)
}

/// `sync_entries` for the entries `owns` says are klatch's.
fn sync_owned(list: &mut Vec<Value>, cmd: &str, keep: bool, owns: fn(&Value) -> bool) -> bool {
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
fn hooks_of(settings: &mut Value) -> Result<&mut serde_json::Map<String, Value>, String> {
    let root = settings
        .as_object_mut()
        .ok_or("the settings file is not a JSON object")?;
    root.entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "\"hooks\" in the settings file is not an object".to_string())
}

/// The list of matchers for one event, created if missing.
fn event_list<'a>(
    hooks: &'a mut serde_json::Map<String, Value>,
    event: &str,
) -> Result<&'a mut Vec<Value>, String> {
    hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| format!("hooks.{event} in the settings file is not a list"))
}

fn new_entry(cmd: &str) -> Value {
    json!({"hooks": [{"type": "command", "command": cmd}]})
}

/// The timeout, in seconds, written into every ordinary hook entry `init` installs (#1246). The first read by a new
/// binary may wait on the Keychain for up to `klatch::keys::KEYCHAIN_READ_LIMIT_SECS` before it fails with a message
/// that says what to do; the Claude Code default was not verified, so the entry says it outright, with room to spare.
const HOOK_TIMEOUT_SECS: u64 = 30;

/// Give every hook of klatch's in `list` a timeout of at least `HOOK_TIMEOUT_SECS`: add it where it is missing,
/// raise a lower one, keep a longer one the user chose.
fn ensure_timeout(list: &mut [Value], owns: fn(&Value) -> bool) {
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
        let cmd = command(event)?;
        let list = event_list(hooks, event)?;
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
    let cmd = hook_command(bin, EVENT, None, true)?;
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
    let list = event_list(hooks, EVENT)?;
    if !sync_entries(list, &cmd, on) && on {
        list.push(new_entry(&cmd));
    }
    ensure_timeout(list, is_ours);
    if list.is_empty() {
        hooks.remove(EVENT);
    }
    Ok(())
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

/// One optional hook klatch can install.
struct Optional {
    event: &'static str,
    entry: fn(&str) -> Result<Value, String>,
    owns: fn(&Value) -> bool,
    /// Replace an existing entry whole, not only its command.
    refresh: bool,
}

/// The tools the claim hook looks at.
const ENFORCE_MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit|Bash";

/// How long the wake watcher may run, and the timeout Claude Code gets for it (a little more).
const WAKE_WATCH_SECS: u64 = 3500;
const WAKE_HOOK_TIMEOUT_SECS: u64 = 3600;

fn is_wake_command(cmd: &str) -> bool {
    cmd.contains("klatch") && cmd.contains(" --discover wake")
}

fn is_wake(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(is_wake_command)
}

fn enforce_entry(bin: &str) -> Result<Value, String> {
    let cmd = hook_command(bin, "PreToolUse", None, true)?;
    Ok(json!({"matcher": ENFORCE_MATCHER, "hooks": [{"type": "command", "command": cmd, "timeout": HOOK_TIMEOUT_SECS}]}))
}

fn wake_entry(bin: &str) -> Result<Value, String> {
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

fn set_optional(
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
    if !sync_owned(list, &cmd, how != Switch::Off, spec.owns) && how == Switch::On {
        list.push(entry);
    } else if how != Switch::Off && spec.refresh {
        refresh_owned(list, spec.owns, &entry["hooks"][0]);
    } else if how != Switch::Off && spec.event == "PreToolUse" {
        ensure_timeout(list, is_ours);
    }
    if list.is_empty() {
        hooks.remove(spec.event);
    }
    Ok(())
}

/// Whether a settings object has no entry for the event (a settings value that is not an object counts as present).
fn event_absent(settings: &Value, event: &str) -> bool {
    let has_event =
        |r: &serde_json::Map<String, Value>| r.get("hooks").is_some_and(|h| h.get(event).is_some());
    settings.as_object().is_some_and(|r| !has_event(r))
}

/// An entry written by an older klatch: bring every field of the hooks `owns` claims up to date, not just the command.
fn refresh_owned(list: &mut [Value], owns: fn(&Value) -> bool, fresh: &Value) {
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

/// The longest-common-subsequence table of two line lists: `table[i][j]` is the length for `a[i..]` and `b[j..]`.
fn lcs_table(a: &[&str], b: &[&str]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

/// What the next line of a diff is.
enum Step {
    Same,
    Add,
    Remove,
}

/// The next step through two line lists at positions `i` and `j`, following the longest common subsequence.
fn next_step(a: &[&str], b: &[&str], lcs: &[Vec<usize>], (i, j): (usize, usize)) -> Step {
    match (a.get(i), b.get(j)) {
        (Some(x), Some(y)) if x == y => Step::Same,
        (Some(_), None) => Step::Remove,
        (None, _) => Step::Add,
        (Some(_), Some(_)) if lcs[i][j + 1] >= lcs[i + 1][j] => Step::Add,
        (Some(_), Some(_)) => Step::Remove,
    }
}

/// A line diff of two texts: removed lines with `-`, added with `+`, unchanged lines with two spaces.
pub fn diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let lcs = lcs_table(&a, &b);
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < a.len() || j < b.len() {
        match next_step(&a, &b, &lcs, (i, j)) {
            Step::Same => {
                out.push_str(&format!("  {}\n", a[i]));
                i += 1;
                j += 1;
            }
            Step::Add => {
                out.push_str(&format!("+ {}\n", b[j]));
                j += 1;
            }
            Step::Remove => {
                out.push_str(&format!("- {}\n", a[i]));
                i += 1;
            }
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
            for hook in list.as_array().unwrap().iter().flat_map(|m| m["hooks"].as_array().unwrap()) {
                let t = hook["timeout"].as_u64().unwrap_or_else(|| panic!("{event} has no timeout"));
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
        assert_eq!(v["hooks"]["SessionStart"][0]["hooks"][0]["timeout"], HOOK_TIMEOUT_SECS);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["timeout"], 120);
        assert_eq!(v["hooks"]["PostToolUse"][0]["hooks"][0]["timeout"], HOOK_TIMEOUT_SECS);
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
        let mut other = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/usr/bin/mine"}]}]}});
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
}
