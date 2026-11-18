use super::*;
use crate::world::CodexInput;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "two_agents_under_one_name_hide_each_others_posts_and_an_empty_read_says_so",
            origin: "#495, #500",
            layer: "fault",
            run: shared_name,
        },
        Scenario {
            name: "a_codex_session_is_one_stable_member_across_hook_resume_and_command",
            origin: "#341, RESEARCH-NOTES 37",
            layer: "functional",
            run: codex_session,
        },
    ]
}

/// A fake Codex hook payload: the JSON Codex writes on a hook's standard input.
fn payload(sid: &str, cwd: &str, source: &str) -> String {
    format!(
        r#"{{"session_id":"{sid}","cwd":"{cwd}","hook_event_name":"SessionStart","source":"{source}"}}"#
    )
}

/// A Codex session has no registry record: a fake Codex hook payload (`session_id`, `cwd`, `source`) joins it to the
/// lobby as one member, a `resume` payload with the same id and another directory is the same member, a command run
/// with `CODEX_THREAD_ID` is that member too, and a second thread in the same directory is a different member.
fn codex_session(w: &World) -> Res {
    let hook = ["hook", "--event", "SessionStart"];
    let start = |sid: &str, cwd: &str, source: &str| {
        w.codex(&hook, CodexInput::Hook(&payload(sid, cwd, source)))
    };
    let first = start("codex-thread-0001", "/work/bench", "startup");
    check(first.ok, format!("the codex hook failed: {}", first.stderr))?;
    let resumed = start("codex-thread-0001", "/elsewhere", "resume");
    check(
        resumed.ok,
        format!("the resumed hook failed: {}", resumed.stderr),
    )?;
    let post = w.codex(
        &["post", "lobby", "from the codex agent"],
        CodexInput::Thread("codex-thread-0001"),
    );
    check(
        post.ok,
        format!("the codex command failed: {}", post.stderr),
    )?;
    start("codex-thread-0002", "/work/bench", "startup");
    let who = w.ok("observer", &["who", "lobby", "--all"])?;
    let names: Vec<&str> = who
        .lines()
        .filter_map(|l| l.split('\t').next())
        .filter(|n| n.starts_with("codex-"))
        .collect();
    check(
        names.len() == 2 && names.contains(&"codex-bench"),
        format!("expected the resumed thread as one member plus a second thread:\n{who}"),
    )?;
    let read = w.ok("observer", &["read", "lobby"])?;
    check(
        read.lines()
            .any(|l| l.contains("from the codex agent") && l.contains("codex-bench")),
        format!("the command was not the hook's member:\n{read}"),
    )?;
    Ok(vec![])
}

fn shared_name(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("one", &["join", "town"])?;
    w.ok("two", &["join", "town"])?;
    w.ok("one", &["post", "town", "from one"])?;
    has(&w.ok("two", &["read", "town"])?, "from one", "two sees one")?;
    // a second process acting as "two" finds the room already read, and says why
    let again = w.run("two", &["read", "town"]);
    check(
        again.ok && again.stdout.is_empty(),
        "stdout must stay empty",
    )?;
    has(
        &again.stderr,
        "same member name",
        "the empty read explains itself",
    )?;
    Ok(vec![])
}
