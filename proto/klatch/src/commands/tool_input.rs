//! What a tool call looks like to a hook: the tool, the paths it names, its command and the working directory. Shared by
//! the claim hook (`PreToolUse`), the machine checkpoint and the passive claims.

use super::*;

/// One tool call as a `PreToolUse` hook is given it.
pub(super) struct ToolInput {
    pub(super) tool: String,
    pub(super) targets: Vec<String>,
    pub(super) command: String,
    pub(super) cwd: String,
}

pub(super) fn tool_input(v: &serde_json::Value) -> Option<ToolInput> {
    let tool = v.get("tool_name")?.as_str()?.to_string();
    let input = v.get("tool_input")?;
    let text = |k: &str| input.get(k).and_then(|x| x.as_str());
    let mut targets: Vec<String> = ["file_path", "notebook_path", "path"]
        .iter()
        .filter_map(|k| text(k))
        .map(String::from)
        .collect();
    if tool == "apply_patch" {
        targets.extend(patch_targets(
            text("command").or(text("input")).unwrap_or(""),
        ));
    }
    Some(ToolInput {
        tool,
        targets,
        command: text("command").unwrap_or_default().to_string(),
        cwd: v
            .get("cwd")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// `path` with the symlinks in it resolved, when that differs from the path as written. The part that does not
/// exist yet (a file about to be created) is kept as written under its nearest existing directory, so the result
/// is where the file will land. Taken from `cwd` when relative.
pub(super) fn real_path(path: &str, cwd: &str) -> Option<String> {
    let abs = if path.starts_with('/') || cwd.is_empty() {
        std::path::PathBuf::from(path)
    } else {
        std::path::Path::new(cwd).join(path)
    };
    let mut existing = abs.as_path();
    let mut tail = Vec::new();
    while !existing.exists() {
        tail.push(existing.file_name()?);
        existing = existing.parent()?;
    }
    let mut real = existing.canonicalize().ok()?;
    real.extend(tail.iter().rev());
    let real = real.to_str()?.to_string();
    (real != klatch::domain::clean_path(path, cwd)).then_some(real)
}
