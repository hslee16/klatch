//! The commands about the key and the store file: they need the keystore as well as the gateway, and each one is in
//! the audit. `key_command` sends each to its own function.

use super::route::{KeyAct, Keystore};
use super::*;
use std::path::Path;

/// What every keystore command needs: where the store is, and who the audit says did it.
pub(super) struct KeyCtx<'a> {
    pub db: &'a Path,
    pub who: Principal,
}

impl KeyCtx<'_> {
    fn gateway(&self) -> Result<Gateway<SqliteStore>> {
        Ok(Gateway::new(
            custody::open_store(self.db)?,
            CliClock,
            AllowAll,
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

pub(super) fn key_command(db: &Path, route: Keystore) -> Result<()> {
    let ctx = KeyCtx {
        db,
        who: Principal::unattributed("admin"),
    };
    match route {
        Keystore::Key(act) => key_act(&ctx, act),
        Keystore::Archive(act) => archive::archive_command(&ctx, act),
    }
}

fn key_act(ctx: &KeyCtx, act: KeyAct) -> Result<()> {
    match act {
        KeyAct::Recovery => recovery(ctx),
        KeyAct::Recover { code, label } => recover(ctx, &code, label.as_deref()),
        KeyAct::SealLabel { label } => add_label_seal(ctx, &label),
        KeyAct::RotateKey => rotate_key(ctx),
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

fn add_label_seal(ctx: &KeyCtx, label: &str) -> Result<()> {
    ctx.gateway()?.admin_seal_label(&ctx.who, label)?;
    let made = custody::ensure_label_key(ctx.db, label)?;
    println!(
        "label {label} is sealed: new messages under it are stored encrypted under a key of their own"
    );
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
