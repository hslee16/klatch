//! Big-store scenarios (the bench expansion after the retention, parking, receipt and budget epics): the features whose
//! cost grows with the size of the store, or that have a crash or concurrency window. Each asserts correctness first and
//! records timings; the hard ceilings are generous on purpose (a breach means something is wrong, not slow).

use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "archiving_fifty_thousand_messages_leaves_the_live_store_usable_and_restores_whole",
            origin: "#487, DESIGN 55, DESIGN 29.6",
            layer: "scale",
            run: archive_fifty_thousand,
        },
        Scenario {
            name: "sixteen_processes_archiving_and_posting_at_once_lose_and_duplicate_nothing",
            origin: "#487, DESIGN 55",
            layer: "fault",
            run: archive_and_post_race,
        },
        Scenario {
            name: "paging_a_hundred_thousand_message_room_costs_the_same_at_the_head_and_the_tail",
            origin: "#518, DESIGN 60",
            layer: "scale",
            run: history_paging,
        },
        Scenario {
            name: "the_board_renders_five_hundred_parked_members_and_two_hundred_tasks_in_bounded_text",
            origin: "#308, DESIGN 29.5, DESIGN 29.7",
            layer: "scale",
            run: board_parked_and_tasks,
        },
        Scenario {
            name: "fifty_members_renew_their_claims_by_activity_and_all_lapse_together_when_silent",
            origin: "#494, DESIGN 52",
            layer: "scale",
            run: claim_renewal_fifty,
        },
        Scenario {
            name: "a_hundred_rooms_with_news_still_print_a_digest_inside_the_byte_budget",
            origin: "#549, DESIGN 50, #557",
            layer: "scale",
            run: hook_hundred_rooms,
        },
        Scenario {
            name: "a_receipt_for_a_room_of_three_hundred_members_answers_fast_and_prints_no_text",
            origin: "#546, DESIGN 61",
            layer: "scale",
            run: receipt_three_hundred,
        },
    ]
}

/// Seed `rounds` x `posters` x 100 messages into `room`. A member may post 100 a minute per room (R22), so each round
/// is a minute (and a second) after the last; the clock ends after the last round.
fn seed(w: &World, room: &str, posters: usize, rounds: usize) -> Result<usize, String> {
    for round in 0..rounds {
        for p in 0..posters {
            let reqs: Vec<String> = (0..100)
                .map(|i| {
                    World::post_request(
                        i,
                        room,
                        &format!("seed {p}-{round}-{i} some ordinary text of a plausible length for a chat line"),
                    )
                })
                .collect();
            all_ok(&w.serve(&format!("p{p}"), &reqs)?, 100)?;
        }
        w.advance(61);
    }
    Ok(posters * rounds * 100)
}

fn export_lines(w: &World, room: &str) -> Result<Vec<serde_json::Value>, String> {
    let run = w.admin(&["export", room, "--json"], room);
    check(run.ok, format!("export failed: {}", run.stderr))?;
    Ok(run
        .stdout
        .lines()
        .filter_map(|l| l.find('{').map(|i| l[i..].to_string()))
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect())
}

fn archive_files(w: &World) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(w.home.join("archive"))
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default()
}

/// The live post ceiling while an archive runs: a writer waiting on the archive's lock this long is a finding.
const LIVE_POST_CEILING_MS: f64 = 10_000.0;

