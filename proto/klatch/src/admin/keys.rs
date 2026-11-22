//! The commands about the key and the store file: they need the keystore as well as the gateway, and each one is in
//! the audit. `key_command` sends each to its own function.

use super::route::{ArchiveAct, KeyAct, Keystore};
use super::*;
use std::path::Path;

/// What every keystore command needs: where the store is, and who the audit says did it (the principal of the proof).
pub(super) struct KeyCtx<'a> {
    pub db: &'a Path,
    pub who: Principal,
}

impl KeyCtx<'_> {
    fn gateway(&self) -> Result<Gateway<SqliteStore>> {
        Ok(Gateway::new(
            custody::open_store(self.db)?,
            CliClock,
            RequireHuman::new(AllowAll),
        ))
    }

    /// Record an act in the audit.
    pub fn note(&self, action: &str, outcome: &str) -> Result<()> {
        Ok(self
            .gateway()?
            .admin_note(&self.who, (action, ""), outcome)?)
    }

    /// The store key, or an error saying `why` there is none.
    pub fn key_or(&self, why: &str) -> Result<klatch::keys::Key> {
        custody::current_key(self.db)?.ok_or_else(|| anyhow::anyhow!("{why}"))
    }
}

/// What the guard is asked before the act: the command's own name.
fn action_name(route: &Keystore) -> &'static str {
    match route {
        Keystore::Key(KeyAct::Recovery) => "recovery",
        Keystore::Key(KeyAct::Recover { .. }) => "recover",
        Keystore::Key(KeyAct::SealLabel { .. }) => "seal-label",
        Keystore::Key(KeyAct::RotateKey) => "rotate-key",
        Keystore::Key(KeyAct::RotateLabelKey { .. }) => "rotate-label-key",
        Keystore::Key(KeyAct::Backup { .. }) => "backup",
        Keystore::Key(KeyAct::Restore { .. }) => "restore",
        Keystore::Key(KeyAct::RunArchive { .. }) => "run-archive",
        Keystore::Key(KeyAct::Export { .. }) => "export",
        Keystore::Archive(ArchiveAct::Retention { .. }) => "retention",
        Keystore::Archive(ArchiveAct::Archive { .. }) => "archive",
        Keystore::Archive(ArchiveAct::RestoreArchive { .. }) => "restore-archive",
        Keystore::Archive(ArchiveAct::Archives) => "archives",
    }
}

/// Run a keystore act. The proof is the argument that makes it callable at all, and the gateway's guard is asked
/// for the act by name before anything touches the key or the store file (#1233).
pub(super) fn key_command(
    db: &Path,
    proof: &klatch::proof::AdminProof,
    route: Keystore,
) -> Result<()> {
    let ctx = KeyCtx {
        db,
        who: proof.principal(),
    };
    // The guard is asked on a gateway over a store of its own (in memory): the keystore family must be gated
    // BEFORE it opens the real store, because some acts (`recover`, `restore`) run exactly when there is no usable
    // key and opening the store would make one. The guard decides on the principal alone here; the act's own audit
    // row is written to the real store.
    Gateway::new(
        SqliteStore::open_in_memory()?,
        CliClock,
        RequireHuman::new(AllowAll),
    )
    .admin_authorize(&ctx.who, action_name(&route), "")?;
    match route {
        Keystore::Key(act) => key_act(&ctx, act),
        Keystore::Archive(act) => archive::archive_command(&ctx, act),
    }
}

fn key_act(ctx: &KeyCtx, act: KeyAct) -> Result<()> {
    match act {
        KeyAct::Recovery => recovery(ctx),
        KeyAct::Recover { code, label } => recover(ctx, &code, label.as_deref()),
        KeyAct::SealLabel { label, keep_plain } => add_label_seal(ctx, &label, keep_plain),
        KeyAct::RotateKey => rotate_key(ctx),
        KeyAct::RotateLabelKey { label } => rotate_label_key(ctx, &label),
        KeyAct::Backup { file } => backup(ctx, &file),
        KeyAct::Restore { file } => restore(ctx, &file),
        KeyAct::RunArchive { file, run } => run_archive(ctx, &file, &run),
        KeyAct::Export { room, json } => export::export(ctx, &room, json),
    }
}

fn recovery(ctx: &KeyCtx) -> Result<()> {
    let key = ctx.key_or("this store has no key yet; run any klatch command to make one")?;
    ctx.note("recovery-show", "")?;
    println!("{}", klatch::keys::recovery_code(&key));
    let gateway = ctx.gateway()?;
    for label in gateway.sealed_labels()? {
        if let Some(k) = custody::label_key(ctx.db, &label)? {
            println!("label {label}: {}", klatch::keys::recovery_code(&k));
        }
    }
    println!(
        "Keep these codes somewhere safe and private. Each restores one key; they are not stored anywhere else."
    );
    Ok(())
}

