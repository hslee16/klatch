use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "old_messages_are_archived_encrypted_by_retention_and_a_restore_brings_them_back",
            origin: "#69, #70, D2, R32",
            layer: "fault",
            run: retention_archive,
        },
        Scenario {
            name: "an_ask_with_a_deadline_is_not_reported_stale_before_it_and_shows_how_long_is_left",
            origin: "#306, DESIGN s32 item 44",
            layer: "functional",
            run: ask_deadline_by_clock,
        },
        Scenario {
            name: "a_parked_member_keeps_its_claims_and_its_place_past_the_hide_and_prune_windows",
            origin: "#305, DESIGN s32 item 45",
            layer: "functional",
            run: parked_holds_by_clock,
        },
        Scenario {
            name: "a_task_outlives_its_session_shows_orphaned_with_its_checkpoint_and_can_be_handed_over",
            origin: "#307, DESIGN s32 item 48",
            layer: "functional",
            run: durable_task_by_clock,
        },
    ]
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
    // the derived transcript regenerates identically from the store, one JSON object per line (#71)
    let json_lines = |run: &crate::world::Run| -> Vec<String> {
        run.stdout
            .lines()
            .filter_map(|l| l.find('{').map(|i| l[i..].to_string()))
            .collect()
    };
    let j1 = w.admin(&["export", "r", "--json"], "r");
    let j2 = w.admin(&["export", "r", "--json"], "r");
    let (l1, l2) = (json_lines(&j1), json_lines(&j2));
    check(j1.ok && l1 == l2, "the json export is not repeatable")?;
    check(
        l1.len() == 2 && l1.iter().all(|l| l.contains("\"room\":\"r\"")),
        format!("unexpected json export:\n{}", j1.stdout),
    )?;
    has(
        &j1.stdout,
        "\"seq\":1",
        "seq 1 missing from the json export",
    )?;
    has(
        &w.admin(&["audit"], "").stdout,
        "restore-archive",
        "the restore is not in the audit",
    )?;
    Ok(vec![])
}

// T0 is 2027-01-15 08:00 UTC, so the end of 2027-01-20 is a little over five days away.
fn ask_deadline_by_clock(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok(
        "alice",
        &["post", "lobby", "seeking: a slow review (by 2027-01-20)"],
    )?;
    w.ok("alice", &["post", "lobby", "seeking: a quick thing"])?;
    has(
        &w.ok("bob", &["board", "lobby"])?,
        "due in 5d",
        "the deadline shows on the board",
    )?;
    w.advance(2 * 86_400);
    let told = w.ok("alice", &["hook"])?;
    has(
        &told,
        "your ask #2 has had no claim",
        "the undated ask is reported",
    )?;
    lacks(
        &told,
        "your ask #1 has had no claim",
        "the dated ask waits for its deadline",
    )?;
    w.advance(4 * 86_400);
    has(
        &w.ok("alice", &["hook"])?,
        "your ask #1 has had no claim",
        "past the deadline the dated ask is reported",
    )?;
    has(
        &w.ok("bob", &["asks", "lobby"])?,
        "overdue",
        "the ask shows as overdue",
    )?;
    Ok(vec![])
}

fn stop_hook(w: &World, who: &str, input: &str) -> Res {
    use std::io::Write;
    let mut child = w
        .spawn_piped(who, &["hook", "--event", "Stop"])
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(input.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    check(out.status.success(), "the Stop hook failed")?;
    check(out.stdout.is_empty(), "the Stop hook printed something")?;
    Ok(vec![])
}

fn parked_holds_by_clock(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("bob", &["join", "lobby"])?;
    w.ok("alice", &["claim", "lobby", "build.log", "--ttl", "600"])?;
    w.ok(
        "alice",
        &["post", "lobby", "parked: nightly build, back by 3d"],
    )?;
    w.advance(2 * 86_400);
    // alice has been silent for two days: past the ten minute hide window and the one day prune window
    w.ok("bob", &["hook", "--event", "SessionStart", "--no-join"])?;
    let who = w.ok("bob", &["who", "lobby"])?;
    has(&who, "alice", "a parked member is still listed")?;
    has(
        &who,
        "parked, back in 1d: nightly build",
        "with its return time",
    )?;
    has(
        &w.ok("bob", &["claims", "lobby"])?,
        "build.log",
        "its claim holds past its lease",
    )?;
    w.advance(86_400 + 60);
    w.ok("bob", &["hook", "--event", "SessionStart", "--no-join"])?;
    lacks(
        &w.ok("bob", &["claims", "lobby"])?,
        "build.log",
        "after the return time the claim has run out",
    )?;
    lacks(
        &w.ok("bob", &["who", "lobby", "--all"])?,
        "alice",
        "and the silent member is pruned",
    )?;
    // the Stop hook records background work, prints nothing, and a post ends the park
    w.ok("carol", &["join", "lobby"])?;
    stop_hook(
        w,
        "carol",
        r#"{"background_tasks":[{"id":"t1"}],"session_crons":[]}"#,
    )?;
    has(
        &w.ok("bob", &["who", "lobby"])?,
        "parked, back in 1h: 1 background task",
        "background work parks the member",
    )?;
    w.ok("carol", &["post", "lobby", "back"])?;
    lacks(
        &w.ok("bob", &["who", "lobby"])?,
        "parked",
        "a post ends the park",
    )?;
    Ok(vec![])
}

fn durable_task_by_clock(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok(
        "alice",
        &[
            "post",
            "lobby",
            "working: nightly build: watching the pipeline",
        ],
    )?;
    w.ok(
        "alice",
        &[
            "post",
            "lobby",
            "checkpoint: build 41 green, resume at step 4",
        ],
    )?;
    w.advance(3 * 86_400);
    // alice's session is long gone; a different helper finds the task and where it stood
    w.ok("bob", &["join", "lobby"])?;
    let board = w.ok("bob", &["board", "lobby"])?;
    has(&board, "tasks:", "the board lists tasks")?;
    has(
        &board,
        "alice: nightly build (orphaned",
        "orphaned, not gone",
    )?;
    let full = w.ok("bob", &["tasks", "lobby"])?;
    has(
        &full,
        "resume at step 4",
        "the checkpoint is readable in full",
    )?;
    w.ok("alice", &["post", "lobby", "handoff: nightly build to bob"])?;
    let mine = w.ok("bob", &["tasks", "lobby"])?;
    has(&mine, "bob: nightly build", "bob owns it after the handoff")?;
    has(
        &mine,
        "handed over by alice",
        "and it says who handed it over",
    )?;
    w.ok("bob", &["post", "lobby", "finished: nightly build"])?;
    check(
        w.ok("bob", &["tasks", "lobby"])?.trim().is_empty(),
        "a finished task is gone",
    )?;
    Ok(vec![])
}
