//! Whether Codex trusts klatch's hooks, read from the Codex files only (#544, #590; RESEARCH-NOTES 49.3 and 49.7).
//!
//! What was verified against Codex itself (0.147.0 and 0.169.0, `codex app-server`, `hooks/list`):
//! - a hook's trust key is `<file>:<event_snake>:<group>:<handler>`, the group being the index of the matcher group
//!   in the event's array and the handler its index in that group's `hooks`, for `hooks.json` and for a hook written
//!   inline in `config.toml` (`[[hooks.Stop]]` with `[[hooks.Stop.hooks]]`; the file is then `config.toml`);
//! - the recorded `trusted_hash` is the SHA-256 of the compact JSON, keys sorted, of
//!   `{"event_name", "hooks": [{"async", "command", "statusMessage" when set, "timeout", "type"}], "matcher" when
//!   set}`; `timeout` defaults to 600 and `async` to false; unknown fields do not count. Codex reports a hook whose
//!   record has another hash as modified (skipped until reviewed again);
//! - Codex skips an `async` hook ("async hooks are not supported yet").
//!
//! The hash is recomputed here only to compare with the record. It is never written: klatch never writes a trust
//! record. Where Codex keys a project `.codex/hooks.json` hook is not verified, so that layer is reported unchecked.

use super::{Finding, finding};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// One handler of a matcher group, as far as the trust hash and the skip rules look at it.
#[derive(Default)]
struct Handler {
    command: String,
    timeout: u64,
    is_async: bool,
    status: Option<String>,
}

/// A matcher group: its optional matcher and its handlers, in file order.
#[derive(Default)]
struct Group {
    matcher: Option<String>,
    handlers: Vec<Handler>,
}

/// Every event of a hooks source with its matcher groups, in file order.
type Events = Vec<(String, Vec<Group>)>;

