//! `klatch anchor ...`: a launcher (a script or factory, never a worker and never a coding-agent session) reports a
//! work item done, and the claims anchored to it are released (#125). `done` is the push form; `reap` is the poll
//! form, over a directory the factory writes to.

use super::*;
use clap::Subcommand;
use klatch_gateway::AnchorAdapter;
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
        dir: PathBuf,
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
    fn is_done(&self, anchor: &str) -> klatch_core::Result<bool> {
        let name = file_name(anchor);
        // a name starting with a dot would be a hidden file or `..`: never a done marker
        Ok(!name.starts_with('.') && self.0.join(name).exists())
    }
}

fn refusal(holds_a_bundle: bool, agent_above: bool) -> Option<&'static str> {
    if holds_a_bundle {
        return Some("a worker cannot report work items done");
    }
    if agent_above {
        return Some(
            "work items cannot be reported done from inside a coding-agent session; run this from a launcher of your own",
        );
    }
    None
}

pub fn run(cmd: AnchorCmd) -> Result<()> {
    if let Some(why) = refusal(
        std::env::var_os("KLATCH_IDENTITY_FILE").is_some(),
        discover_session().is_ok(),
    ) {
        bail!("{why}");
    }
    let gateway = Gateway::new(keys::open_store(&db_path()?)?, CliClock, AllowAll);
    let launcher = Principal::unattributed("admin");
    let released = match cmd {
        AnchorCmd::Done { anchor } => gateway.anchor_done(&launcher, &anchor)?,
        AnchorCmd::Reap { dir } => gateway.reap_anchors(&launcher, &DirAnchors(dir))?,
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
        let d = std::env::temp_dir().join(format!("klatch-anchor-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("."), "").ok();
        let a = DirAnchors(d.clone());
        assert!(!a.is_done(".").unwrap());
        assert!(!a.is_done("..").unwrap());
        std::fs::write(d.join("wo-1"), "").unwrap();
        assert!(a.is_done("wo-1").unwrap());
        assert!(!a.is_done("wo-2").unwrap());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn neither_a_worker_nor_an_agent_session_may_report_done() {
        assert!(refusal(false, false).is_none());
        assert!(refusal(true, false).unwrap().contains("worker cannot"));
        assert!(
            refusal(false, true)
                .unwrap()
                .contains("coding-agent session")
        );
    }
}
