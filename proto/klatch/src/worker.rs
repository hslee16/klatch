//! `klatch worker ...`: what a launcher (a script, a factory, an orchestrator that is not itself a coding-agent
//! session) uses to give a worker an identity (#121, #120). `issue` writes a key bundle for one worker; the launcher
//! hands the worker the path, in `KLATCH_IDENTITY_FILE`, and never the contents. A worker cannot issue, revoke or
//! list credentials, and neither can anything running inside a coding-agent session: otherwise an agent could mint
//! itself an identity.

use super::*;
use clap::Subcommand;
use klatch::gateway::CredentialSpec;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum WorkerCmd {
    /// Issue a credential for a worker and write its key bundle (mode 600, never replacing a file); or, with
    /// `--count`, for N workers named `<prefix>-1` to `<prefix>-N`, all or nothing
    Issue {
        /// One worker's name (with `--out`)
        #[arg(conflicts_with_all = ["count", "prefix", "out_dir"], required_unless_present = "count")]
        member: Option<String>,
        /// The highest label the worker may read and post, at most the agent cap
        #[arg(long)]
        clearance: String,
        /// The idle timeout in seconds (1 to 30 days): a credential lives this long from issue and, each time it is
        /// used, this long from that use, up to `--max-life`
        #[arg(long, default_value_t = 3600)]
        ttl: i64,
        /// The most seconds a credential lives from issue, which renewal on use never passes (1 second to 30
        /// days; default 24 hours)
        #[arg(long)]
        max_life: Option<i64>,
        /// The member that launched the worker, recorded in the audit
        #[arg(long)]
        parent: Option<String>,
        /// Where to write one worker's bundle
        #[arg(long, requires = "member", conflicts_with = "out_dir")]
        out: Option<PathBuf>,
        /// How many workers to issue in one call (1 to 500)
        #[arg(long, requires_all = ["prefix", "out_dir"])]
        count: Option<usize>,
        /// The name every worker of the batch starts with
        #[arg(long)]
        prefix: Option<String>,
        /// The directory the batch's bundles are written to, one `<name>.bundle` each
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Revoke a worker's credential: its token stops working at once
    Revoke { member: String },
    /// List credentials and where they stand
    List,
}

/// The most workers one call may issue.
const MAX_BATCH: usize = 500;

/// What to issue: each worker's name and the file its bundle goes to.
fn plan_issue(
    member: Option<String>,
    out: Option<PathBuf>,
    count: Option<usize>,
    prefix: Option<String>,
    out_dir: Option<PathBuf>,
) -> Result<Vec<(String, PathBuf)>> {
    match (member, out, count, prefix, out_dir) {
        (Some(m), Some(out), None, None, None) => Ok(vec![(m, out)]),
        (None, None, Some(n), Some(prefix), Some(dir)) => {
            if !(1..=MAX_BATCH).contains(&n) {
                bail!("a batch is 1 to {MAX_BATCH} workers");
            }
            Ok((1..=n)
                .map(|i| {
                    let name = format!("{prefix}-{i}");
                    let path = dir.join(format!("{name}.bundle"));
                    (name, path)
                })
                .collect())
        }
        _ => bail!("give one worker with --out, or --count with --prefix and --out-dir"),
    }
}

/// Issue every worker of the plan. All or nothing: if any one fails, every credential this call issued is revoked and
/// every bundle it wrote is removed, so no live credential is left that nobody holds the token of.
fn issue_all(
    (gateway, launcher, db): (&Gateway<SqliteStore>, &Principal, &std::path::Path),
    plan: &[(String, PathBuf)],
    (clearance, ttl, parent, max_life): (&str, i64, Option<&str>, Option<i64>),
) -> Result<Vec<(String, PathBuf)>> {
    if let Some(dir) = plan
        .first()
        .and_then(|(_, p)| p.parent())
        .filter(|d| !d.as_os_str().is_empty())
    {
        std::fs::create_dir_all(dir)?;
    }
    let mut done: Vec<(String, PathBuf)> = Vec::new();
    for (member, path) in plan {
        if let Err(e) = issue_one(
            (gateway, launcher, db),
            (member, path),
            (clearance, ttl, parent, max_life),
        ) {
            for (m, p) in &done {
                let _ = std::fs::remove_file(p);
                let _ = gateway.admin_revoke_credential(launcher, m);
            }
            return Err(e);
        }
        done.push((member.clone(), path.clone()));
    }
    Ok(done)
}

fn issue_one(
    (gateway, launcher, db): (&Gateway<SqliteStore>, &Principal, &std::path::Path),
    (member, out): (&str, &std::path::Path),
    (clearance, ttl, parent, max_life): (&str, i64, Option<&str>, Option<i64>),
) -> Result<()> {
    let spec = CredentialSpec {
        member,
        clearance,
        ttl,
        max_life,
        parent,
    };
    let token = gateway.admin_issue_credential(launcher, &spec)?;
    let written = (|| -> Result<()> {
        let store_key = custody::current_key(db)?
            .ok_or_else(|| anyhow::anyhow!("this store has no key to hand a worker"))?;
        let labels = gateway.sealed_labels_within(member)?;
        let ring = custody::label_ring(db, &labels)?;
        let label_keys = ring
            .labels()
            .into_iter()
            .filter_map(|l| {
                ring.get(l)
                    .map(|k| (l.to_string(), klatch::keys::Key::from_bytes(*k.expose())))
            })
            .collect();
        klatch::keys::Bundle {
            member: member.to_string(),
            token,
            store_key,
            label_keys,
        }
        .write(out)?;
        Ok(())
    })();
    if let Err(e) = written {
        // no live credential may be left that nobody holds the token of
        let _ = gateway.admin_revoke_credential(launcher, member);
        return Err(e);
    }
    Ok(())
}

pub fn run(cmd: WorkerCmd) -> Result<()> {
    let db = db_path()?;
    // the passphrase first (#1233): nothing below is reachable without the proof
    let (store, proof) = crate::admin::prove::open_and_require(&db)?;
    let gateway = Gateway::new(
        store,
        CliClock,
        ForbidFloors::new(RequireHuman::new(AllowAll)),
    );
    let launcher = proof.principal();
    match cmd {
        WorkerCmd::Issue {
            member,
            clearance,
            ttl,
            max_life,
            parent,
            out,
            count,
            prefix,
            out_dir,
        } => {
            let plan = plan_issue(member, out, count, prefix, out_dir)?;
            let issued = issue_all(
                (&gateway, &launcher, &db),
                &plan,
                (&clearance, ttl, parent.as_deref(), max_life),
            )?;
            let cap = max_life.unwrap_or(klatch::gateway::DEFAULT_CREDENTIAL_MAX_LIFE_SECS);
            for (member, path) in &issued {
                println!(
                    "wrote the key bundle for {member} to {} (idle timeout {ttl}s, renewed on use, at most {cap}s from now)",
                    path.display()
                );
            }
            println!(
                "Give each worker the path of its bundle in KLATCH_IDENTITY_FILE, never the contents."
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
                    "{}\t{}\tclearance {}\tparent {}\texpires in {}s\tcap in {}s\tlast renewed {}",
                    c.member,
                    c.state,
                    c.clearance,
                    if c.parent.is_empty() {
                        "none"
                    } else {
                        &c.parent
                    },
                    (c.expires - now).max(0),
                    (c.cap - now).max(0),
                    c.last_renewed.map_or("never".to_string(), |t| format!(
                        "{}s ago",
                        (now - t).max(0)
                    ))
                );
            }
        }
    }
    Ok(())
}
