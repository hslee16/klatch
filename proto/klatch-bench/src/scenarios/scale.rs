use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "board_with_many_members_and_asks",
            origin: "#262, #245",
            layer: "scale",
            run: board_scale,
        },
        Scenario {
            name: "hook_across_many_rooms",
            origin: "#262",
            layer: "scale",
            run: hook_many_rooms,
        },
        Scenario {
            name: "many_waiters_wake_together",
            origin: "#262, #128",
            layer: "scale",
            run: many_waiters,
        },
        Scenario {
            name: "a_long_history_reads_and_streams_whole",
            origin: "#262",
            layer: "scale",
            run: long_history,
        },
    ]
}

fn board_scale(w: &World) -> Res {
    let members = 100;
    for i in 0..members {
        w.ok(&format!("m{i}"), &["join", "lobby"])?;
    }
    let mut asks = 0;
    for asker in 0..4 {
        let reqs: Vec<String> = (0..100)
            .map(|i| World::post_request(i, "lobby", &format!("seeking: thing {asker}-{i}")))
            .collect();
        all_ok(&w.serve(&format!("asker{asker}"), &reqs)?, 100)?;
        asks += 100;
    }
    let t = Instant::now();
    let board = w.ok("viewer", &["board", "lobby"])?;
    let board_ms = ms(t);
    has(
        &board,
        "and 380 more asks",
        "a long section is cut and counted",
    )?;
    let t = Instant::now();
    let listed = w.ok("viewer", &["asks", "lobby"])?;
    let asks_ms = ms(t);
    check(
        listed.lines().count() == asks,
        format!("asks listed {} of {asks}", listed.lines().count()),
    )?;
    Ok(vec![
        ("members".into(), members as f64),
        ("open_asks".into(), asks as f64),
        ("board_ms".into(), board_ms),
        ("asks_ms".into(), asks_ms),
    ])
}

fn hook_many_rooms(w: &World) -> Res {
    let rooms = 40;
    let names: Vec<String> = (0..rooms).map(|i| format!("room{i}")).collect();
    let joins: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, r)| World::read_request(i, r))
        .collect();
    all_ok(&w.serve("x", &joins)?, rooms)?;
    let posts: Vec<String> = names
        .iter()
        .flat_map(|r| (0..3).map(move |i| (r.clone(), i)))
        .enumerate()
        .map(|(n, (r, i))| World::post_request(n, &r, &format!("hello {i} in {r}")))
        .collect();
    all_ok(&w.serve("y", &posts)?, rooms * 3)?;
    let t = Instant::now();
    let out = w.ok("x", &["hook"])?;
    let hook_ms = ms(t);
    for r in &names {
        has(
            &out,
            &format!("[room {r}]"),
            "every room with news is shown",
        )?;
    }
    Ok(vec![
        ("rooms".into(), rooms as f64),
        ("hook_ms".into(), hook_ms),
        ("hook_bytes".into(), out.len() as f64),
    ])
}

fn many_waiters(w: &World) -> Res {
    let n = 20;
    for i in 0..n {
        w.ok(&format!("w{i}"), &["join", "r"])?;
    }
    let t = Instant::now();
    let waiters: Vec<_> = (0..n)
        .map(|i| {
            w.spawn(&format!("w{i}"), &["wait", "r", "--timeout", "30"])
                .map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    std::thread::sleep(std::time::Duration::from_millis(600));
    w.ok("poster", &["post", "r", "wake everyone"])?;
    let posted = Instant::now();
    let mut failed = Vec::new();
    for c in waiters {
        let o = c.wait_with_output().map_err(|e| e.to_string())?;
        let out = String::from_utf8_lossy(&o.stdout);
        if !o.status.success() || !out.contains("wake everyone") {
            failed.push(format!(
                "{} / {}",
                out.trim(),
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
    }
    check(
        failed.is_empty(),
        format!(
            "{} of {n} waiters did not wake with the message: {failed:?}",
            failed.len()
        ),
    )?;
    Ok(vec![
        ("waiters".into(), n as f64),
        ("wake_ms".into(), ms(posted)),
        ("total_ms".into(), ms(t)),
    ])
}

fn long_history(w: &World) -> Res {
    let total = 3000;
    // a member may post 100 times a minute in one room (R22), so the load goes in batches a minute apart
    w.set_clock(T0);
    for who in 0..3 {
        for batch in 0..10 {
            let reqs: Vec<String> = (0..100)
                .map(|i| World::post_request(i, "r", &format!("message {who}-{}", batch * 100 + i)))
                .collect();
            all_ok(&w.serve(&format!("p{who}"), &reqs)?, 100)?;
            w.advance(61);
        }
    }
    w.ok("reader", &["join", "r"])?;
    let t = Instant::now();
    let read = w.ok("reader", &["read", "r", "--peek"])?;
    let read_ms = ms(t);
    check(
        read.lines().count() == total,
        format!("read returned {} of {total} lines", read.lines().count()),
    )?;
    let t = Instant::now();
    let stream = w.ok("reader", &["stream", "r"])?;
    let stream_ms = ms(t);
    let frames = stream
        .lines()
        .filter(|l| l.contains("\"type\":\"message\""))
        .count();
    check(
        frames == total,
        format!("stream returned {frames} message frames of {total}"),
    )?;
    Ok(vec![
        ("messages".into(), total as f64),
        ("read_ms".into(), read_ms),
        ("stream_ms".into(), stream_ms),
    ])
}