/// A hook event as Codex spells it in a trust record: `PreToolUse` is `pre_tool_use`.
pub(super) fn snake(event: &str) -> String {
    let mut out = String::new();
    for (i, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn handler_of(hook: &Value) -> Handler {
    Handler {
        command: hook
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        timeout: hook.get("timeout").and_then(Value::as_u64).unwrap_or(600),
        is_async: hook.get("async").and_then(Value::as_bool) == Some(true),
        status: hook
            .get("statusMessage")
            .and_then(Value::as_str)
            .map(String::from),
    }
}

fn group_of(group: &Value) -> Group {
    Group {
        matcher: group
            .get("matcher")
            .and_then(Value::as_str)
            .map(String::from),
        handlers: group
            .get("hooks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(handler_of)
            .collect(),
    }
}

/// The events of a parsed `hooks.json`.
fn events_of_json(value: &Value) -> Events {
    let Some(events) = value.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    events
        .iter()
        .map(|(event, groups)| {
            let groups = groups.as_array().into_iter().flatten().map(group_of);
            (event.clone(), groups.collect())
        })
        .collect()
}

/// The text of a TOML basic string or the digits of an integer or a boolean, whichever `raw` is.
fn toml_value(raw: &str) -> String {
    let raw = raw.trim();
    match raw.strip_prefix('"') {
        Some(rest) => {
            let mut out = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => out.extend(chars.next()),
                    c => out.push(c),
                }
            }
            out
        }
        None => raw.split('#').next().unwrap_or("").trim().to_string(),
    }
}

/// Apply one `key = value` line to the handler or group being read.
fn apply_toml_line(events: &mut Events, in_handler: bool, key: &str, value: &str) {
    let Some(group) = events.last_mut().and_then(|(_, g)| g.last_mut()) else {
        return;
    };
    let value = toml_value(value);
    if !in_handler {
        if key == "matcher" {
            group.matcher = Some(value);
        }
        return;
    }
    let Some(h) = group.handlers.last_mut() else {
        return;
    };
    match key {
        "command" => h.command = value,
        "timeout" => h.timeout = value.parse().unwrap_or(600),
        "async" => h.is_async = value == "true",
        "statusMessage" => h.status = Some(value),
        _ => {}
    }
}

/// The events written inline in a Codex `config.toml`: the array-of-tables form `[[hooks.<Event>]]` with
/// `[[hooks.<Event>.hooks]]` under it, read by line. Other spellings (inline tables) are not read.
fn events_of_toml(config: &str) -> Events {
    let mut reader = TomlHooks::default();
    for line in config.lines().map(str::trim) {
        reader.line(line);
    }
    reader.events
}

/// Reads the array-of-tables hook form one line at a time.
#[derive(Default)]
struct TomlHooks {
    events: Events,
    /// The last header read was a `.hooks` handler table, not an event group.
    in_handler: bool,
    /// The lines being read belong to a hook table.
    reading: bool,
}

impl TomlHooks {
    fn line(&mut self, line: &str) {
        if let Some(name) = line
            .strip_prefix("[[hooks.")
            .and_then(|r| r.strip_suffix("]]"))
        {
            self.header(name);
        } else if line.starts_with('[') {
            self.reading = false;
        } else if let Some((key, value)) = line.split_once('=').filter(|_| self.reading) {
            apply_toml_line(&mut self.events, self.in_handler, key.trim(), value);
        }
    }

    /// `name` is `<Event>` for a matcher group and `<Event>.hooks` for a handler of the last group.
    fn header(&mut self, name: &str) {
        self.reading = true;
        match name.strip_suffix(".hooks") {
            Some(event) => self.handler_header(event),
            None => {
                self.in_handler = false;
                self.events.push((name.to_string(), vec![Group::default()]));
            }
        }
    }

    fn handler_header(&mut self, event: &str) {
        let group = self
            .events
            .last_mut()
            .filter(|e| e.0 == event)
            .and_then(|e| e.1.last_mut());
        match group {
            Some(g) => {
                self.in_handler = true;
                g.handlers.push(Handler {
                    timeout: 600,
                    ..Handler::default()
                });
            }
            None => self.reading = false,
        }
    }
}

/// The hash Codex records for a hook (see the module notes); `None` for a hook that is not a command.
fn trust_hash(event: &str, group: &Group, h: &Handler) -> String {
    let mut handler = Map::new();
    handler.insert("async".into(), Value::Bool(h.is_async));
    handler.insert("command".into(), Value::String(h.command.clone()));
    if let Some(s) = &h.status {
        handler.insert("statusMessage".into(), Value::String(s.clone()));
    }
    handler.insert("timeout".into(), Value::from(h.timeout));
    handler.insert("type".into(), Value::String("command".into()));
    let mut identity: BTreeMap<&str, Value> = BTreeMap::new();
    identity.insert("event_name", Value::String(snake(event)));
    identity.insert("hooks", Value::Array(vec![Value::Object(handler)]));
    if let Some(m) = &group.matcher {
        identity.insert("matcher", Value::String(m.clone()));
    }
    let json = serde_json::to_string(&identity).unwrap_or_default();
    format!(
        "sha256:{}",
        data_encoding::HEXLOWER.encode(&Sha256::digest(json.as_bytes()))
    )
}

/// What Codex does with one klatch hook.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trust {
    Trusted,
    Untrusted,
    Changed,
    Async,
}

/// The `[hooks.state."<key>"]` sections of a Codex config with their `trusted_hash`. A section with no hash is not
/// trust. Plain line scanning, because the form is fixed and klatch carries no TOML parser for one table.
fn trusted_hashes(config: &str) -> HashMap<String, String> {
    let mut trusted = HashMap::new();
    let mut section: Option<String> = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            section = state_section(line);
        } else if let Some((key, hash)) = section.as_ref().zip(recorded_hash(line)) {
            trusted.insert(key.clone(), hash);
        }
    }
    trusted
}

/// The key of a `[hooks.state."<key>"]` header, or none for any other header.
fn state_section(header: &str) -> Option<String> {
    header
        .strip_prefix("[hooks.state.\"")
        .and_then(|r| r.strip_suffix("\"]"))
        .map(String::from)
}

/// The value of a `trusted_hash = "sha256:..."` line; none for any other line or a value that is no hash.
fn recorded_hash(line: &str) -> Option<String> {
    let value = line.strip_prefix("trusted_hash")?;
    let hash = toml_value(value.trim_start().trim_start_matches('='));
    hash.starts_with("sha256:").then_some(hash)
}

