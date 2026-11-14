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
            name: "a_sealed_label_needs_its_own_key_and_the_key_comes_back_from_its_recovery_code",
            origin: "#160, D3, D4",
            layer: "fault",
            run: sealed_label,
        },
        Scenario {
            name: "a_worker_holds_only_what_its_credential_carries_and_loses_it_on_revoke_or_expiry",
            origin: "#121, #120, #161",
            layer: "fault",
            run: workers,
        },
        Scenario {
            name: "a_run_is_its_own_namespace_archived_encrypted_and_removed_without_touching_anything_else",
            origin: "#122, #118",
            layer: "fault",
            run: runs,
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

fn sealed_label(w: &World) -> Res {
    w.set_clock(T0);
    for who in ["alice", "carol"] {
        check(w.admin(&["clear", who, "private"], "").ok, "clear failed")?;
    }
    check(
        w.admin(&["label", "vault", "private"], "").ok,
        "label failed",
    )?;
    let sealed = w.admin(&["seal-label", "private"], "");
    check(sealed.ok, format!("seal-label failed: {}", sealed.stderr))?;
    has(&sealed.stdout, "a key was made", "no label key was made")?;
    check(
        !w.admin(&["seal-label", "public"], "").ok,
        "the lowest label was sealed",
    )?;
    // a member cleared for the label holds its key and reads and writes normally
    w.ok(
        "alice",
        &["post", "vault", "the sealed launch code is hunter2"],
    )?;
    has(
        &w.ok("carol", &["read", "vault", "--peek"])?,
        "hunter2",
        "a member with the key could not read",
    )?;
    // the label key is the only way in: the store key alone shows neither the text nor lets text be written
    let codes = w.admin(&["recovery"], "");
    let label_code = codes
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("label private: "))
        .ok_or("no recovery code for the label key")?
        .trim()
        .to_string();
    let keyfile = key_files(w)
        .into_iter()
        .find(|p| p.to_string_lossy().contains(".label.private"))
        .ok_or("no label key file")?;
    let saved = std::fs::read(&keyfile).map_err(|e| e.to_string())?;
    std::fs::remove_file(&keyfile).map_err(|e| e.to_string())?;
    let blind_run = w.run("carol", &["read", "vault", "--peek"]);
    let blind = format!(
        "ok={} {}{}",
        blind_run.ok, blind_run.stdout, blind_run.stderr
    );
    lacks(
        &blind,
        "hunter2",
        "the text was readable without the label key",
    )?;
    has(&blind, "sealed", "no note that the message is sealed")?;
    has(
        &w.refused(
            "alice",
            &["post", "vault", "plain text under a sealed label"],
        )?,
        "sealed and you hold no key",
        "a write without the label key was not refused",
    )?;
    // the key comes back from its recovery code, and a code for another key does not
    check(
        !w.admin(
            &["recover", "--label", "private", "KLATCH-AAAA-AAAA"],
            "recover",
        )
        .ok,
        "a wrong code was accepted",
    )?;
    let back = w.admin(&["recover", "--label", "private", &label_code], "recover");
    check(back.ok, format!("recover failed: {}", back.stderr))?;
    check(
        std::fs::read(&keyfile).map_err(|e| e.to_string())? == saved,
        "the restored key differs",
    )?;
    has(
        &w.ok("carol", &["read", "vault", "--peek"])?,
        "hunter2",
        "the text did not come back",
    )?;
    // the person administering holds every label key, and the readable export is audited
    has(
        &w.admin(&["export", "vault"], "vault").stdout,
        "hunter2",
        "the export cannot read a sealed label",
    )?;
    Ok(vec![])
}