fn recover(ctx: &KeyCtx, code: &[String], label: Option<&str>) -> Result<()> {
    let key = klatch::keys::restore(&code.join(" ")).map_err(|e| anyhow::anyhow!("{e}"))?;
    match label {
        Some(l) => custody::restore_label_key(ctx.db, l, &key)?,
        None => custody::restore_key(ctx.db, &key)?,
    }
    ctx.note("recover-key", label.unwrap_or(""))?;
    println!("the key is back in place");
    Ok(())
}

fn run_archive(ctx: &KeyCtx, file: &Path, run: &str) -> Result<()> {
    let key = ctx.key_or("this store has no key to open an archive with")?;
    let text = run::read_archive(file, &key, run)?;
    // audited before anything is printed
    ctx.note("run-archive", &format!("{run} from {}", file.display()))?;
    print!("{text}");
    Ok(())
}

fn add_label_seal(ctx: &KeyCtx, label: &str, keep_plain: bool) -> Result<()> {
    let gateway = ctx.gateway()?;
    // a bad label makes no key
    gateway.check_sealable(label)?;
    // the key comes first: the messages already under the label are sealed with it in the same step as the marking
    let made = custody::ensure_label_key(ctx.db, label)?;
    let converted = if keep_plain {
        gateway.admin_seal_label_keep_plain(&ctx.who, label)?;
        None
    } else {
        gateway.use_label_keys(custody::label_ring(ctx.db, &[label.to_string()])?);
        let n = gateway.admin_seal_label(&ctx.who, label)?;
        // archive files made before the label was sealed hold the same plain text (#1153); a repeat finishes a crash
        let files = custody::wrap_label_archives(ctx.db, label)?;
        if files > 0 {
            println!("sealed the earlier text of the label in {files} archive file(s)");
        }
        Some(n)
    };
    println!(
        "label {label} is sealed: new messages under it are stored encrypted under a key of their own"
    );
    match converted {
        Some(n) => println!("converted {n} earlier message text(s) of the label to sealed text"),
        None => println!(
            "earlier messages were left as plain text (`klatch admin seal-label {label}` without --keep-plain seals them)"
        ),
    }
    if made {
        println!(
            "a key was made for it; `klatch admin recovery` shows the code that restores it. Keep that code: a sealed label whose key is lost cannot be read again."
        );
    }
    Ok(())
}

fn backup(ctx: &KeyCtx, file: &Path) -> Result<()> {
    let key = ctx.key_or("this store has no key yet; nothing to back up")?;
    custody::open_store(ctx.db)?.backup_to(file, &key)?;
    ctx.note("backup", &file.to_string_lossy())?;
    println!(
        "wrote an encrypted backup to {} (it opens only with this store's key: keep the recovery code too)",
        file.display()
    );
    Ok(())
}

fn restore(ctx: &KeyCtx, file: &Path) -> Result<()> {
    let key =
        ctx.key_or("this store has no key; restore the key first with `klatch admin recover`")?;
    klatch::store::encrypt::restore_backup(ctx.db, file, &key)?;
    ctx.note("restore", &file.to_string_lossy())?;
    println!(
        "restored {}; the store it replaced is kept as rooms.db.before-restore beside it",
        file.display()
    );
    Ok(())
}

fn rotate_key(ctx: &KeyCtx) -> Result<()> {
    let key = custody::rotate_key(ctx.db)?;
    ctx.note("rotate-key", "")?;
    println!(
        "the store is now encrypted under a new key. The old key and the old recovery code no longer work."
    );
    println!("New recovery code: {}", klatch::keys::recovery_code(&key));
    println!(
        "The archive files were moved to the new key too. Backups made by `klatch admin backup` keep the key they were made under and open only with it."
    );
    Ok(())
}

fn rotate_label_key(ctx: &KeyCtx, label: &str) -> Result<()> {
    // a label that is unknown or the lowest cannot be sealed, so it has no key to rotate
    ctx.gateway()?.check_sealable(label)?;
    let key = custody::rotate_label_key(ctx.db, label)?;
    ctx.note("rotate-label-key", label)?;
    println!(
        "label {label} is now sealed under a new key. The old key and its recovery code open none of its messages."
    );
    println!(
        "New recovery code for label {label}: {}",
        klatch::keys::recovery_code(&key)
    );
    println!(
        "Every message of the label in the store and in the archive files was moved. Workers launched with the old key hold a dead one: relaunch workers that hold this label."
    );
    println!(
        "Backups and archive copies made earlier keep the old key's text, so a leaked old key still opens those copies; remove them if that matters."
    );
    Ok(())
}
