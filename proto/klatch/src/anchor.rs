//! `klatch anchor ...`: a launcher (a script or factory, never a worker and never a coding-agent session) reports a
//! work item done, and the claims anchored to it are released (#125). `done` is the push form; `reap` is the poll
//! form, over a directory the factory writes to.

use super::*;
use clap::Subcommand;
use klatch::gateway::AnchorAdapter;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum AnchorCmd {
    /// A work item is done: release every claim anchored to it
    Done { anchor: String },
    /// Release the claims of every item the factory marked done: an item is done when a file named after it exists in
    /// the directory (characters outside letters, digits, dot and dash are written as `_` and two hex digits, so
    /// `mission/42` is the file `mission_2f42`)
    Reap {
        #[arg(long)]
        dir: Option<PathBuf>,
        /// A directory whose subdirectories are worktrees: a claim anchored `wt:<name>` is released once
        /// `<name>` is no longer in it (the worktree was removed). Needs an existing directory, so a wrong path
        /// releases nothing
        #[arg(long)]
        worktrees: Option<PathBuf>,
    },
    /// A GitHub pull request or issue changed: read `gh api` output or a webhook payload (from a file, or from the
    /// standard input) and release the claims anchored to each pull request merged or closed and each issue closed.
    /// Claim with `--anchor gh:pr:OWNER/REPO/N` or `gh:issue:OWNER/REPO/N`. klatch never calls GitHub and does not
    /// check a webhook signature: pipe in only what you already trust, for example
    /// `gh api repos/O/R/pulls/7 | klatch anchor github`
    Github {
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

/// The file name of an anchor: safe on every file system, and never a path.
pub(crate) fn file_name(anchor: &str) -> String {
    anchor
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c.to_string()
            } else {
                format!("_{:02x}", c as u32)
            }
        })
        .collect()
}

/// Done when the factory has written a file for the item.
pub(crate) struct DirAnchors(pub PathBuf);

impl AnchorAdapter for DirAnchors {
    fn is_done(&self, anchor: &str) -> klatch::domain::Result<bool> {
        let name = file_name(anchor);
        // a name starting with a dot would be a hidden file or `..`: never a done marker
        Ok(!name.starts_with('.') && self.0.join(name).exists())
    }
}

/// Done when a worktree directory is gone: the anchor is `wt:<name>`, the worktree is `<root>/<name>`.
pub(crate) struct WorktreeAnchors(pub PathBuf);

impl AnchorAdapter for WorktreeAnchors {
    fn is_done(&self, anchor: &str) -> klatch::domain::Result<bool> {
        let unanswered = |why: &str| Err(klatch::domain::Error::Backend(why.to_string()));
        let Some(name) = anchor.strip_prefix("wt:") else {
            return unanswered("not a worktree anchor");
        };
        // a name that is a path, or hidden, never names a worktree; and a root that is not there says nothing
        if name.is_empty() || name.starts_with('.') || name.contains('/') || !self.0.is_dir() {
            return unanswered("cannot tell");
        }
        Ok(!self.0.join(name).exists())
    }
}

/// Sends each anchor to the adapter that knows its kind; a kind without one is left to its lease.
pub(crate) struct Routed {
    pub dir: Option<DirAnchors>,
    pub worktrees: Option<WorktreeAnchors>,
}

impl AnchorAdapter for Routed {
    fn is_done(&self, anchor: &str) -> klatch::domain::Result<bool> {
        let adapter: Option<&dyn AnchorAdapter> = if anchor.starts_with("wt:") {
            self.worktrees.as_ref().map(|a| a as &dyn AnchorAdapter)
        } else {
            self.dir.as_ref().map(|a| a as &dyn AnchorAdapter)
        };
        match adapter {
            Some(a) => a.is_done(anchor),
            None => Err(klatch::domain::Error::Backend(
                "no adapter for this anchor".into(),
            )),
        }
    }
}

fn read_events(file: Option<&PathBuf>) -> Result<String> {
    use std::io::Read;
    let mut text = String::new();
    match file {
        Some(f) => text = std::fs::read_to_string(f)?,
        None => {
            std::io::stdin().take(4 << 20).read_to_string(&mut text)?;
        }
    }
    Ok(text)
}

