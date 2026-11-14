use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "deadlock_two_and_three_parties",
            origin: "#246, run 15",
            layer: "functional",
            run: deadlock,
        },
        Scenario {
            name: "many_claimants_one_wins",
            origin: "#240, #262",
            layer: "scale",
            run: claim_contention,
        },
        Scenario {
            name: "a_session_that_ends_frees_its_claims_at_once",
            origin: "#277, #258",
            layer: "fault",
            run: ended_holder,
        },
        Scenario {
            name: "a_dead_holders_claim_lapses_and_another_takes_it",
            origin: "#233, #59, #262",
            layer: "fault",
            run: dead_holder,
        },
        Scenario {
            name: "killed_claimants_leave_the_store_usable",
            origin: "#262",
            layer: "fault",
            run: killed_claimants,
        },
        Scenario {
            name: "the_deadlock_wait_window_closes",
            origin: "#246, #292",
            layer: "fault",
            run: deadlock_window,
        },
        Scenario {
            name: "sessions_in_different_repositories_do_not_collide_on_claims",
            origin: "#179, #297",
            layer: "functional",
            run: repo_scopes,
        },
        Scenario {
            name: "worktrees_share_their_repositorys_scope",
            origin: "#179",
            layer: "functional",
            run: worktree_scope,
        },
        Scenario {
            name: "a_non_coding_member_declares_its_own_scope",
            origin: "#297, #179",
            layer: "functional",
            run: bot_scope,
        },
    ]
}

