//! `klatch run ...`: a launcher opens a run, whose rooms are `run.<id>.<name>`, and ends it, which removes them (#122).
//! Ending can write an encrypted archive first. Not for a worker and not for anything inside a coding-agent session.

use super::*;
use clap::Subcommand;
use klatch_core::Message;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum RunCmd {
    /// Open a run: its rooms (`run.<id>.<name>`) may be used from now on
    Create { id: String },
    /// End a run: every room of it is removed, after an optional encrypted archive of them
    End {
        id: String,
        /// Write an archive here first (encrypted under the store key; never replaces a file)
        #[arg(long)]
        archive: Option<PathBuf>,
    },
    /// List the open runs and how many rooms each has
    List,
}

/// One message of an archive, as a line of JSON.
fn line(room: &str, m: &Message) -> String {
    serde_json::json!({
        "room": room, "seq": m.seq, "id": m.id, "sender": m.sender, "verified": m.attribution == klatch_core::Attribution::Verified,
        "label": m.label, "ts": m.ts, "body": m.body, "kind": m.kind, "payload": m.payload,
    })
    .to_string()
}

/// What an archive is encrypted against: it opens only for this run.
fn context(id: &str) -> Vec<u8> {
    format!("klatch run archive {id}").into_bytes()
}

/// Write `rooms` as an encrypted archive of run `id`: one JSON line per message, protected under `key`, in a file
/// only its owner can read that is never put over another file.
pub(crate) fn write_archive(
    path: &std::path::Path,
    key: &klatch_keys::Key,
    id: &str,
    rooms: &[(String, Vec<Message>)],
) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut text = String::new();
    for (room, ms) in rooms {
        for m in ms {
            text.push_str(&line(room, m));
            text.push('\n');
        }
    }
    let blob = klatch_keys::protect(key, &context(id), text.as_bytes())
        .map_err(|e| anyhow::anyhow!("no random source: {e}"))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(&blob)?;
    f.sync_all()?;
    Ok(())
}

/// Read an archive back: the JSON lines.
pub(crate) fn read_archive(
    path: &std::path::Path,
    key: &klatch_keys::Key,
    id: &str,
) -> Result<String> {
    let blob = std::fs::read(path)?;
    let plain = klatch_keys::reveal(key, &context(id), &blob).map_err(|e| {
        anyhow::anyhow!("{e} (is this the archive of run {id}, made under this store's key?)")
    })?;
    Ok(String::from_utf8(plain)?)
}

fn refusal(holds_a_bundle: bool, agent_above: bool) -> Option<&'static str> {
    if holds_a_bundle {
        return Some("a worker cannot open, end or list runs");
    }
    if agent_above {
        return Some(
            "runs cannot be managed from inside a coding-agent session; run this from a launcher of your own",
        );
    }
    None
}

pub fn run(cmd: RunCmd) -> Result<()> {
    if let Some(why) = refusal(
        std::env::var_os("KLATCH_IDENTITY_FILE").is_some(),
        discover_session().is_ok(),
    ) {
        bail!("{why}");
    }
    let db = db_path()?;
    let gateway = Gateway::new(keys::open_store(&db)?, CliClock, AllowAll);
    // the person running the run holds every label key, so the archive holds what the rooms held
    gateway.use_label_keys(keys::label_ring(&db, &gateway.sealed_labels()?)?);
    let launcher = Principal::unattributed("admin");
    match cmd {
        RunCmd::Create { id } => {
            gateway.admin_run_create(&launcher, &id)?;
            println!("run {id} is open: its rooms are run.{id}.<name>");
        }
        RunCmd::End { id, archive } => {
            let gone = match archive {
                Some(path) => {
                    let key = keys::current_key(&db)?
                        .ok_or_else(|| anyhow::anyhow!("this store has no key to archive under"))?;
                    // written before the rooms go: a failure here (the file exists, the disk is full) fails the call
                    // and the run stays open
                    let mut write = |rooms: &[(String, Vec<Message>)]| -> klatch_core::Result<()> {
                        write_archive(&path, &key, &id, rooms)
                            .map_err(|e| klatch_core::Error::Backend(e.to_string()))
                    };
                    gateway.admin_run_end(&launcher, &id, Some(&mut write))?
                }
                None => gateway.admin_run_end(&launcher, &id, None)?,
            };
            println!(
                "run {id} ended: {} room(s) and {} message(s) removed",
                gone.rooms, gone.messages
            );
        }
        RunCmd::List => {
            for r in gateway.admin_runs(&launcher)? {
                println!(
                    "{}\t{} room(s)\topened {}s ago",
                    r.id,
                    r.rooms,
                    (now() - r.created).max(0)
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neither_a_worker_nor_an_agent_session_may_manage_runs() {
        assert!(refusal(false, false).is_none());
        assert!(refusal(true, false).unwrap().contains("worker cannot"));
        assert!(
            refusal(false, true)
                .unwrap()
                .contains("coding-agent session")
        );
    }
}