pub fn run(cmd: AnchorCmd) -> Result<()> {
    // the passphrase first (#1233): nothing below is reachable without the proof
    let (store, proof) = crate::admin::prove::open_and_require(&db_path()?)?;
    let gateway = Gateway::new(
        store,
        CliClock,
        ForbidFloors::new(RequireHuman::new(AllowAll)),
    );
    let launcher = proof.principal();
    let released = match cmd {
        AnchorCmd::Done { anchor } => gateway.anchor_done(&launcher, &anchor)?,
        AnchorCmd::Reap { dir, worktrees } => {
            if dir.is_none() && worktrees.is_none() {
                bail!("give --dir, --worktrees or both");
            }
            let routed = Routed {
                dir: dir.map(DirAnchors),
                worktrees: worktrees.map(WorktreeAnchors),
            };
            gateway.reap_anchors(&launcher, &routed)?
        }
        AnchorCmd::Github { file } => {
            gateway.github_events(&launcher, &read_events(file.as_ref())?)?
        }
    };
    println!("released {released} claim(s)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anchor_becomes_a_safe_file_name_and_never_a_path() {
        assert_eq!(file_name("wo-7.a"), "wo-7.a");
        assert_eq!(file_name("mission/42"), "mission_2f42");
        assert_eq!(file_name("wo:7"), "wo_3a7");
        assert_eq!(file_name("../x"), ".._2fx");
        assert!(!file_name("a/b/c").contains('/'));
    }

    #[test]
    fn a_dot_file_is_never_a_done_marker() {
        let d = temp_dir("dot");
        std::fs::write(d.join("."), "").ok();
        let a = DirAnchors(d.to_path_buf());
        assert!(!a.is_done(".").unwrap());
        assert!(!a.is_done("..").unwrap());
        std::fs::write(d.join("wo-1"), "").unwrap();
        assert!(a.is_done("wo-1").unwrap());
        assert!(!a.is_done("wo-2").unwrap());
    }

    fn temp_dir(tag: &str) -> crate::test_dir::TestDir {
        crate::test_dir::TestDir::new(&format!("anchor-{tag}"))
    }

    #[test]
    fn a_worktree_anchor_is_done_when_its_directory_is_gone() {
        let root = temp_dir("gone");
        std::fs::create_dir(root.join("feature-x")).unwrap();
        let a = WorktreeAnchors(root.to_path_buf());
        assert!(!a.is_done("wt:feature-x").unwrap());
        std::fs::remove_dir(root.join("feature-x")).unwrap();
        assert!(a.is_done("wt:feature-x").unwrap());
    }

    #[test]
    fn a_worktree_anchor_that_is_a_path_or_a_missing_root_says_nothing() {
        let root = temp_dir("odd");
        let a = WorktreeAnchors(root.to_path_buf());
        for odd in ["wt:", "wt:../x", "wt:a/b", "wt:.hidden", "wo:7"] {
            assert!(a.is_done(odd).is_err(), "{odd}");
        }
        let missing = WorktreeAnchors(root.join("not-there"));
        assert!(
            missing.is_done("wt:x").is_err(),
            "a wrong root freed a claim"
        );
    }

    #[test]
    fn each_anchor_goes_to_its_own_adapter_and_a_kind_with_none_is_left_alone() {
        let root = temp_dir("routed");
        let marks = temp_dir("routed-marks");
        std::fs::write(marks.join("wo-1"), "").unwrap();
        let both = Routed {
            dir: Some(DirAnchors(marks.to_path_buf())),
            worktrees: Some(WorktreeAnchors(root.to_path_buf())),
        };
        assert!(both.is_done("wo-1").unwrap());
        assert!(both.is_done("wt:gone").unwrap());
        let only_dir = Routed {
            dir: Some(DirAnchors(marks.to_path_buf())),
            worktrees: None,
        };
        assert!(only_dir.is_done("wt:gone").is_err());
        let only_wt = Routed {
            dir: None,
            worktrees: Some(WorktreeAnchors(root.to_path_buf())),
        };
        assert!(only_wt.is_done("wo-1").is_err());
    }
}
