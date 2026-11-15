//! Property tests with Hegel (epic #396). Each draws generated cases (100 by default, well under a second in all) and,
//! when one fails, shrinks it to a minimal counterexample and prints how to replay it. They are **evidence, not proof**:
//! a pass means no counterexample was found in the cases tried.
//!
//! Run them alone: `cargo test -p klatch --test properties`. Run more cases:
//! `HEGEL_TEST_CASES=100000 cargo test -p klatch --test properties`. Replay a failure from its blob or seed: see
//! CONTRIBUTING.md. Draw plain integers and map them to values inside the test: Hegel prints and shrinks those without
//! further setup.

use hegel::TestCase;
use hegel::generators as gs;
use klatch::domain::{SHARED_SCOPE, clean_path, overlaps, scopes_collide};
use klatch::protocol::{decode_client_line, decode_server_line};

/// Path segments that hit the interesting cases often: dots, empties, ordinary names and a non-ASCII one. A test draws
/// indices into this list (shrinking toward the first entry).
const SEGMENTS: [&str; 7] = ["a", "b", "..", ".", "", "c.d", "ä"];
const SCOPES: [&str; 5] = ["*", "", "a", "b", "ab"];

fn indices() -> impl hegel::generators::PrintableGenerator<Vec<u8>> {
    gs::vecs(gs::integers::<u8>().max_value(SEGMENTS.len() as u8 - 1))
}

fn path_of(indices: &[u8], rooted: bool) -> String {
    let joined = indices
        .iter()
        .map(|&i| SEGMENTS[i as usize])
        .collect::<Vec<_>>()
        .join("/");
    if rooted { format!("/{joined}") } else { joined }
}

#[hegel::test]
fn a_cleaned_path_is_absolute_has_no_dot_segments_and_cleaning_again_changes_nothing(tc: TestCase) {
    let rooted = tc.draw(gs::booleans());
    let path = path_of(&tc.draw(indices()), rooted);
    let cwd = path_of(&tc.draw(indices()), true);
    let cleaned = clean_path(&path, &cwd);
    assert!(cleaned.starts_with('/'), "not absolute: {cleaned:?}");
    assert!(!cleaned.contains("//"), "doubled slash: {cleaned:?}");
    assert!(
        cleaned.split('/').all(|s| s != "." && s != ".."),
        "dot segment left in {cleaned:?} (from {path:?} in {cwd:?})"
    );
    // an absolute path does not depend on the directory it is cleaned from, and cleaning twice is cleaning once
    assert_eq!(clean_path(&cleaned, "/elsewhere"), cleaned);
    assert_eq!(clean_path(&cleaned, &cwd), cleaned);
    // appending "x/.." to a path changes nothing; this is the property that notices a dropped ".." under some depth.
    // (An empty relative path is the exception to how it is spelled: "" plus "/x/.." would be an absolute path.)
    let longer = if path.is_empty() {
        "x/..".to_string()
    } else {
        format!("{path}/x/..")
    };
    assert_eq!(
        clean_path(&longer, &cwd),
        cleaned,
        "appending x/.. to {path:?}"
    );
}

#[hegel::test]
fn scopes_collide_is_symmetric_reflexive_and_the_shared_scope_collides_with_everything(
    tc: TestCase,
) {
    let pick = |i: u8| SCOPES[i as usize];
    let a = pick(tc.draw(gs::integers::<u8>().max_value(SCOPES.len() as u8 - 1)));
    let b = pick(tc.draw(gs::integers::<u8>().max_value(SCOPES.len() as u8 - 1)));
    assert_eq!(scopes_collide(a, b), scopes_collide(b, a), "{a:?} {b:?}");
    assert!(scopes_collide(a, a), "{a:?} does not collide with itself");
    assert!(scopes_collide(SHARED_SCOPE, b) && scopes_collide(a, SHARED_SCOPE));
    if a != SHARED_SCOPE && b != SHARED_SCOPE && a != b {
        assert!(
            !scopes_collide(a, b),
            "different scopes {a:?} {b:?} collide"
        );
    }
}

