use super::*;

pub(super) fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a_muted_room_shows_only_mentions_and_read_shows_all",
            origin: "#20, R14",
            layer: "functional",
            run: muted_room,
        },
        Scenario {
            name: "a_hook_message_stays_readable_once",
            origin: "#198",
            layer: "functional",
            run: hook_message_readable_once,
        },
        Scenario {
            name: "post_tool_use_hook_prints_json_context",
            origin: "#202",
            layer: "functional",
            run: post_tool_use_json,
        },
        Scenario {
            name: "new_session_joins_without_flood_and_is_taught",
            origin: "#267, #209",
            layer: "functional",
            run: new_session_joins,
        },
        Scenario {
            name: "session_end_marks_done_and_clears_offers",
            origin: "#258",
            layer: "functional",
            run: session_end,
        },
        Scenario {
            name: "a_filtered_wait_leaves_messages_unread",
            origin: "#216, #237",
            layer: "functional",
            run: filtered_wait,
        },
        Scenario {
            name: "a_child_session_ending_never_marks_its_parent",
            origin: "#284, #285",
            layer: "functional",
            run: child_end_spares_parent,
        },
        Scenario {
            name: "nested_sessions_get_their_own_members",
            origin: "#284, #285",
            layer: "functional",
            run: nested_sessions,
        },
        Scenario {
            name: "two_sessions_with_one_name_get_distinct_members",
            origin: "#170, #285",
            layer: "functional",
            run: same_name_distinct,
        },
        Scenario {
            name: "a_renamed_session_keeps_its_member",
            origin: "#170, #285",
            layer: "functional",
            run: renamed_keeps_member,
        },
        Scenario {
            name: "a_hook_with_no_session_id_still_uses_the_walk",
            origin: "#284, #285",
            layer: "functional",
            run: no_session_id,
        },
        Scenario {
            name: "session_end_with_no_record_and_no_memory_does_nothing",
            origin: "#284, #285",
            layer: "fault",
            run: end_unknown,
        },
        Scenario {
            name: "members_age_out_of_who_and_are_pruned_after_a_day",
            origin: "#282, #292",
            layer: "functional",
            run: aging_and_prune,
        },
        Scenario {
            name: "offers_follow_their_senders_presence",
            origin: "#250, #292",
            layer: "functional",
            run: offers_presence,
        },
    ]
}

fn who_line(w: &World, member: &str) -> Result<String, String> {
    let who = w.ok("observer", &["who", "lobby", "--all"])?;
    who.lines()
        .find(|l| l.starts_with(&format!("{member}\t")))
        .map(String::from)
        .ok_or_else(|| format!("{member} is not in the lobby:\n{who}"))
}

fn hook_message_readable_once(w: &World) -> Res {
    w.ok("alice", &["join", "r"])?;
    w.ok("bob", &["post", "r", "the ask"])?;
    has(
        &w.ok("alice", &["hook"])?,
        "from bob (unverified): the ask",
        "the hook shows it",
    )?;
    lacks(
        &w.ok("alice", &["hook"])?,
        "the ask",
        "the hook does not repeat it",
    )?;
    has(
        &w.ok("alice", &["read", "r"])?,
        "the ask",
        "read still returns it once",
    )?;
    lacks(&w.ok("alice", &["read", "r"])?, "the ask", "and only once")?;
    Ok(vec![])
}

fn post_tool_use_json(w: &World) -> Res {
    w.ok("alice", &["join", "r"])?;
    w.ok("bob", &["post", "r", "ping \"quoted\""])?;
    let out = w.ok("alice", &["hook", "--event", "PostToolUse"])?;
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).map_err(|e| format!("not JSON: {e}: {out}"))?;
    check(
        v["hookSpecificOutput"]["hookEventName"] == "PostToolUse",
        "the event name is carried",
    )?;
    has(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or(""),
        "ping \"quoted\"",
        "the text is escaped and intact",
    )?;
    check(
        w.ok("alice", &["hook", "--event", "PostToolUse"])?
            .is_empty(),
        "nothing to say prints nothing",
    )?;
    Ok(vec![])
}

