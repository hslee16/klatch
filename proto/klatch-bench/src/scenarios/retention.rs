use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "old_messages_are_archived_encrypted_by_retention_and_a_restore_brings_them_back",
        origin: "#69, #70, D2, R32",
        layer: "fault",
        run: retention_archive,
    }]
}

fn archive_files(w: &World) -> Vec<std::path::PathBuf> {
    match std::fs::read_dir(w.home.join("archive")) {
        Ok(d) => d.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => vec![],
    }
}

fn retention_archive(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["post", "r", "old message with the code hunter2"])?;
    // no retention yet: nothing may be archived
    let none = w.admin(&["archive", "--room", "r"], "");
    check(!none.ok, "an archive ran with no retention set")?;
    check(archive_files(w).is_empty(), "an archive file appeared")?;
    w.advance(10 * 86_400);
    w.ok("alice", &["post", "r", "recent message"])?;
    check(
        w.admin(&["retention", "r", "7"], "").ok,
        "setting retention failed",
    )?;
    check(
        !w.admin(&["retention", "r", "0"], "").ok,
        "a retention of 0 days was accepted",
    )?;
    let made = w.admin(&["archive", "--room", "r"], "");
    check(made.ok, format!("archive failed: {}", made.stderr))?;
    has(&made.stdout, "archived 1 messages", "wrong count")?;
    let files = archive_files(w);
    check(files.len() == 1, format!("{} archive files", files.len()))?;
    let bytes = std::fs::read(&files[0]).map_err(|e| e.to_string())?;
    check(
        !bytes.windows(7).any(|x| x == b"hunter2"),
        "the archive holds plaintext",
    )?;
    // active reads are unaffected: the recent message is there, the old text is not, the seq stays
    let read = w.ok("bob", &["read", "r", "--peek"])?;
    has(&read, "recent message", "the live message is missing")?;
    lacks(&read, "hunter2", "archived text is still in the room")?;
    let again = w.admin(&["archive", "--room", "r"], "");
    check(
        again.ok,
        format!("a second archive failed: {}", again.stderr),
    )?;
    has(&again.stdout, "nothing is older", "archived twice")?;
    // restore
    let file_s = files[0].to_string_lossy().into_owned();
    let back = w.admin(&["restore-archive", &file_s], "");
    check(back.ok, format!("restore failed: {}", back.stderr))?;
    has(&back.stdout, "restored 1 messages", "wrong restore count")?;
    let export = w.admin(&["export", "r"], "r");
    has(
        &export.stdout,
        "hunter2",
        "the restored message is not back",
    )?;
    check(files[0].exists(), "the archive was removed by a restore")?;
    has(
        &w.admin(&["audit"], "").stdout,
        "restore-archive",
        "the restore is not in the audit",
    )?;
    Ok(vec![])
}