#[hegel::test]
fn path_overlap_is_symmetric_and_a_path_overlaps_itself_and_what_is_beneath_it(tc: TestCase) {
    let a = path_of(&tc.draw(indices()), tc.draw(gs::booleans()));
    let b = path_of(&tc.draw(indices()), tc.draw(gs::booleans()));
    assert_eq!(overlaps(&a, &b), overlaps(&b, &a), "{a:?} {b:?}");
    assert!(overlaps(&a, &a), "{a:?}");
    let below = format!("{}/x", a.trim_end_matches('/'));
    assert!(overlaps(&a, &below), "{a:?} vs {below:?}");
}

#[hegel::test]
fn the_decoders_never_panic_and_refuse_what_is_not_a_version_1_frame(tc: TestCase) {
    let line = tc.draw(gs::text());
    // the property is the absence of a panic; an error is the right answer for almost all of these
    let _ = decode_client_line(&line);
    let _ = decode_server_line(&line);
    // a JSON object whose version is not 1 is never accepted
    let n = tc.draw(gs::integers::<u32>());
    tc.assume(n != 1);
    let framed = format!(r#"{{"v":{n},"type":"ping"}}"#);
    assert!(decode_client_line(&framed).is_err(), "accepted {framed}");
    assert!(decode_server_line(&framed).is_err(), "accepted {framed}");
}

// ---- the hook never prints more than its budget, and always prints JSON -----------------------------------------

/// Room names that are valid (1 to 40 of letters, digits, `-_.`), including the longest, which costs the most header.
const ROOM_NAMES: [&str; 5] = [
    "lobby",
    "a",
    "r.1_b-c",
    "ask-12",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
];

/// Text that hits the escapes and the multi-byte cases: quotes, backslashes, newlines, control characters, emoji,
/// and whatever Hegel makes of `text()`, repeated so a room can overflow the budget.
fn nasty_text(tc: &TestCase) -> String {
    const PIECES: [&str; 8] = [
        "plain ",
        "\"quoted\" ",
        "back\\slash ",
        "line\nbreak ",
        "tab\t",
        "\u{1}\u{7}\u{1f}",
        "\u{1F600}\u{00e4}\u{4e2d} ",
        "```\n",
    ];
    let mut s = String::new();
    for _ in 0..tc.draw(gs::integers::<u8>().max_value(6)) {
        s.push_str(
            PIECES[tc.draw(gs::integers::<u8>().max_value(PIECES.len() as u8 - 1)) as usize],
        );
    }
    s.push_str(&tc.draw(gs::text()).chars().take(30).collect::<String>());
    s.repeat(1 + tc.draw(gs::integers::<u16>().max_value(24)) as usize)
}

fn digest_for(tc: &TestCase, room: &str) -> klatch::domain::RoomDigest {
    use klatch::domain::{Attribution, Claim, Message, OpenAsk, ReleasedClaim, RoomDigest};
    let count = |max: u8| tc.draw(gs::integers::<u8>().max_value(max)) as usize;
    let mut d = RoomDigest {
        room: room.into(),
        ..RoomDigest::default()
    };
    for seq in 1..=count(14) as i64 {
        d.messages.push(Message {
            id: format!("m{seq}"),
            seq,
            sender: ["alice", "bob"][count(1)].into(),
            attribution: if tc.draw(gs::booleans()) {
                Attribution::Verified
            } else {
                Attribution::Unattributed
            },
            label: "public".into(),
            body: nasty_text(tc),
            ts: 1_000,
            kind: tc.draw(gs::booleans()).then(|| "note".to_string()),
            payload: tc.draw(gs::booleans()).then(|| nasty_text(tc)),
        });
    }
    for _ in 0..count(12) {
        d.claims.push(Claim {
            path: nasty_text(tc),
            owner: "bob".into(),
            reason: nasty_text(tc),
            expires: 2_000,
            scope: String::new(),
        });
    }
    for _ in 0..count(6) {
        d.released.push(ReleasedClaim {
            owner: "bob".into(),
            path: nasty_text(tc),
            note: tc.draw(gs::booleans()).then(|| nasty_text(tc)),
        });
    }
    let ask = |seq: i64, text: String| OpenAsk {
        seq,
        sender: "bob".into(),
        text,
        ts: 900,
        claimed: false,
        outcomes: Vec::new(),
        waiting_on_asker: None,
        room: None,
    };
    for seq in 1..=count(8) as i64 {
        d.stale.push(ask(seq, nasty_text(tc)));
    }
    for seq in 1..=count(8) as i64 {
        d.backlog.push(ask(100 + seq, nasty_text(tc)));
    }
    for _ in 0..count(4) {
        d.aged.push(klatch::domain::AgedClaim {
            path: nasty_text(tc),
            held_secs: 3_600,
        });
    }
    d
}

/// For rooms and messages with quotes, escapes, control characters and non-ASCII, in any number: what the hook
/// prints is valid JSON, carries the text unchanged, and the text stays within `DIGEST_MAX_BYTES` once escaped (the
/// cap Claude Code applies to hook output, past which the model sees only a preview, #549).
#[hegel::test]
fn the_hook_output_is_always_valid_json_within_its_byte_budget(tc: TestCase) {
    use klatch::domain::{budget, hook_output_json, printed_len, render_hook, render_hook_within};
    // up to hundreds of rooms: past about 45 the headers alone outgrew the budget (#599)
    let rooms = tc.draw(gs::integers::<u16>().max_value(300)) as usize;
    // only the first few rooms are drawn afresh; the rest are copies under other names, so the case stays cheap to
    // generate and to shrink however many rooms it has
    let fresh: Vec<_> = (0..rooms.min(5))
        .map(|i| {
            digest_for(
                &tc,
                ROOM_NAMES[(i + tc.draw(gs::integers::<u8>().max_value(4)) as usize) % 5],
            )
        })
        .collect();
    let digests: Vec<_> = (0..rooms)
        .map(|i| {
            let mut d = fresh[i % fresh.len()].clone();
            if i >= fresh.len() {
                d.room = format!("{}{i}", d.room).chars().take(40).collect();
            }
            d
        })
        .collect();
    let text = render_hook(&digests, 1_500);
    assert!(
        printed_len(&text) <= budget::DIGEST_MAX_BYTES,
        "{} rooms print {} bytes, over {}",
        rooms,
        printed_len(&text),
        budget::DIGEST_MAX_BYTES
    );
    let json = hook_output_json("UserPromptSubmit", &text, None);
    assert!(
        json.len() <= budget::HOOK_MAX_BYTES,
        "the envelope took the output to {} bytes",
        json.len()
    );
    if !text.is_empty() {
        let value: serde_json::Value =
            serde_json::from_str(json.trim_end()).expect("the hook prints valid JSON");
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            text.as_str(),
            "the text did not survive the escaping"
        );
    }
    // a session start shares the cap with the convention and boards, so it asks for less
    let smaller = tc.draw(
        gs::integers::<usize>()
            .min_value(4_000)
            .max_value(budget::DIGEST_MAX_BYTES),
    );
    let within = render_hook_within(&digests, 1_500, smaller);
    assert!(
        printed_len(&within) <= smaller,
        "{} rooms print {} bytes inside a budget of {smaller}",
        rooms,
        printed_len(&within)
    );
}