fn new_session_joins(w: &World) -> Res {
    w.ok("old", &["join", "lobby"])?;
    w.ok("old", &["post", "lobby", "old chatter"])?;
    w.ok("old", &["post", "lobby", "seeking: a rare thing"])?;
    w.ok("old", &["post", "lobby", "offering: text helpers"])?;
    let start = w.ok("fresh", &["hook", "--event", "SessionStart"])?;
    lacks(
        &start,
        "old chatter",
        "history must not flood a new session",
    )?;
    has(&start, "[board lobby]", "it sees the board")?;
    has(&start, "seeking: a rare thing", "with the open ask")?;
    has(
        &start,
        "[klatch convention v1]",
        "and is taught the convention",
    )?;
    has(
        &w.ok("old", &["who", "lobby"])?,
        "fresh",
        "it joined the lobby",
    )?;
    lacks(
        &w.ok("other", &["hook", "--event", "UserPromptSubmit"])?,
        "[board",
        "only session start shows the board",
    )?;
    lacks(
        &w.ok("quiet", &["hook", "--event", "SessionStart", "--no-join"])?,
        "convention",
        "--no-join is quiet",
    )?;
    Ok(vec![])
}

fn session_end(w: &World) -> Res {
    w.ok("alice", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "offering: text helpers"])?;
    w.ok("bob", &["join", "lobby"])?;
    check(
        w.ok("alice", &["hook", "--event", "SessionEnd"])?
            .is_empty(),
        "SessionEnd prints nothing",
    )?;
    let who = w.ok("bob", &["who", "lobby"])?;
    has(&who, "status: done", "the member is marked done")?;
    has(
        &w.ok("bob", &["board", "lobby"])?,
        "no current offers",
        "its offer left the board",
    )?;
    Ok(vec![])
}

fn filtered_wait(w: &World) -> Res {
    w.ok("alice", &["join", "r"])?;
    w.ok("bob", &["post", "r", "chatter"])?;
    let timeout = w.ok(
        "alice",
        &["wait", "r", "--timeout", "1", "--only", "seeking:"],
    )?;
    has(
        &timeout,
        "[klatch wait: timeout room=r after=1s]",
        "the marker",
    )?;
    has(
        &w.ok("alice", &["read", "r"])?,
        "chatter",
        "the skipped message is still unread",
    )?;
    Ok(vec![])
}

fn child_end_spares_parent(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let parent = h.start("parent-sid-0001", "par")?;
    parent.hook("SessionStart")?;
    let child = parent.spawn_child("child-sid-0002", "kid")?;
    child.hook("SessionStart")?;
    // The harness drops the child's record as it exits, then runs the SessionEnd hook: the walk up the process
    // tree now reaches the parent's record.
    child.drop_record();
    child.hook("SessionEnd")?;
    has(
        &who_line(w, "kid")?,
        "status: done",
        "the child is marked done",
    )?;
    lacks(
        &who_line(w, "par")?,
        "status: done",
        "the parent must not be marked done",
    )?;
    Ok(vec![])
}

fn nested_sessions(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let parent = h.start("parent-sid-0001", "par")?;
    let child = parent.spawn_child("child-sid-0002", "kid")?;
    has(
        &parent.hook("SessionStart")?,
        "joined lobby as par",
        "the parent joins as itself",
    )?;
    has(
        &child.hook("SessionStart")?,
        "joined lobby as kid",
        "the child joins as itself",
    )?;
    lacks(
        &who_line(w, "par")?,
        "child of",
        "a top level session has no parent",
    )?;
    has(
        &who_line(w, "kid")?,
        "child of par",
        "the child is shown with the session that started it",
    )?;
    let runs = w.ok("observer", &["hooks", "--of", "kid"])?;
    has(
        &runs,
        "SessionStart",
        "the child's own hook run is recorded",
    )?;
    Ok(vec![])
}

fn same_name_distinct(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let first = h.start("aaaa1111-first", "proj-20")?;
    let second = h.start("bbbb2222-second", "proj-20")?;
    has(
        &first.hook("SessionStart")?,
        "joined lobby as proj-20 ",
        "the first keeps the name",
    )?;
    let out = second.hook("SessionStart")?;
    lacks(
        &out,
        "joined lobby as proj-20 ",
        "the second must not take the same member",
    )?;
    has(&out, "joined lobby as proj-20-", "it gets a suffix")?;
    let who = w.ok("observer", &["who", "lobby", "--all"])?;
    check(
        who.lines().filter(|l| l.starts_with("proj-20")).count() == 2,
        format!("expected two members:\n{who}"),
    )?;
    // Each keeps its member on later hooks.
    lacks(
        &first.hook("UserPromptSubmit")?,
        "error",
        "the first is stable",
    )?;
    check(
        w.ok("observer", &["hooks", "--of", "proj-20"])?
            .contains("UserPromptSubmit"),
        "the first's hooks are recorded under its own name",
    )?;
    Ok(vec![])
}

