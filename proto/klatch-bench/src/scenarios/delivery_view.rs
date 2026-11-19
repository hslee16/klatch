use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "a_narrowed_view_shows_own_scope_addressed_and_broadcast_news_and_read_shows_all",
        origin: "#186, DESIGN 24.1 decision 5",
        layer: "fault",
        run: delivery_view,
    },
    Scenario {
        name: "a_lobby_shared_by_many_repositories_counts_what_a_narrowed_view_hides",
        origin: "#673, #635",
        layer: "scale",
        run: many_repository_lobby,
    }]
}

fn delivery_view(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("listener", &["--member-scope", "team:a", "join", "lobby"])?;
    w.ok("mate", &["--member-scope", "team:a", "join", "lobby"])?;
    w.ok("other", &["--member-scope", "team:b", "join", "lobby"])?;
    has(
        &w.ok("listener", &["view"])?,
        "view: all",
        "the default view",
    )?;
    has(
        &w.ok("listener", &["view", "scope"])?,
        "view: scope",
        "narrowing",
    )?;
    w.refused("listener", &["view", "nonsense"])?;
    w.advance(5);
    w.ok("other", &["post", "lobby", "chatter from the other team"])?;
    w.advance(5);
    w.ok("mate", &["post", "lobby", "note from a teammate"])?;
    w.advance(5);
    w.ok("other", &["post", "lobby", "@listener this one is for you"])?;
    w.advance(5);
    w.ok("other", &["post", "lobby", "@all everyone please pull"])?;
    let seen = w.ok("listener", &["hook", "--event", "UserPromptSubmit"])?;
    has(&seen, "note from a teammate", "own scope")?;
    has(&seen, "this one is for you", "addressed")?;
    has(&seen, "everyone please pull", "broadcast")?;
    lacks(
        &seen,
        "chatter from the other team",
        "other scope, not addressed",
    )?;
    // a view never narrows an explicit read
    let read = w.ok("listener", &["read", "lobby", "--peek"])?;
    has(&read, "chatter from the other team", "read shows all")?;
    Ok(vec![])
}

/// The number in "N message(s) outside your view in room R, newest #SEQ", if the text has the line.
fn outside_count(text: &str) -> Option<usize> {
    text.split("outside your view").next().and_then(|head| {
        head.trim_end()
            .trim_end_matches("messages")
            .trim_end_matches("message")
            .trim()
            .rsplit(' ')
            .next()
            .and_then(|n| n.parse().ok())
    })
}

/// A lobby shared by many repositories (#673): members in different scopes post to one room. A listener on the `all`
/// view and one on `scope`, both in repository 0, are compared on hook bytes, hook time and wake time, and the count
/// the narrowed listener is told is hidden is checked against what the room holds from the other repositories.
fn many_repository_lobby(w: &World) -> Res {
    let repos = 12;
    let per_repo = 4;
    let each = 10;
    w.set_clock(T0);
    for r in 0..repos {
        for m in 0..per_repo {
            let scope = format!("git:repo{r}");
            w.ok(
                &format!("r{r}m{m}"),
                &["--member-scope", &scope, "join", "lobby"],
            )?;
        }
    }
    for who in ["all-view", "scoped"] {
        w.ok(who, &["--member-scope", "git:repo0", "join", "lobby"])?;
    }
    w.ok("scoped", &["view", "scope"])?;
    for r in 0..repos {
        for m in 0..per_repo {
            let reqs: Vec<String> = (0..each)
                .map(|i| World::post_request(i, "lobby", &format!("repo {r} member {m} note {i}")))
                .collect();
            all_ok(&w.serve(&format!("r{r}m{m}"), &reqs)?, each)?;
        }
        w.advance(61);
    }
    let hidden_expected = (repos - 1) * per_repo * each;
    let visible_expected = per_repo * each;
    // the wake pointer first: it reads the same unread messages the hook is about to show
    let wake_ms = |who: &str| -> Result<(f64, String), String> {
        let started = Instant::now();
        let o = w
            .spawn(who, &["wake", "--timeout", "30"])
            .map_err(|e| e.to_string())?
            .wait_with_output()
            .map_err(|e| e.to_string())?;
        let took = ms(started);
        check(o.status.code() == Some(2), format!("{who} did not wake"))?;
        Ok((took, String::from_utf8_lossy(&o.stderr).into_owned()))
    };
    let (wake_all_ms, wake_all) = wake_ms("all-view")?;
    let (wake_scope_ms, wake_scope) = wake_ms("scoped")?;
    lacks(&wake_all, "outside your view", "the all view has nothing outside")?;
    let wake_hidden = outside_count(&wake_scope).ok_or("the wake pointer lacks the hidden count")?;
    check(
        wake_hidden == hidden_expected,
        format!("wake says {wake_hidden} hidden, the room holds {hidden_expected}"),
    )?;
    let t = Instant::now();
    let all_out = w.ok("all-view", &["hook", "--event", "UserPromptSubmit"])?;
    let hook_all_ms = ms(t);
    let t = Instant::now();
    let scope_out = w.ok("scoped", &["hook", "--event", "UserPromptSubmit"])?;
    let hook_scope_ms = ms(t);
    let hook_hidden = outside_count(&scope_out).ok_or("the hook lacks the hidden count")?;
    check(
        hook_hidden == hidden_expected,
        format!("the hook says {hook_hidden} hidden, the room holds {hidden_expected}"),
    )?;
    lacks(&all_out, "outside your view", "the all view has nothing outside")?;
    lacks(&scope_out, "repo 5 member", "the pointer carried a message")?;
    // Both digests sit at the byte budget (#549), so scope does not print fewer bytes; it spends them on the member's
    // own repository instead of the newest messages of every repository.
    let shown = scope_out.matches("repo 0 member").count();
    let shown_all = all_out.matches("repo 0 member").count();
    check(
        shown > 0 && shown <= visible_expected && shown >= shown_all,
        format!(
            "scope showed {shown} own-repository messages, all showed {shown_all}, the room holds {visible_expected}"
        ),
    )?;
    Ok(vec![
        ("members".into(), (repos * per_repo + 2) as f64),
        ("hidden".into(), hidden_expected as f64),
        ("hook_bytes_all".into(), all_out.len() as f64),
        ("hook_bytes_scope".into(), scope_out.len() as f64),
        ("hook_all_ms".into(), hook_all_ms),
        ("hook_scope_ms".into(), hook_scope_ms),
        ("wake_all_ms".into(), wake_all_ms),
        ("wake_scope_ms".into(), wake_scope_ms),
    ])
}