fn archive_fifty_thousand(w: &World) -> Res {
    w.set_clock(T0);
    let total = seed(w, "r", 10, 50)?;
    w.advance(2 * 86_400);
    check(
        w.admin(&["retention", "r", "1"], "").ok,
        "setting retention failed",
    )?;
    let t = Instant::now();
    let mut child = w
        .spawn_admin(&["archive", "--room", "r"])
        .map_err(|e| e.to_string())?;
    // while it runs, the live store must keep answering: posts from fresh members and reads (under the R22 cap)
    let (mut live_ops, mut live_max, mut failures) = (0, 0f64, Vec::new());
    while child.try_wait().map_err(|e| e.to_string())?.is_none() && live_ops < 90 {
        let name = format!("live{live_ops}");
        let s = Instant::now();
        let post = w.run(&name, &["post", "r", "posted during the archive"]);
        live_max = live_max.max(ms(s));
        if !post.ok {
            failures.push(post.stderr.trim().to_string());
        }
        let read = w.run("reader", &["history", "r", "--last", "3"]);
        if !read.ok {
            failures.push(read.stderr.trim().to_string());
        }
        live_ops += 1;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let archive_ms = ms(t);
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    check(out.status.success(), format!("archive failed: {said}"))?;
    has(
        &said,
        &format!("archived {total} messages"),
        "every old message moved",
    )?;
    check(
        failures.is_empty(),
        format!("the live store refused work during the archive: {failures:?}"),
    )?;
    check(
        live_max < LIVE_POST_CEILING_MS,
        format!(
            "a live post waited {live_max:.0} ms behind the archive (ceiling {LIVE_POST_CEILING_MS} ms)"
        ),
    )?;
    let files = archive_files(w);
    check(files.len() == 1, format!("{} archive files", files.len()))?;
    // the live store: tombstones keep the seq, new messages continue it
    let after = w.ok("reader", &["history", "r", "--since", "0", "--last", "2"])?;
    let post = w.ok("late", &["post", "r", "after the archive"])?;
    has(&post, "posted", "a post after the archive")?;
    let newest = w.ok("reader", &["history", "r", "--last", "1"])?;
    has(
        &newest,
        "after the archive",
        "the newest message is the new one",
    )?;
    check(
        !after.contains("seed 0-0-0") || after.contains("archived"),
        format!("an archived message is still readable in full:\n{after}"),
    )?;
    let t = Instant::now();
    let doctor = w.run("reader", &["doctor"]);
    let doctor_ms = ms(t);
    no_panic(&doctor.stdout, "doctor on a big store")?;
    let t = Instant::now();
    let listing = w.admin(&["archives"], "");
    let archives_ms = ms(t);
    check(listing.ok, format!("archives failed: {}", listing.stderr))?;
    has(
        &listing.stdout,
        &format!("{total} messages"),
        "the listing counts the archive",
    )?;
    let file_s = files[0].to_string_lossy().into_owned();
    let t = Instant::now();
    let back = w.admin(&["restore-archive", &file_s], "");
    let restore_ms = ms(t);
    check(back.ok, format!("restore failed: {}", back.stderr))?;
    has(
        &back.stdout,
        &format!("restored {total} messages"),
        "every message came back",
    )?;
    Ok(vec![
        ("messages".into(), total as f64),
        ("archive_ms".into(), archive_ms),
        ("live_post_max_ms".into(), live_max),
        ("live_ops".into(), live_ops as f64),
        ("doctor_ms".into(), doctor_ms),
        ("archives_ms".into(), archives_ms),
        ("restore_ms".into(), restore_ms),
    ])
}

fn archive_and_post_race(w: &World) -> Res {
    w.set_clock(T0);
    let old = seed(w, "r", 5, 2)?;
    w.advance(10 * 86_400);
    check(
        w.admin(&["retention", "r", "7"], "").ok,
        "setting retention failed",
    )?;
    let (archivers, posters, per) = (8, 8, 10);
    let mut archive_children = Vec::new();
    let mut post_children = Vec::new();
    for _ in 0..archivers {
        archive_children.push(
            w.spawn_admin(&["archive", "--room", "r"])
                .map_err(|e| e.to_string())?,
        );
    }
    for p in 0..posters {
        use std::io::Write;
        let mut c = w
            .spawn_piped(&format!("racer{p}"), &["serve", "--stdio"])
            .map_err(|e| e.to_string())?;
        let body: String = (0..per)
            .map(|i| {
                format!(
                    "{}\n",
                    World::post_request(i, "r", &format!("racing {p}-{i}"))
                )
            })
            .collect();
        c.stdin
            .take()
            .ok_or("no stdin")?
            .write_all(body.as_bytes())
            .map_err(|e| e.to_string())?;
        post_children.push(c);
    }
    let mut posted = 0;
    for c in post_children {
        let o = c.wait_with_output().map_err(|e| e.to_string())?;
        posted += String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| l.contains("\"ok\":true"))
            .count();
    }
    check(
        posted == posters * per,
        format!("{posted} of {} racing posts were accepted", posters * per),
    )?;
    let (mut won, mut collided, mut other) = (0, 0, Vec::new());
    for c in archive_children {
        let o = c.wait_with_output().map_err(|e| e.to_string())?;
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        no_panic(&said, "a racing archive")?;
        if o.status.success() {
            won += 1;
        } else if said.contains("already exists") {
            collided += 1;
        } else {
            other.push(said.trim().to_string());
        }
    }
    check(
        other.is_empty(),
        format!("an archive failed in an unplanned way: {other:?}"),
    )?;
    check(won >= 1, "no racing archive succeeded (#598)")?;
    // nothing lost, nothing doubled: bring every archive back, then every message is present once and seq is dense
    let mut restored = Vec::new();
    for f in archive_files(w) {
        if f.extension().is_some_and(|e| e == "db") {
            let r = w.admin(&["restore-archive", &f.to_string_lossy()], "");
            check(r.ok, format!("restore failed: {}", r.stderr))?;
            restored.push(format!("{}: {}", f.display(), r.stdout.trim()));
        }
    }
    let listing = w.admin(&["archives"], "");
    let lines = export_lines(w, "r")?;
    let total = old + posters * per;
    let seqs: Vec<i64> = lines.iter().filter_map(|l| l["seq"].as_i64()).collect();
    check(
        seqs == (1..=total as i64).collect::<Vec<_>>(),
        format!(
            "seq is not dense 1..{total}: {} rows, first {:?}",
            seqs.len(),
            &seqs[..seqs.len().min(5)]
        ),
    )?;
    let bodies: Vec<&str> = lines.iter().filter_map(|l| l["body"].as_str()).collect();
    for p in 0..posters {
        for i in 0..per {
            let needle = format!("racing {p}-{i}");
            let n = bodies.iter().filter(|b| **b == needle).count();
            check(n == 1, format!("{needle:?} is present {n} times"))?;
        }
    }
    let tombstones = bodies.iter().filter(|b| b.starts_with("[archived")).count();
    check(
        tombstones == 0,
        format!(
            "{tombstones} messages are still tombstones after every restore (won {won}, collided {collided}); restored {restored:?}; archives:\n{}{}",
            listing.stdout, listing.stderr
        ),
    )?;
    Ok(vec![
        ("archive_won".into(), won as f64),
        ("archive_collided".into(), collided as f64),
    ])
}