fn renamed_keeps_member(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let s = h.start("cccc3333-session", "first-name")?;
    s.hook("SessionStart")?;
    s.rename("renamed-by-the-user")?;
    // A resumed or restarted session runs SessionStart again under its new name: it must be the same member.
    has(
        &s.hook("SessionStart")?,
        "back in lobby as first-name",
        "the session is told it is back, under its first name",
    )?;
    let who = w.ok("observer", &["who", "lobby", "--all"])?;
    lacks(
        &who,
        "renamed-by-the-user",
        "a rename must not make a new member",
    )?;
    has(&who, "first-name", "the session keeps its first name")?;
    Ok(vec![])
}

fn no_session_id(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let s = h.start("dddd4444-session", "solo")?;
    has(
        &s.hook_without_id("SessionStart")?,
        "joined lobby as solo",
        "the walk still finds the session",
    )?;
    Ok(vec![])
}

fn end_unknown(w: &World) -> Res {
    let h = FakeHarness::new(w)?;
    let s = h.start("eeee5555-session", "ghost")?;
    s.drop_record();
    // No record and nothing remembered for this session id: the hook must do nothing, quietly.
    let out = s.hook("SessionEnd")?;
    check(
        out.trim().is_empty(),
        format!("a SessionEnd with nothing to go on printed: {out}"),
    )?;
    let who = w.ok("observer", &["who", "lobby", "--all"])?;
    lacks(&who, "status: done", "no one was marked done")?;
    Ok(vec![])
}

fn aging_and_prune(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.advance(601);
    let who = w.ok("bob", &["who", "lobby"])?;
    lacks(
        &who,
        "alice\t",
        "a member unseen for over ten minutes is hidden",
    )?;
    has(&who, "not seen for over 10 minutes", "and counted")?;
    has(
        &w.ok("bob", &["who", "lobby", "--all"])?,
        "alice",
        "who --all still lists it",
    )?;
    w.advance(86_400);
    w.ok("carol", &["hook", "--event", "SessionStart", "--no-join"])?;
    lacks(
        &w.ok("bob", &["who", "lobby", "--all"])?,
        "alice",
        "a session start prunes members unseen for a day",
    )?;
    Ok(vec![])
}

fn offers_presence(w: &World) -> Res {
    w.set_clock(T0);
    w.ok("alice", &["join", "lobby"])?;
    w.ok("alice", &["post", "lobby", "offering: text helpers"])?;
    w.advance(100);
    has(
        &w.ok("bob", &["board", "lobby"])?,
        "alice offers",
        "the offer stands while alice is present",
    )?;
    w.advance(600);
    has(
        &w.ok("bob", &["board", "lobby"])?,
        "no current offers",
        "it goes when alice has not been seen for ten minutes",
    )?;
    Ok(vec![])
}

fn muted_room(w: &World) -> Res {
    w.ok("alice", &["join", "lobby"])?;
    w.ok("bob", &["post", "lobby", "chatter one"])?;
    w.ok("bob", &["post", "lobby", "@alice please look"])?;
    w.ok("bob", &["post", "lobby", "@alicia not for alice"])?;
    w.ok("alice", &["mute", "lobby"])?;
    let hook = w.ok("alice", &["hook", "--event", "UserPromptSubmit"])?;
    has(&hook, "@alice please look", "a mention gets through a mute")?;
    lacks(&hook, "chatter one", "ordinary chatter is held back")?;
    lacks(&hook, "@alicia", "a different name is not a mention")?;
    has(
        &w.ok("alice", &["read", "lobby", "--peek"])?,
        "chatter one",
        "read still shows everything",
    )?;
    w.ok("alice", &["unmute", "lobby"])?;
    w.ok("bob", &["post", "lobby", "chatter two"])?;
    has(
        &w.ok("alice", &["hook", "--event", "UserPromptSubmit"])?,
        "chatter two",
        "an unmuted room shows everything again",
    )?;
    Ok(vec![])
}
