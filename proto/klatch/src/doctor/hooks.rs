//! The hook checks: whether klatch's hooks in the Claude Code settings point at this binary, and which are on.

use super::{Report, Status};
use crate::init::{HookKind, hook_program, installed_hooks};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The settings file's content, if it exists and parses; `Err` says why not.
fn settings(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("not valid JSON ({e})"))
}

/// The settings files that may hold klatch's hooks, labelled by scope.
fn settings_places(project_dir: &Path, home: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    let mut places = vec![(
        "project",
        project_dir.join(".claude").join("settings.local.json"),
    )];
    if let Some(h) = home {
        places.push(("user", h.join(".claude").join("settings.json")));
    }
    places
}

/// A settings file to check and the scope it stands for.
#[derive(Clone, Copy)]
struct Place<'a> {
    label: &'a str,
    path: &'a Path,
}

type Installed = Vec<(String, String, HookKind)>;

/// The worst thing wrong with the installed hooks: a program that does not exist (reported at once), else the first
/// one that is not this binary.
fn hook_problem(installed: &Installed, exe: &Path) -> Option<(Status, String)> {
    let mut problem = None;
    for (event, cmd, _) in installed {
        let program = hook_program(cmd);
        if !Path::new(program).is_file() {
            return Some((
                Status::Fail,
                format!("the {event} hook runs {program}, which does not exist"),
            ));
        }
        let same = std::fs::canonicalize(program).ok() == std::fs::canonicalize(exe).ok();
        if !same && problem.is_none() {
            problem = Some((
                Status::Warn,
                format!(
                    "the {event} hook runs {program}, not this binary ({})",
                    exe.display()
                ),
            ));
        }
    }
    problem
}

/// The line for one settings file whose hooks are installed.
fn report_place(r: &mut Report, place: &Place, installed: &Installed, exe: &Path) {
    let Place { label, path } = *place;
    match hook_problem(installed, exe) {
        Some((status, why)) => r.add(
            status,
            format!("hooks ({label}): {why}"),
            Some(&format!(
                "run klatch init --scope {label} again; it rewrites the path to this binary"
            )),
        ),
        None => {
            let mut events: Vec<&str> = installed.iter().map(|h| h.0.as_str()).collect();
            events.sort_unstable();
            events.dedup();
            r.add(
                Status::Ok,
                format!(
                    "hooks ({label}): klatch runs on {} from {}",
                    events.join(", "),
                    path.display()
                ),
                None,
            );
        }
    }
}

/// Which optional hooks one settings file turns on.
#[derive(Default, Clone, Copy)]
struct Optional {
    wake: bool,
    enforce: bool,
    pre_edit: bool,
}

/// Check one settings file. `None` when it holds no klatch hooks (or none could be read, which is said first).
fn check_place(r: &mut Report, place: &Place, exe: &Path) -> Option<Optional> {
    let Place { label, path } = *place;
    let value = match settings(path) {
        Ok(Some(v)) => v,
        Ok(None) => return None,
        Err(e) => {
            r.add(
                Status::Warn,
                format!("hooks ({label}): {} could not be read: {e}", path.display()),
                None,
            );
            return None;
        }
    };
    let installed = installed_hooks(&value);
    if installed.is_empty() {
        return None;
    }
    let on = |kind| installed.iter().any(|h| h.2 == kind);
    let found = Optional {
        wake: on(HookKind::Wake),
        enforce: on(HookKind::Enforce),
        pre_edit: on(HookKind::PreEdit),
    };
    report_place(r, place, &installed, exe);
    Some(found)
}

pub(super) fn check_hooks(r: &mut Report, project_dir: &Path, home: Option<&Path>, exe: &Path) {
    let mut found: Option<Optional> = None;
    for (label, path) in settings_places(project_dir, home) {
        if let Some(here) = check_place(r, &Place { label, path: &path }, exe) {
            let all = found.get_or_insert_with(Optional::default);
            all.wake |= here.wake;
            all.enforce |= here.enforce;
            all.pre_edit |= here.pre_edit;
        }
    }
    let Some(on) = found else {
        r.add(
            Status::Warn,
            "hooks: klatch's hooks are not installed in the project or user settings (they are only needed for Claude Code sessions)",
            Some("klatch init"),
        );
        return;
    };
    report_switch(r, "wake", on.wake, "--wake");
    report_switch(r, "enforce", on.enforce, "--enforce");
    report_switch(r, "pre-edit-warn", on.pre_edit, "--pre-edit-warn");
}

/// The line saying whether an optional hook is on, and the flag that turns it on.
fn report_switch(r: &mut Report, name: &str, on: bool, flag: &str) {
    let state = if on {
        "on".to_string()
    } else {
        format!("off (klatch init {flag} turns it on)")
    };
    r.add(Status::Ok, format!("{name} hook: {state}"), None);
}