fn history_paging(w: &World) -> Res {
    let total = seed(w, "r", 10, 100)?;
    let page_ms = |args: &[&str], want: &str| -> Result<f64, String> {
        let t = Instant::now();
        let out = w.ok("reader", args)?;
        let took = ms(t);
        has(&out, want, "the page")?;
        Ok(took)
    };
    let head = page_ms(&["history", "r", "--since", "0"], "[1 ")?;
    let middle = page_ms(
        &["history", "r", "--since", &(total / 2).to_string()],
        &format!("[{} ", total / 2 + 1),
    )?;
    let tail = page_ms(
        &["history", "r", "--since", &(total - 20).to_string()],
        &format!("[{total} "),
    )?;
    let newest = page_ms(
        &["history", "r", "--last", "200"],
        &format!("[{} ", total - 199),
    )?;
    // the cost must not grow with the position: the tail page may not take many times the head page
    let worst = middle.max(tail).max(newest);
    check(
        worst < head * 8.0 + 200.0,
        format!(
            "paging cost grows with the room: head {head:.0} ms, middle {middle:.0}, tail {tail:.0}, newest {newest:.0}"
        ),
    )?;
    Ok(vec![
        ("messages".into(), total as f64),
        ("head_ms".into(), head),
        ("middle_ms".into(), middle),
        ("tail_ms".into(), tail),
        ("newest200_ms".into(), newest),
    ])
}

fn board_parked_and_tasks(w: &World) -> Res {
    w.set_clock(T0);
    let (parked, tasks) = (500, 200);
    for i in 0..parked {
        w.ok(
            &format!("park{i}"),
            &["post", "lobby", &format!("parked: build {i}, back by 3h")],
        )?;
    }
    for i in 0..tasks {
        w.ok(
            &format!("task{i}"),
            &[
                "post",
                "lobby",
                &format!("working: job {i}: step one of many"),
            ],
        )?;
    }
    let t = Instant::now();
    let board = w.ok("viewer", &["board", "lobby"])?;
    let board_ms = ms(t);
    let t = Instant::now();
    let task_list = w.ok("viewer", &["tasks", "lobby"])?;
    let tasks_ms = ms(t);
    let t = Instant::now();
    let who = w.ok("viewer", &["who", "lobby"])?;
    let who_ms = ms(t);
    check(
        board.len() < 12_000,
        format!(
            "the board grew to {} bytes with {parked} parked members and {tasks} tasks",
            board.len()
        ),
    )?;
    check(
        task_list.lines().count() >= tasks.min(1),
        "the task list is empty",
    )?;
    has(&who, "park0", "a parked member is listed")?;
    Ok(vec![
        ("parked".into(), parked as f64),
        ("tasks".into(), tasks as f64),
        ("board_ms".into(), board_ms),
        ("board_bytes".into(), board.len() as f64),
        ("tasks_ms".into(), tasks_ms),
        ("who_ms".into(), who_ms),
    ])
}