// ---- archive and restore ---------------------------------------------------------------------------------------

/// A file name no other case uses, in the system temp dir (the property runs many cases in one process).
fn scratch_file(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("klatch-prop-{tag}-{}-{n}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

/// Archiving moves old messages out and leaves tombstones; restoring puts them back. For any messages (unicode
/// bodies, handoffs, either attribution), any two cutoffs and either restore order: `seq` stays 1..n with no gap
/// after every step, a tombstone is never delivered as mail to anyone, a second archive moves nothing, and after the
/// restores the room is exactly what it was.
#[hegel::test]
fn archive_then_restore_is_the_identity_and_seq_stays_dense_and_tombstones_are_never_mail(
    tc: TestCase,
) {
    use klatch::domain::{Attribution, Message};
    use klatch::store::{NewMessage, Seat, SqliteStore, Store, insecure_test_key};
    let s = SqliteStore::open_in_memory().unwrap();
    let members = ["alice", "bob", "carol"];
    for m in members {
        s.join("r", m, 1).unwrap();
    }
    let n = tc.draw(gs::integers::<u8>().max_value(14)) as i64;
    let mut ts = 10;
    for _ in 0..n {
        let who = members[tc.draw(gs::integers::<u8>().max_value(2)) as usize];
        ts += tc.draw(gs::integers::<i64>().min_value(0).max_value(3));
        let body = nasty_text(&tc);
        let payload = nasty_text(&tc);
        let mut m = NewMessage::new(
            ("r", who),
            if tc.draw(gs::booleans()) {
                Attribution::Verified
            } else {
                Attribution::Unattributed
            },
            "public",
            &body,
        );
        if tc.draw(gs::booleans()) {
            m = m.with_handoff("note", &payload);
        }
        s.post_as(&m, ts).unwrap();
    }
    let cutoff = |tc: &TestCase| tc.draw(gs::integers::<i64>().min_value(0).max_value(ts + 2));
    let key = insecure_test_key();
    let original = s.tail("r", 0, 1_000).unwrap();
    let dense = |log: &[Message]| {
        assert_eq!(
            log.iter().map(|m| m.seq).collect::<Vec<_>>(),
            (1..=n).collect::<Vec<_>>(),
            "seq has a gap"
        )
    };
    let (first, second) = (cutoff(&tc), cutoff(&tc));
    let (file_a, file_b) = (scratch_file("a"), scratch_file("b"));
    let moved_a = s.archive_before("r", first, &file_a, &key).unwrap();
    let moved_b = s.archive_before("r", second, &file_b, &key).unwrap();
    let archived = s.tail("r", 0, 1_000).unwrap();
    dense(&archived);
    let due = |c: i64| original.iter().filter(|m| m.ts < c).count();
    assert_eq!(
        moved_a,
        due(first),
        "the first archive moved the wrong number"
    );
    assert_eq!(
        moved_a + moved_b,
        due(first.max(second)),
        "a tombstone was archived again or a message was missed"
    );
    for (before, after) in original.iter().zip(&archived) {
        let gone = before.ts < first.max(second);
        assert_eq!(
            after.kind.as_deref() == Some("archived"),
            gone,
            "seq {} tombstoned wrongly",
            before.seq
        );
        assert_eq!(
            (&after.id, after.seq, &after.sender, after.ts),
            (&before.id, before.seq, &before.sender, before.ts)
        );
        if !gone {
            assert_eq!(after, before, "a live message changed");
        }
    }
    for (i, m) in members.iter().enumerate() {
        let mail = s
            .read(Seat::new("r", m), tc.draw(gs::booleans()), 5_000)
            .unwrap();
        assert!(
            mail.iter()
                .all(|x| x.kind.as_deref() != Some("archived") && !x.body.starts_with("[archived")),
            "{} was handed a tombstone as mail",
            members[i]
        );
    }
    let restore_b_first = tc.draw(gs::booleans());
    let order = if restore_b_first {
        [(&file_b, moved_b), (&file_a, moved_a)]
    } else {
        [(&file_a, moved_a), (&file_b, moved_b)]
    };
    for (file, _) in order {
        if file.exists() {
            s.restore_archive(file, &key).unwrap();
            dense(&s.tail("r", 0, 1_000).unwrap());
        }
    }
    assert_eq!(
        s.tail("r", 0, 1_000).unwrap(),
        original,
        "restore is not the identity"
    );
    for f in [file_a, file_b] {
        let _ = std::fs::remove_file(f);
    }
}

// ---- paging ----------------------------------------------------------------------------------------------------

/// `tail` and `history` read without moving anyone's cursor, and paging through a room with either gives the whole
/// log, once, in order. Concatenated pages equal the full log; each page is within its size; `more` says how many
/// follow; and after all the paging the member's unread mail is exactly what it was.
#[hegel::test]
fn paging_through_tail_and_history_returns_the_whole_log_once_and_moves_no_cursor(tc: TestCase) {
    use klatch::domain::Principal;
    use klatch::gateway::{AllowAll, FixedClock, Gateway, HistoryRange};
    use klatch::store::SqliteStore;
    let gw = Gateway::new(
        SqliteStore::open_in_memory().unwrap(),
        FixedClock(1_000),
        AllowAll,
    );
    let (alice, bob) = (
        Principal::unattributed("alice"),
        Principal::unattributed("bob"),
    );
    gw.join(&alice, "r").unwrap();
    gw.join(&bob, "r").unwrap();
    let n = tc.draw(gs::integers::<u8>().max_value(48)) as i64;
    for i in 1..=n {
        let who = if tc.draw(gs::booleans()) {
            &alice
        } else {
            &bob
        };
        gw.post(who, "r", &format!("message {i}")).unwrap();
    }
    let unread = |p: &Principal| gw.read(p, "r", false).unwrap();
    let (alice_unread, bob_unread) = (unread(&alice), unread(&bob));
    let full = gw.tail(&alice, "r", (0, 1_000)).unwrap();
    assert_eq!(
        full.iter().map(|m| m.seq).collect::<Vec<_>>(),
        (1..=n).collect::<Vec<_>>()
    );
    // tail pages
    let size = tc.draw(gs::integers::<usize>().min_value(1).max_value(9));
    let (mut paged, mut after) = (Vec::new(), 0);
    for _ in 0..=n + 1 {
        // bounded, so a pager that never advances fails the property instead of hanging it
        let page = gw.tail(&bob, "r", (after, size)).unwrap();
        if page.is_empty() {
            break;
        }
        assert!(
            page.len() <= size,
            "a page of {} for a limit of {size}",
            page.len()
        );
        after = page.last().unwrap().seq;
        paged.extend(page);
    }
    assert_eq!(paged, full, "the tail pages are not the log");
    // history pages forward from `since`; `more` counts what follows
    let (mut paged, mut since) = (Vec::new(), 0);
    for _ in 0..=n + 1 {
        // bounded, so a pager that never advances fails the property instead of hanging it
        let h = gw
            .history(
                &bob,
                "r",
                HistoryRange {
                    since: Some(since),
                    last: None,
                },
            )
            .unwrap();
        let Some(last) = h.messages.last() else { break };
        assert_eq!(
            h.more as i64,
            n - last.seq,
            "`more` is wrong after seq {}",
            last.seq
        );
        since = last.seq;
        paged.extend(h.messages);
    }
    assert_eq!(paged, full, "the history pages are not the log");
    // the newest k, and nothing moved
    let k = tc.draw(gs::integers::<usize>().min_value(1).max_value(30));
    let newest = gw
        .history(
            &alice,
            "r",
            HistoryRange {
                since: None,
                last: Some(k),
            },
        )
        .unwrap();
    let want: Vec<_> = full
        .iter()
        .skip(full.len().saturating_sub(k))
        .cloned()
        .collect();
    assert_eq!(
        newest.messages, want,
        "history --last {k} is not the end of the log"
    );
    assert_eq!(newest.more, 0);
    assert_eq!(
        (unread(&alice), unread(&bob)),
        (alice_unread, bob_unread),
        "paging moved a cursor"
    );
}
