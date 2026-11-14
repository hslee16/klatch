use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "the_store_is_encrypted_and_a_lost_key_comes_back_from_its_recovery_code",
            origin: "#155, #156, R32, R34",
            layer: "fault",
            run: lost_key,
        },
        Scenario {
            name: "a_backup_is_encrypted_a_restore_brings_it_back_and_a_readable_export_is_audited",
            origin: "#159, R38",
            layer: "fault",
            run: backup_and_export,
        },
        Scenario {
            name: "rotating_the_key_keeps_every_message_and_retires_the_old_key",
            origin: "#155, R34",
            layer: "fault",
            run: rotation,
        },
    ]
}

fn read_all(dir: &std::path::Path) -> Vec<u8> {
    let mut all = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path().is_file() {
                all.extend(std::fs::read(e.path()).unwrap_or_default());
            }
        }
    }
    all
}

fn key_files(w: &World) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(w.home.join("keys"))
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn lost_key(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["post", "vault", "the launch code is hunter2"])?;
    let data = read_all(&w.home);
    check(
        !data.windows(7).any(|x| x == b"hunter2") && !data.windows(5).any(|x| x == b"vault"),
        "the store is readable on disk",
    )?;
    // the code is shown behind the terminal gate and is audited
    let code = w.admin(&["recovery"], "");
    check(code.ok, format!("recovery failed: {}", code.stderr))?;
    let code_line = code
        .stdout
        .lines()
        .find(|l| l.starts_with("KLATCH-"))
        .ok_or("no recovery code printed")?
        .to_string();
    has(
        &w.admin(&["audit"], "").stdout,
        "recovery-show",
        "the audit does not show the code was read",
    )?;
    // lose the key: the store is refused, loudly, and no new key is made
    for k in key_files(w) {
        std::fs::remove_file(k).map_err(|e| e.to_string())?;
    }
    has(
        &w.refused("alice", &["read", "vault"])?,
        "klatch admin recover",
        "a lost key was not refused with the way back",
    )?;
    check(
        key_files(w).is_empty(),
        "a new key was made over an existing store",
    )?;
    // a wrong code is refused and changes nothing; the right one restores the key
    let wrong = w.admin(&["recover", "KLATCH-AAAA-AAAA"], "recover");
    check(!wrong.ok, "a wrong code was accepted")?;
    check(key_files(w).is_empty(), "a wrong code put a key in place")?;
    let right = w.admin(&["recover", &code_line], "recover");
    check(right.ok, format!("recover failed: {}", right.stderr))?;
    has(
        &w.ok("bob", &["read", "vault"])?,
        "hunter2",
        "the data did not come back",
    )?;
    Ok(vec![])
}

fn rotation(w: &World) -> Res {
    w.set_clock(T0);
    for i in 0..40 {
        w.ok("alice", &["post", "r", &format!("message number {i}")])?;
    }
    let old_key = std::fs::read(&key_files(w)[0]).map_err(|e| e.to_string())?;
    let rotated = w.admin(&["rotate-key"], "rotate");
    check(rotated.ok, format!("rotate failed: {}", rotated.stderr))?;
    has(
        &rotated.stdout,
        "New recovery code: KLATCH-",
        "no new recovery code",
    )?;
    let new_key = std::fs::read(&key_files(w)[0]).map_err(|e| e.to_string())?;
    check(old_key != new_key, "the key did not change")?;
    check(key_files(w).len() == 1, "a temporary key file was left")?;
    let read = w.ok("bob", &["read", "r"])?;
    check(
        read.lines().count() == 40,
        format!("{} of 40 messages after rotation", read.lines().count()),
    )?;
    // the old key no longer opens the store
    std::fs::write(&key_files(w)[0], &old_key).map_err(|e| e.to_string())?;
    check(
        !w.run("carol", &["read", "r"]).ok,
        "the old key still opens the store",
    )?;
    Ok(vec![])
}

fn backup_and_export(w: &World) -> Res {
    w.set_clock(T0);
    w.ok(
        "alice",
        &["post", "r", "kept in the backup: the code is hunter2"],
    )?;
    let file = w.home.join("archive.db");
    let file_s = file.to_string_lossy().into_owned();
    let made = w.admin(&["backup", &file_s], "");
    check(made.ok, format!("backup failed: {}", made.stderr))?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    check(
        !bytes.windows(7).any(|x| x == b"hunter2"),
        "the backup holds plaintext",
    )?;
    check(
        !w.admin(&["backup", &file_s], "").ok,
        "a backup overwrote a file",
    )?;
    w.ok("alice", &["post", "r", "posted after the backup"])?;
    let restored = w.admin(&["restore", &file_s], "restore");
    check(restored.ok, format!("restore failed: {}", restored.stderr))?;
    let read = w.ok("bob", &["read", "r", "--peek"])?;
    has(
        &read,
        "hunter2",
        "the backed up message is gone after a restore",
    )?;
    lacks(
        &read,
        "posted after the backup",
        "the restore kept a later message",
    )?;
    check(
        w.home.join("rooms.db.before-restore").exists(),
        "the replaced store was not kept",
    )?;
    // the readable export is the one deliberate way out, and it leaves a record
    let export = w.admin(&["export", "r"], "r");
    check(export.ok, format!("export failed: {}", export.stderr))?;
    has(&export.stdout, "hunter2", "the export is not readable")?;
    has(
        &w.admin(&["audit"], "").stdout,
        "export",
        "the export is not in the audit",
    )?;
    check(
        !w.admin(&["export", "r"], "wrong").ok,
        "an export ran without the room typed back",
    )?;
    Ok(vec![])
}