/// The event and trust of each klatch hook of one hooks source `file`.
fn judge(file: &Path, events: &Events, trusted: &HashMap<String, String>) -> Vec<(String, Trust)> {
    slots(events)
        .filter(|(.., h)| crate::init::is_klatch_command(&h.command))
        .map(|(event, n, m, group, h)| {
            let key = format!("{}:{}:{n}:{m}", file.display(), snake(event));
            (
                event.to_string(),
                trust_of(trusted.get(&key), (event, group, h)),
            )
        })
        .collect()
}

/// Every handler with its event, group index and handler index (the numbers of Codex's trust key).
fn slots(events: &Events) -> impl Iterator<Item = (&str, usize, usize, &Group, &Handler)> {
    events.iter().flat_map(|(event, groups)| {
        groups.iter().enumerate().flat_map(move |(n, group)| {
            let handlers = group.handlers.iter().enumerate();
            handlers.map(move |(m, h)| (event.as_str(), n, m, group, h))
        })
    })
}

/// What Codex does with a hook whose trust record (the `trusted_hash`, if any) is `record`.
fn trust_of(record: Option<&String>, (event, group, h): (&str, &Group, &Handler)) -> Trust {
    match record {
        _ if h.is_async => Trust::Async,
        None => Trust::Untrusted,
        Some(hash) if *hash == trust_hash(event, group, h) => Trust::Trusted,
        Some(_) => Trust::Changed,
    }
}

/// "SessionStart, Stop" for the events whose hooks have `which`.
fn events_with(judged: &[(String, Trust)], which: Trust) -> Option<String> {
    let mut events: Vec<&str> = judged
        .iter()
        .filter(|(_, t)| *t == which)
        .map(|(e, _)| e.as_str())
        .collect();
    events.dedup();
    (!events.is_empty()).then(|| events.join(", "))
}

const FIX: &str = "open Codex and review the hooks in /hooks to trust them; klatch never passes --dangerously-bypass-hook-trust and never writes the trust record itself";

/// The finding for one hooks source: ok when every klatch hook is trusted at its current definition, else what
/// Codex will skip and why.
fn source_finding(file: &Path, config: &Path, judged: &[(String, Trust)]) -> Finding {
    let problems: Vec<String> = [
        (Trust::Untrusted, "not trusted by Codex"),
        (
            Trust::Changed,
            "changed since they were trusted (the record holds another hash)",
        ),
        (
            Trust::Async,
            "async, which Codex skips (async hooks are not supported yet)",
        ),
    ]
    .iter()
    .filter_map(|(t, why)| events_with(judged, *t).map(|e| format!("{why}: {e}")))
    .collect();
    if problems.is_empty() {
        return finding(
            true,
            format!(
                "codex hooks: klatch's hooks in {} have a trust record in {} that matches their current definition",
                file.display(),
                config.display()
            ),
            None,
        );
    }
    finding(
        false,
        format!(
            "codex hooks: klatch hook(s) in {} will be skipped silently in `codex exec`: {}",
            file.display(),
            problems.join("; ")
        ),
        Some(FIX),
    )
}