fn workers(w: &World) -> Res {
    w.set_clock(T0);
    check(
        w.admin(&["label", "vault", "private"], "").ok,
        "label failed",
    )?;
    check(
        w.admin(&["seal-label", "private"], "").ok,
        "seal-label failed",
    )?;
    let bundle = |member: &str| w.home.join(format!("{member}.bundle"));
    let issue = |member: &str, clearance: &str, ttl: &str| {
        let out = bundle(member);
        let o = w.run(
            "launcher",
            &[
                "worker",
                "issue",
                member,
                "--clearance",
                clearance,
                "--ttl",
                ttl,
                "--parent",
                "lead",
                "--out",
                &out.to_string_lossy(),
            ],
        );
        (o, out)
    };
    let (hi, hi_path) = issue("w-hi", "private", "600");
    check(hi.ok, format!("issue failed: {}", hi.stderr))?;
    let (lo, lo_path) = issue("w-lo", "internal", "600");
    check(lo.ok, format!("issue failed: {}", lo.stderr))?;
    // the worker with the clearance and the bundle's label key writes under the sealed label, verified, with no
    // keystore and no typed name
    let post = w.as_worker(
        &hi_path,
        &["post", "vault", "the sealed launch code is hunter2"],
    );
    check(
        post.ok,
        format!("the cleared worker could not post: {}", post.stderr),
    )?;
    let own = w.as_worker(&hi_path, &["read", "vault", "--peek"]);
    check(own.ok, format!("read failed: {}", own.stderr))?;
    // below the room's label the other worker is refused, whatever it holds
    has(
        &w.as_worker(&lo_path, &["read", "vault"]).stderr,
        "above your clearance",
        "a worker below the room was not refused",
    )?;
    // the credential is the only identity: a typed name cannot be added to it
    let mut c = w.as_worker(&lo_path, &["post", "r", "x"]);
    check(c.ok, "the lower worker could not post in an open room")?;
    c = w.admin(&["export", "vault"], "vault");
    has(&c.stdout, "w-hi [", "no verified sender in the export")
        .or_else(|_| has(&c.stdout, "w-hi ", "no verified sender in the export"))?;
    lacks(
        &c.stdout,
        "w-hi (unverified)",
        "a credential holder was marked unverified",
    )?;
    has(
        &c.stdout,
        "hunter2",
        "the export cannot read what the worker sealed",
    )?;
    // the audit names the parent and never the token
    let audit = w.admin(&["audit"], "");
    has(
        &audit.stdout,
        "issue-credential",
        "the issue is not in the audit",
    )?;
    has(
        &audit.stdout,
        "parent=lead",
        "the audit does not name the parent",
    )?;
    lacks(&audit.stdout, "KLATCH-CRED-", "a token reached the audit")?;
    // revocation and expiry stop a worker at once
    check(
        w.run("launcher", &["worker", "revoke", "w-hi"]).ok,
        "revoke failed",
    )?;
    has(
        &w.as_worker(&hi_path, &["post", "vault", "after revoke"])
            .stderr,
        "revoked",
        "a revoked worker still posted",
    )?;
    let (short, short_path) = issue("w-short", "internal", "5");
    check(short.ok, "short issue failed")?;
    check(
        w.as_worker(&short_path, &["post", "r", "in time"]).ok,
        "a fresh credential failed",
    )?;
    w.advance(10);
    has(
        &w.as_worker(&short_path, &["post", "r", "too late"]).stderr,
        "expired",
        "an expired worker still posted",
    )?;
    Ok(vec![])
}

fn runs(w: &World) -> Res {
    w.set_clock(T0);
    for id in ["a", "b"] {
        check(
            w.run("launcher", &["run", "create", id]).ok,
            "run create failed",
        )?;
    }
    w.ok(
        "alice",
        &["post", "run.a.main", "the run a secret is hunter2"],
    )?;
    w.ok("alice", &["post", "run.a.review", "second room of run a"])?;
    w.ok("bob", &["post", "run.b.main", "work for run b"])?;
    w.ok("bob", &["post", "lobby", "an ordinary room"])?;
    let archive = w.home.join("run-a.archive");
    let archive_s = archive.to_string_lossy().into_owned();
    let ended = w.run("launcher", &["run", "end", "a", "--archive", &archive_s]);
    check(ended.ok, format!("run end failed: {}", ended.stderr))?;
    has(
        &ended.stdout,
        "2 room(s)",
        "the run did not say what it removed",
    )?;
    // the rooms are gone and cannot come back; everything else stays
    has(
        &w.refused("alice", &["post", "run.a.main", "late"])?,
        "not open",
        "a worker recreated a torn-down run",
    )?;
    has(
        &w.ok("carol", &["read", "run.b.main", "--peek"])?,
        "work for run b",
        "another run was touched",
    )?;
    has(
        &w.ok("carol", &["read", "lobby", "--peek"])?,
        "ordinary room",
        "an ordinary room was touched",
    )?;
    // the archive is encrypted and still readable by the person at the terminal, audited
    let bytes = std::fs::read(&archive).map_err(|e| e.to_string())?;
    check(
        !bytes.windows(7).any(|x| x == b"hunter2"),
        "the archive holds plaintext",
    )?;
    let shown = w.admin(&["run-archive", &archive_s, "--run", "a"], "a");
    check(shown.ok, format!("run-archive failed: {}", shown.stderr))?;
    has(
        &shown.stdout,
        "hunter2",
        "the archive did not hold what the run held",
    )?;
    has(
        &shown.stdout,
        "run.a.review",
        "the second room is missing from the archive",
    )?;
    has(
        &w.admin(&["audit"], "").stdout,
        "run-archive",
        "reading the archive is not in the audit",
    )?;
    check(
        !w.admin(&["run-archive", &archive_s, "--run", "b"], "b").ok,
        "an archive opened for another run",
    )?;
    Ok(vec![])
}