fn git(dir: &std::path::Path, args: &[&str]) -> Result<(), String> {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "bench")
        .env("GIT_AUTHOR_EMAIL", "bench@example.com")
        .env("GIT_COMMITTER_NAME", "bench")
        .env("GIT_COMMITTER_EMAIL", "bench@example.com")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    check(
        o.status.success(),
        format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

fn repo(w: &World, name: &str) -> Result<std::path::PathBuf, String> {
    let dir = w.home.join(format!("repos-{name}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dir = std::fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    git(&dir, &["init", "-q"])?;
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "start"])?;
    Ok(dir)
}

fn deadlock(w: &World) -> Res {
    w.ok("alice", &["claim", "r", "X.py"])?;
    w.ok("bob", &["claim", "r", "Y.py"])?;
    let first = w.refused("alice", &["claim", "r", "Y.py"])?;
    lacks(&first, "deadlock", "no cycle yet")?;
    let second = w.refused("bob", &["claim", "r", "X.py"])?;
    has(&second, "deadlock", "the closing attempt is a deadlock")?;
    has(&second, "you yield", "and says who yields")?;
    // Three parties.
    w.ok("a", &["claim", "r3", "X"])?;
    w.ok("b", &["claim", "r3", "Y"])?;
    w.ok("c", &["claim", "r3", "Z"])?;
    lacks(
        &w.refused("a", &["claim", "r3", "Y"])?,
        "deadlock",
        "a to b",
    )?;
    lacks(
        &w.refused("b", &["claim", "r3", "Z"])?,
        "deadlock",
        "b to c",
    )?;
    has(
        &w.refused("c", &["claim", "r3", "X"])?,
        "deadlock",
        "c closes the cycle",
    )?;
    Ok(vec![])
}

fn claim_contention(w: &World) -> Res {
    let n = 20;
    let t = Instant::now();
    let children: Vec<_> = (0..n)
        .map(|i| {
            w.spawn(&format!("c{i}"), &["claim", "r", "ask:1"])
                .map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    let (mut won, mut refused) = (0, 0);
    for c in children {
        let o = c.wait_with_output().map_err(|e| e.to_string())?;
        if o.status.success() {
            won += 1;
        } else if String::from_utf8_lossy(&o.stderr).contains("is claimed by") {
            refused += 1;
        }
    }
    check(
        won == 1 && refused == n - 1,
        format!("{won} won and {refused} were refused out of {n}"),
    )?;
    Ok(vec![
        ("claimants".into(), n as f64),
        ("claim_ms".into(), ms(t)),
    ])
}

fn dead_holder(w: &World) -> Res {
    // The holder's process is gone (each CLI call exits), but its claim persists until its lease runs out.
    w.set_clock(T0);
    w.ok("holder", &["claim", "r", "ask:1", "--ttl", "300"])?;
    w.advance(299);
    has(
        &w.refused("other", &["claim", "r", "ask:1"])?,
        "holder",
        "the lease is honoured while it lasts",
    )?;
    w.advance(2);
    w.ok("other", &["claim", "r", "ask:1"])?;
    Ok(vec![])
}

fn ended_holder(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("asker", &["join", "r"])?;
    w.ok("asker", &["post", "r", "seeking: a thing"])?;
    w.ok("holder", &["claim", "r", "ask:1"])?;
    w.ok("holder", &["claim", "r", "src/a.rs"])?;
    has(
        &w.refused("other", &["claim", "r", "src/a.rs"])?,
        "holder",
        "the claim is honoured while the holder works",
    )?;
    w.ok("holder", &["hook", "--event", "SessionEnd"])?;
    w.ok("other", &["claim", "r", "src/a.rs"])?;
    w.ok("other", &["claim", "r", "ask:1"])?;
    Ok(vec![])
}

fn killed_claimants(w: &World) -> Res {
    let n = 30;
    let mut children = Vec::new();
    for i in 0..n {
        children.push(
            w.spawn(&format!("c{i}"), &["claim", "r", &format!("path{i}")])
                .map_err(|e| e.to_string())?,
        );
    }
    for (i, c) in children.iter_mut().enumerate() {
        if i % 2 == 0 {
            let _ = c.kill();
        }
    }
    for mut c in children {
        let _ = c.wait();
    }
    // Whatever was killed, the store must still work and hold no duplicate claim.
    let claims = w.ok("viewer", &["claims", "r"])?;
    let mut paths: Vec<&str> = claims
        .lines()
        .filter_map(|l| l.split('\t').next())
        .collect();
    let before = paths.len();
    paths.sort_unstable();
    paths.dedup();
    check(
        paths.len() == before,
        format!("a claim appears twice:\n{claims}"),
    )?;
    w.ok("viewer", &["post", "r", "still alive"])?;
    has(
        &w.ok("other", &["read", "r"])?,
        "still alive",
        "the store still works",
    )?;
    Ok(vec![("claims_kept".into(), before as f64)])
}

fn deadlock_window(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("a", &["claim", "r", "X"])?;
    w.ok("b", &["claim", "r", "Y"])?;
    lacks(
        &w.refused("a", &["claim", "r", "Y"])?,
        "deadlock",
        "no cycle yet",
    )?;
    w.advance(60);
    has(
        &w.refused("b", &["claim", "r", "X"])?,
        "deadlock",
        "inside the window the closing attempt is a deadlock",
    )?;
    // A wait older than two minutes is no longer a wait.
    w.ok("c", &["claim", "r2", "X"])?;
    w.ok("d", &["claim", "r2", "Y"])?;
    lacks(
        &w.refused("c", &["claim", "r2", "Y"])?,
        "deadlock",
        "no cycle yet in r2",
    )?;
    w.advance(121);
    lacks(
        &w.refused("d", &["claim", "r2", "X"])?,
        "deadlock",
        "the first wait has expired",
    )?;
    Ok(vec![])
}

fn repo_scopes(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let (a, b) = (repo(w, "alpha")?, repo(w, "beta")?);
    let s1 = h.start_in("aaaa1111-one", "one", &a)?;
    let s2 = h.start_in("bbbb2222-two", "two", &b)?;
    let s3 = h.start_in("cccc3333-three", "three", &a)?;
    for s in [&s1, &s2, &s3] {
        s.hook("SessionStart")?;
    }
    let ok =
        |s: &crate::Session, args: &[&str]| -> Result<(bool, String), String> { s.klatch(args) };
    check(
        ok(&s1, &["claim", "lobby", "src/lib.rs"])?.0,
        "the first repository claims src/lib.rs",
    )?;
    check(
        ok(&s2, &["claim", "lobby", "src/lib.rs"])?.0,
        "another repository may claim the same path",
    )?;
    let (taken, why) = ok(&s3, &["claim", "lobby", "src/lib.rs"])?;
    check(
        !taken && why.contains("claimed by one"),
        format!("a second session in the first repository must conflict: {why}"),
    )?;
    // A room-wide name is shared across repositories.
    check(ok(&s1, &["claim", "lobby", "ask:1"])?.0, "ask:1 is claimed")?;
    check(
        !ok(&s2, &["claim", "lobby", "ask:1"])?.0,
        "an ask is room-wide, so another repository's claim conflicts",
    )?;
    let claims = w.ok("observer", &["claims", "lobby"])?;
    check(
        claims.matches("scope: git:").count() >= 2,
        format!("the claims show their repository scopes:\n{claims}"),
    )?;
    Ok(vec![])
}

fn worktree_scope(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let main = repo(w, "main")?;
    let tree = w.home.join("repos-main-worktree");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            &tree.to_string_lossy(),
            "-b",
            "side",
        ],
    )?;
    let tree = std::fs::canonicalize(&tree).map_err(|e| e.to_string())?;
    let s1 = h.start_in("dddd4444-main", "mainsession", &main)?;
    let s2 = h.start_in("eeee5555-tree", "treesession", &tree)?;
    s1.hook("SessionStart")?;
    s2.hook("SessionStart")?;
    check(
        s1.klatch(&["claim", "lobby", "README.md"])?.0,
        "the main checkout claims README.md",
    )?;
    check(
        !s2.klatch(&["claim", "lobby", "README.md"])?.0,
        "a worktree of the same repository shares its scope, so it conflicts",
    )?;
    Ok(vec![])
}

fn bot_scope(w: &World) -> Res {
    // A member with no harness, no repository and no registry: it declares a scope and uses the lobby.
    w.ok(
        "grokbot",
        &["--member-scope", "bot:grok", "claim", "lobby", "plan.md"],
    )?;
    check(
        w.ok("grokbot", &["scope"])?.trim() == "bot:grok",
        "its scope is remembered",
    )?;
    w.ok(
        "other",
        &[
            "--member-scope",
            "project:acme",
            "claim",
            "lobby",
            "plan.md",
        ],
    )?;
    let claims = w.ok("observer", &["claims", "lobby"])?;
    has(
        &claims,
        "scope: bot:grok",
        "the bot's claim shows its scope",
    )?;
    has(
        &claims,
        "scope: project:acme",
        "and so does the other member's",
    )?;
    w.ok("grokbot", &["post", "lobby", "seeking: a summariser"])?;
    w.ok("other", &["claim", "lobby", "ask:1"])?;
    w.ok("other", &["post", "lobby", "answer to #1: here"])?;
    lacks(
        &w.ok("observer", &["asks", "lobby"])?,
        "#1",
        "a non-coding member asks and is answered like any other",
    )?;
    Ok(vec![])
}
