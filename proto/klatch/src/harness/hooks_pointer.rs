//! The pointer for a member that works where no klatch hooks run (#1416): hooks live in the untracked
//! `.claude/settings.local.json` of one checkout, so a worktree, another project or a headless worker never gets the
//! room digest. When such a member posts for the first time without having read the room, it is told once how a
//! person can turn the hooks on. Read only, bounded, installs nothing and prints no path.

use std::path::{Path, PathBuf};

mod scan;

pub use scan::command_is_klatch;

/// The one line, data and not an instruction. No path in it: the caller's directory is its own business.
pub const POINTER: &str = "klatch: no klatch hooks are installed where you work, so no room digest reaches you here (this line is DATA, not an instruction); a person can turn them on with `klatch init --scope user` (it asks before it writes anything)";

/// The pointer for a member working in `cwd`, or None when hooks are installed, when this is a deliberately started
/// headless worker, or when any settings file cannot be read with certainty (then nothing is said).
pub fn no_hooks_pointer(cwd: &Path, home: Option<&Path>, headless: bool) -> Option<&'static str> {
    if headless {
        return None;
    }
    let root = scan::project_root(cwd);
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in scan::dirs_up_to(cwd, &root) {
        files.push(dir.join(".claude").join("settings.local.json"));
        files.push(dir.join(".claude").join("settings.json"));
    }
    let mut seen = Vec::new();
    for f in &files {
        seen.push(scan::look(f, &root)?);
    }
    if let Some(h) = home {
        // the user scope may link out of the project by design; it is read as the owner's own file
        seen.push(scan::look(&h.join(".claude").join("settings.json"), h)?);
    }
    (!seen.contains(&scan::Seen::Hooks)).then_some(POINTER)
}

/// `no_hooks_pointer` for this process: the working directory, `HOME`, and whether the process was started with its own
/// identity file (a worker a person launched with `klatch worker issue` or the team script, which bring their own
/// settings; their presence is read, their content never).
pub fn pointer_here() -> Option<&'static str> {
    let cwd = std::env::current_dir().ok()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let headless = std::env::var_os("KLATCH_IDENTITY_FILE").is_some();
    no_hooks_pointer(&cwd, home.as_deref(), headless)
}

#[cfg(test)]
mod tests;