/// The finding about a project `.codex/hooks.json`, or none when it has no klatch hook. Where Codex keys a project
/// hook was not verified, so this says it cannot be checked.
fn project_finding(project_dir: &Path) -> Option<Finding> {
    let file = project_dir.join(".codex").join("hooks.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
    let has_klatch = events_of_json(&value)
        .iter()
        .flat_map(|(_, groups)| groups)
        .flat_map(|g| &g.handlers)
        .any(|h| crate::init::is_klatch_command(&h.command));
    has_klatch.then(|| {
        finding(
            false,
            format!(
                "codex hooks (project): klatch hooks in {} cannot be checked: Codex loads a project .codex layer only when the project is trusted, and where it records trust for these hooks is not verified",
                file.display()
            ),
            Some("open Codex in this project and review the hooks in /hooks"),
        )
    })
}

/// The findings about klatch's hooks in the Codex files: the user `hooks.json`, hooks written inline in
/// `config.toml`, and a project `.codex/hooks.json`. Empty when there is no klatch hook in any of them.
pub fn hook_findings(codex_home: &Path, project_dir: Option<&Path>) -> Vec<Finding> {
    let config = codex_home.join("config.toml");
    let config_text = std::fs::read_to_string(&config).unwrap_or_default();
    let trusted = trusted_hashes(&config_text);
    let mut found = Vec::new();
    let file = codex_home.join("hooks.json");
    if let Ok(text) = std::fs::read_to_string(&file) {
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => found.extend(sourced(&file, &config, &events_of_json(&value), &trusted)),
            Err(_) => found.push(finding(
                false,
                format!(
                    "codex hooks: {} is not valid JSON, so klatch cannot tell whether its hooks are trusted",
                    file.display()
                ),
                None,
            )),
        }
    }
    found.extend(sourced(
        &config,
        &config,
        &events_of_toml(&config_text),
        &trusted,
    ));
    found.extend(project_dir.and_then(project_finding));
    found
}

/// The finding for a source, when it holds a klatch hook.
fn sourced(
    file: &Path,
    config: &Path,
    events: &Events,
    trusted: &HashMap<String, String>,
) -> Option<Finding> {
    let judged = judge(file, events, trusted);
    (!judged.is_empty()).then(|| source_finding(file, config, &judged))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_spelled_the_way_codex_keys_a_trust_record() {
        assert_eq!(snake("PreToolUse"), "pre_tool_use");
        assert_eq!(snake("SessionStart"), "session_start");
        assert_eq!(snake("PostToolUseFailure"), "post_tool_use_failure");
    }

    #[test]
    fn only_a_section_with_a_sha256_hash_is_trust() {
        let cfg = "[hooks.state.\"a\"]\ntrusted_hash = \"sha256:1\"\n\n[hooks.state.\"b\"]\n\n[other]\ntrusted_hash = \"sha256:2\"\n[hooks.state.\"c\"]\ntrusted_hash = \"\"\n";
        let t = trusted_hashes(cfg);
        assert_eq!(t.get("a").map(String::as_str), Some("sha256:1"));
        assert!(!t.contains_key("b") && !t.contains_key("c") && !t.contains_key("other"));
    }

    #[test]
    fn the_hash_is_the_one_codex_reports_for_the_same_hook() {
        // the first is a real `currentHash` from `codex app-server` hooks/list (0.147.0 and 0.169.0); the second is
        // the same rule applied to a hook with no matcher (five real hashes agreed with the rule)
        let cmd = "/opt/klatch/klatch --discover hook --event SessionStart";
        let handler = Handler {
            command: cmd.into(),
            timeout: 600,
            ..Handler::default()
        };
        let startup = Group {
            matcher: Some("startup".into()),
            handlers: Vec::new(),
        };
        assert_eq!(
            trust_hash("SessionStart", &startup, &handler),
            "sha256:b7e2c8af3709751cf204a8346551891a6a5707fb48c9b60c42a085ca932338c2"
        );
        assert_eq!(
            trust_hash("SessionStart", &Group::default(), &handler),
            "sha256:1055e4969f2467390aa1f1af05f9c325b70f1fbd5f27c654eb5fdbff8ae4626d"
        );
    }

    #[test]
    fn inline_hooks_are_read_by_group_and_handler() {
        let cfg = "[[hooks.Stop]]\nmatcher = \"x\"\n[[hooks.Stop.hooks]]\ntype = \"command\"\ncommand = \"a \\\"b\\\"\"\ntimeout = 30\n[[hooks.Stop.hooks]]\ncommand = \"c\"\nasync = true\n[other]\ncommand = \"ignored\"\n";
        let events = events_of_toml(cfg);
        assert_eq!(events.len(), 1);
        let group = &events[0].1[0];
        assert_eq!(group.matcher.as_deref(), Some("x"));
        assert_eq!(group.handlers.len(), 2);
        assert_eq!(group.handlers[0].command, "a \"b\"");
        assert_eq!(group.handlers[0].timeout, 30);
        assert!(group.handlers[1].is_async && group.handlers[1].timeout == 600);
    }
}
