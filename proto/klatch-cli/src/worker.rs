//! `klatch worker ...`: what a launcher (a script, a factory, an orchestrator that is not itself a coding-agent
//! session) uses to give a worker an identity (#121, #120). `issue` writes a key bundle for one worker; the launcher
//! hands the worker the path, in `KLATCH_IDENTITY_FILE`, and never the contents. A worker cannot issue, revoke or
//! list credentials, and neither can anything running inside a coding-agent session: otherwise an agent could mint
//! itself an identity.

use super::*;
use clap::Subcommand;
use klatch_gateway::CredentialSpec;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum WorkerCmd {
    /// Issue a credential for a worker and write its key bundle (mode 600, never replacing a file)
    Issue {
        member: String,
        /// The highest label the worker may read and post, at most the agent cap
        #[arg(long)]
        clearance: String,
        /// Seconds the credential lives (1 to 30 days)
        #[arg(long, default_value_t = 3600)]
        ttl: i64,
        /// The member that launched the worker, recorded in the audit
        #[arg(long)]
        parent: Option<String>,
        /// Where to write the bundle
        #[arg(long)]
        out: PathBuf,
    },
    /// Revoke a worker's credential: its token stops working at once
    Revoke { member: String },
    /// List credentials and where they stand
    List,
}

/// Why this process may not manage credentials, or `None`.
fn refusal(holds_a_bundle: bool, agent_above: bool) -> Option<&'static str> {
    if holds_a_bundle {
        return Some("a worker cannot issue, revoke or list credentials");
    }
    if agent_above {
        return Some(
            "credentials cannot be managed from inside a coding-agent session; run this from a launcher of your own",
        );
    }
    None
}

pub fn run(cmd: WorkerCmd) -> Result<()> {
    if let Some(why) = refusal(
        std::env::var_os("KLATCH_IDENTITY_FILE").is_some(),
        discover_session().is_ok(),
    ) {
        bail!("{why}");
    }
    let db = db_path()?;
    let gateway = Gateway::new(keys::open_store(&db)?, CliClock, AllowAll);
    let launcher = Principal::unattributed("admin");
    match cmd {
        WorkerCmd::Issue {
            member,
            clearance,
            ttl,
            parent,
            out,
        } => {
            let spec = CredentialSpec {
                member: &member,
                clearance: &clearance,
                ttl,
                parent: parent.as_deref(),
            };
            let token = gateway.admin_issue_credential(&launcher, &spec)?;
            let written = (|| -> Result<()> {
                let store_key = keys::current_key(&db)?
                    .ok_or_else(|| anyhow::anyhow!("this store has no key to hand a worker"))?;
                let labels = gateway.sealed_labels_within(&member)?;
                let ring = keys::label_ring(&db, &labels)?;
                let label_keys = ring
                    .labels()
                    .into_iter()
                    .filter_map(|l| {
                        ring.get(l)
                            .map(|k| (l.to_string(), klatch_keys::Key::from_bytes(*k.expose())))
                    })
                    .collect();
                klatch_keys::Bundle {
                    member: member.clone(),
                    token,
                    store_key,
                    label_keys,
                }
                .write(&out)?;
                Ok(())
            })();
            if let Err(e) = written {
                // no live credential may be left that nobody holds the token of
                let _ = gateway.admin_revoke_credential(&launcher, &member);
                return Err(e);
            }
            println!(
                "wrote the key bundle for {member} to {} (expires in {ttl}s). Give the worker the path in KLATCH_IDENTITY_FILE, never the contents.",
                out.display()
            );
        }
        WorkerCmd::Revoke { member } => {
            gateway.admin_revoke_credential(&launcher, &member)?;
            println!("revoked the credential of {member}");
        }
        WorkerCmd::List => {
            let now = now();
            for c in gateway.admin_credentials(&launcher)? {
                println!(
                    "{}\t{}\tclearance {}\tparent {}\texpires in {}s",
                    c.member,
                    c.state,
                    c.clearance,
                    if c.parent.is_empty() {
                        "none"
                    } else {
                        &c.parent
                    },
                    (c.expires - now).max(0)
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
    fn neither_a_worker_nor_an_agent_session_may_manage_credentials() {
        assert!(refusal(false, false).is_none());
        assert!(refusal(true, false).unwrap().contains("worker cannot"));
        assert!(
            refusal(false, true)
                .unwrap()
                .contains("coding-agent session")
        );
        assert!(refusal(true, true).is_some());
    }
}
