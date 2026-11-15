use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![Scenario {
        name: "claims_anchored_to_a_pull_request_and_a_worktree_are_released_by_their_events",
        origin: "#57, #58, DESIGN D5",
        layer: "fault",
        run: anchored,
    }]
}

const MERGED: &str = r#"{"action":"closed","pull_request":{"number":7,"state":"closed","merged":true,"title":"x"},"repository":{"full_name":"Acme/Widgets"}}"#;
const OPEN: &str = r#"{"action":"reopened","pull_request":{"number":7,"state":"open"},"repository":{"full_name":"Acme/Widgets"}}"#;

fn anchored(w: &World) -> Res {
    w.set_clock(T0);
    let dir = w.home.join("anchor-fixtures");
    std::fs::create_dir_all(dir.join("worktrees").join("feature-x")).map_err(|e| e.to_string())?;
    let file = |name: &str, body: &str| -> Result<String, String> {
        let p = dir.join(name);
        std::fs::write(&p, body).map_err(|e| e.to_string())?;
        Ok(p.to_string_lossy().into_owned())
    };
    let (merged, open) = (file("merged.json", MERGED)?, file("open.json", OPEN)?);
    let hold = |who: &str, path: &str, anchor: &str| {
        w.ok(
            who,
            &["claim", "r", path, "--ttl", "3600", "--anchor", anchor],
        )
    };
    hold("alice", "pr.rs", "gh:pr:Acme/Widgets/7")?;
    hold("bob", "wt.rs", "wt:feature-x")?;
    // an open pull request releases nothing; a merged one releases its claim and no other
    has(
        &w.ok("launcher", &["anchor", "github", "--file", &open])?,
        "released 0",
        "open pull request",
    )?;
    has(
        &w.ok("launcher", &["anchor", "github", "--file", &merged])?,
        "released 1",
        "merged pull request",
    )?;
    let held = w.ok("alice", &["claims", "r"])?;
    lacks(&held, "pr.rs", "claim after the merge")?;
    has(&held, "wt.rs", "other claims stay")?;
    // a worktree that is still there keeps its claim; removing it frees the path
    let root = dir.join("worktrees").to_string_lossy().into_owned();
    has(
        &w.ok("launcher", &["anchor", "reap", "--worktrees", &root])?,
        "released 0",
        "worktree present",
    )?;
    std::fs::remove_dir(dir.join("worktrees").join("feature-x")).map_err(|e| e.to_string())?;
    has(
        &w.ok("launcher", &["anchor", "reap", "--worktrees", &root])?,
        "released 1",
        "worktree removed",
    )?;
    lacks(
        &w.ok("alice", &["claims", "r"])?,
        "wt.rs",
        "claim after removal",
    )?;
    // a payload that is not clearly an item is refused whole
    let junk = file("junk.json", "{\"state\":\"closed\"}")?;
    check(
        !w.run("launcher", &["anchor", "github", "--file", &junk]).ok,
        "a payload with no repository was accepted",
    )?;
    Ok(Vec::new())
}