fn claim_renewal_fifty(w: &World) -> Res {
    let (members, per) = (50, 4);
    w.set_clock(T0);
    for m in 0..members {
        w.ok(&format!("m{m}"), &["join", "r"])?;
        for k in 0..per {
            w.ok(
                &format!("m{m}"),
                &["claim", "r", &format!("m{m}/f{k}"), "--ttl", "600"],
            )?;
        }
    }
    let held = |w: &World| -> Result<usize, String> {
        Ok(w.ok("viewer", &["claims", "r"])?
            .lines()
            .filter(|l| l.contains("left"))
            .count())
    };
    let t = Instant::now();
    check(held(w)? == members * per, "not every claim was taken")?;
    let list_ms = ms(t);
    // every member acts at 500 s and again at 1000 s: without renewal their 600 s claims would lapse at 600 s
    for step in 0..2 {
        w.advance(500);
        for m in 0..members {
            w.ok(
                &format!("m{m}"),
                &["post", "r", &format!("still here {step}")],
            )?;
        }
        let n = held(w)?;
        check(
            n == members * per,
            format!(
                "{n} of {} claims held after activity at step {step}",
                members * per
            ),
        )?;
    }
    // then silence past the claim's own ttl: every one lapses
    w.advance(700);
    w.ok("viewer", &["hook", "--event", "SessionStart", "--no-join"])?;
    let n = held(w)?;
    check(
        n == 0,
        format!("{n} claims outlived their ttl after silence"),
    )?;
    Ok(vec![
        ("claims".into(), (members * per) as f64),
        ("claims_list_ms".into(), list_ms),
    ])
}

/// Posts three long lines into each room of `names`, as member `y`.
fn flood(w: &World, names: &[String]) -> Result<(), String> {
    let long = "a long and wordy status line that a busy agent might well post ".repeat(3);
    let posts: Vec<String> = names
        .iter()
        .flat_map(|r| (0..3).map(move |i| (r.clone(), i)))
        .enumerate()
        .map(|(n, (r, i))| World::post_request(n, &r, &format!("{long} {i} in {r}")))
        .collect();
    all_ok(&w.serve("y", &posts)?, names.len() * 3)
}

/// Forty rooms with news fit the digest budget; 100 do not, and the rooms that do not fit collapse into one counted
/// line, so the whole output stays inside the hook budget (#599).
fn hook_hundred_rooms(w: &World) -> Res {
    let rooms = 100;
    let names: Vec<String> = (0..rooms).map(|i| format!("room{i}")).collect();
    let joins: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, r)| World::read_request(i, r))
        .collect();
    all_ok(&w.serve("x", &joins)?, rooms)?;
    all_ok(&w.serve("x2", &joins)?, rooms)?;
    flood(w, &names[..40])?;
    let out40 = hook_event(w, "x2", "UserPromptSubmit", &[])?;
    check(
        out40.len() < 9_000,
        format!(
            "the digest is {} bytes across 40 rooms with news",
            out40.len()
        ),
    )?;
    has(
        &out40,
        "more messages not shown",
        "the cut messages are not counted",
    )?;
    flood(w, &names[40..])?;
    let t = Instant::now();
    let out = hook_event(w, "x", "UserPromptSubmit", &[])?;
    let hook_ms = ms(t);
    check(out.contains("[room "), "no room was shown at all")?;
    check(
        // the real budget: HOOK_MAX_BYTES in klatch/src/domain/budget.rs
        out.len() <= 9_400,
        format!(
            "the digest is {} bytes across {rooms} rooms with news",
            out.len()
        ),
    )?;
    has(
        &out,
        "more rooms have news",
        "the rooms that did not fit are not counted",
    )?;
    Ok(vec![
        ("rooms".into(), rooms as f64),
        ("hook_ms".into(), hook_ms),
        ("hook40_bytes".into(), out40.len() as f64),
        ("hook_bytes".into(), out.len() as f64),
    ])
}

/// Run `klatch hook --event <event>` with the event JSON on standard input, as Claude Code does.
fn hook_event(w: &World, who: &str, event: &str, extra: &[&str]) -> Result<String, String> {
    use std::io::Write;
    let mut args = vec!["hook", "--event", event];
    args.extend_from_slice(extra);
    let mut child = w.spawn_piped(who, &args).map_err(|e| e.to_string())?;
    let input = format!(r#"{{"session_id":"s","hook_event_name":"{event}"}}"#);
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(input.as_bytes())
        .map_err(|e| e.to_string())?;
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    check(o.status.success(), format!("the {event} hook failed"))?;
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn receipt_three_hundred(w: &World) -> Res {
    w.set_clock(T0);
    let members = 300;
    for m in 0..members {
        w.ok(&format!("m{m}"), &["join", "r"])?;
    }
    w.advance(5);
    w.ok("sender", &["post", "r", "secret plan, please look"])?;
    w.advance(30);
    for m in 0..50 {
        w.ok(&format!("m{m}"), &["read", "r", "--peek"])?;
    }
    let t = Instant::now();
    let out = w.ok("sender", &["receipt", "r", "1"])?;
    let receipt_ms = ms(t);
    lacks(&out, "secret plan", "the receipt printed message text")?;
    has(&out, "m0: seen", "a reader is reported")?;
    has(
        &out,
        "not seen since the post",
        "a silent member is reported",
    )?;
    Ok(vec![
        ("members".into(), members as f64),
        ("receipt_ms".into(), receipt_ms),
        ("receipt_bytes".into(), out.len() as f64),
    ])
}
