//! The hook side of the machine checkpoint (#1408): what a PostToolUse input becomes in the member's trail. Only the
//! class of the tool call and, for an edit or a write, the repo-relative path survive; the command line, the file
//! content and the tool's output are read here and dropped, never passed on.

use super::*;
use klatch::domain::{ToolClass, clean_path, repo_relative};
use std::path::Path;

/// The nearest directory at or above `cwd` that holds a `.git` entry; `cwd` itself when there is none.
fn repo_root(cwd: &str) -> String {
    let mut dir = Path::new(cwd);
    loop {
        if dir.join(".git").exists() {
            return dir.to_string_lossy().into_owned();
        }
        match dir.parent() {
            Some(p) if !p.as_os_str().is_empty() => dir = p,
            _ => return cwd.to_string(),
        }
    }
}

/// The class of the call and the repo-relative paths it edited or wrote, from a PostToolUse input.
pub(super) fn tool_trace(input: &serde_json::Value) -> Option<(ToolClass, Vec<String>)> {
    let call = tool_input(input)?;
    let class = ToolClass::of_tool(&call.tool);
    if !class.keeps_paths() || call.cwd.is_empty() {
        return Some((class, Vec::new()));
    }
    let cwd = clean_path(&call.cwd, "/");
    let root = repo_root(&cwd);
    let paths = call
        .targets
        .iter()
        .filter_map(|t| repo_relative(&clean_path(t, &cwd), &root))
        .collect();
    Some((class, paths))
}

/// Add one tool call to the caller's trail. A failure to record never stops the hook.
pub(super) fn note_tool_call(t: &dyn Transport, input: Option<&serde_json::Value>) {
    let Some((class, paths)) = input.and_then(tool_trace) else {
        return;
    };
    if paths.is_empty() {
        let _ = t.note_tool(class.word(), None);
    }
    for p in &paths {
        let _ = t.note_tool(class.word(), Some(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tool_input_becomes_a_class_and_repo_relative_paths_and_nothing_else() {
        // no `.git` anywhere above it, so the working directory is the root
        let root = "/klatch-no-such-dir/proj";
        let edit = json!({"tool_name": "Edit", "cwd": format!("{root}/sub"),
            "tool_input": {"file_path": format!("{root}/sub/../src/a.rs"), "old_string": "SECRET", "new_string": "SECRET"}});
        // the root is the cwd here, so a path above it is outside
        assert_eq!(tool_trace(&edit).unwrap(), (ToolClass::Edit, Vec::new()));
        let inside = json!({"tool_name": "Edit", "cwd": root,
            "tool_input": {"file_path": format!("{root}/src/a.rs"), "new_string": "SECRET"}});
        assert_eq!(
            tool_trace(&inside).unwrap(),
            (ToolClass::Edit, vec!["src/a.rs".to_string()])
        );
        let run =
            json!({"tool_name": "Bash", "cwd": root, "tool_input": {"command": "echo SECRET"}});
        assert_eq!(tool_trace(&run).unwrap(), (ToolClass::Run, Vec::new()));
        let outside =
            json!({"tool_name": "Write", "cwd": root, "tool_input": {"file_path": "/etc/hosts"}});
        assert_eq!(
            tool_trace(&outside).unwrap(),
            (ToolClass::Write, Vec::new())
        );
        assert!(tool_trace(&json!({"cwd": root})).is_none());
    }
}
