//! Sealing a label over the messages already in it, and moving a label's messages to a new key (#378). Both rewrite
//! stored text in one transaction; the binding of a sealed text to its message, label and field never changes.

use super::*;
use crate::domain::{Attribution, Error};
use crate::keys::{Key, LabelKeys};
use crate::store::sqlite::{reseal_archives, resealable_archives};

fn key(b: u8) -> Key {
    Key::from_bytes([b; 32])
}

fn ring(label: &str, b: u8) -> LabelKeys {
    LabelKeys::new().with(label, key(b))
}

fn post(s: &SqliteStore, room: &str, label: &str, body: &str, ts: i64) {
    s.post_as(
        &NewMessage::new((room, "alice"), Attribution::Unattributed, label, body),
        ts,
    )
    .unwrap();
}

fn post_handoff(s: &SqliteStore, label: &str, body: &str, payload: &str, ts: i64) {
    s.post_as(
        &NewMessage::new(("r", "alice"), Attribution::Unattributed, label, body)
            .with_handoff("handoff", payload),
        ts,
    )
    .unwrap();
}

/// The stored body of every message of `label`, in seq order, exactly as written in the file.
fn raw_bodies(s: &SqliteStore, label: &str) -> Vec<String> {
    s.rows(
        "SELECT body FROM messages WHERE label=?1 ORDER BY room, seq",
        [label],
        |r| r.get(0),
    )
    .unwrap()
}

fn raw_payloads(s: &SqliteStore, label: &str) -> Vec<Option<String>> {
    s.rows(
        "SELECT payload FROM messages WHERE label=?1 ORDER BY room, seq",
        [label],
        |r| r.get(0),
    )
    .unwrap()
}

fn read_all(s: &SqliteStore) -> Vec<String> {
    s.tail("r", 0, 50)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect()
}

fn is_sealed(text: &str) -> bool {
    crate::keys::is_sealed_text(text)
}

fn sealed(s: &SqliteStore, label: &str) -> bool {
    s.policy_get(&format!("sealed:{label}")).unwrap().is_some()
}

/// A store whose `secret` label is sealed under key 0x33 with three messages in it, one carrying a payload.
fn sealed_store() -> SqliteStore {
    let s = store();
    s.policy_set("sealed:secret", Some("1"), 1).unwrap();
    s.set_label_keys(ring("secret", 0x33));
    post(&s, "r", "secret", "first secret", 2);
    post_handoff(&s, "secret", "second secret", "payload-two", 3);
    post(&s, "r", "secret", "third secret", 4);
    post(&s, "r", "public", "an ordinary message", 5);
    s
}

// ---- B: sealing a label converts what is already in it ----

#[test]
fn sealing_a_label_converts_every_earlier_plain_body_and_payload_of_that_label_and_no_other() {
    let s = store();
    post(&s, "r", "secret", "old plain one", 1);
    post_handoff(&s, "secret", "old plain two", "old-payload", 2);
    post(&s, "r", "public", "left alone", 3);
    s.set_label_keys(ring("secret", 0x33));
    let n = s.mark_label_sealed_converting("secret", 10).unwrap();
    assert_eq!(n, 3, "two bodies and one payload");
    assert!(sealed(&s, "secret"));
    for body in raw_bodies(&s, "secret") {
        assert!(is_sealed(&body), "{body}");
        assert!(!body.contains("old plain"), "{body}");
    }
    assert!(is_sealed(raw_payloads(&s, "secret")[1].as_ref().unwrap()));
    assert_eq!(raw_bodies(&s, "public"), vec!["left alone".to_string()]);
    // the holder still reads them
    let shown = read_all(&s);
    assert!(shown.contains(&"old plain one".to_string()), "{shown:?}");
    assert!(shown.contains(&"old plain two".to_string()), "{shown:?}");
}

#[test]
fn a_converted_body_is_bound_to_its_message_so_a_moved_one_does_not_open() {
    let s = store();
    post(&s, "r", "secret", "alpha", 1);
    post(&s, "r", "secret", "beta", 2);
    s.set_label_keys(ring("secret", 0x33));
    s.mark_label_sealed_converting("secret", 10).unwrap();
    let bodies = raw_bodies(&s, "secret");
    s.conn
        .execute("UPDATE messages SET body=?1 WHERE seq=1", [&bodies[1]])
        .unwrap();
    let shown = read_all(&s);
    assert!(shown[0].contains("sealed"), "{shown:?}");
    assert!(!shown[0].contains("beta"), "{shown:?}");
}

#[test]
fn sealing_converts_nothing_and_seals_nothing_when_the_label_key_is_not_held() {
    let s = store();
    post(&s, "r", "secret", "old plain", 1);
    s.set_label_keys(LabelKeys::new());
    let err = s.mark_label_sealed_converting("secret", 10).unwrap_err();
    assert!(
        matches!(err, Error::Refused { ref rule, .. } if rule == "label-key"),
        "{err}"
    );
    assert!(!sealed(&s, "secret"), "the label was marked sealed");
    assert_eq!(raw_bodies(&s, "secret"), vec!["old plain".to_string()]);
}

#[test]
fn sealing_a_label_with_nothing_to_convert_needs_no_key_and_marks_it_sealed() {
    let s = store();
    s.set_label_keys(LabelKeys::new());
    assert_eq!(s.mark_label_sealed_converting("secret", 10).unwrap(), 0);
    assert!(sealed(&s, "secret"));
}

#[test]
fn an_archived_tombstone_is_not_sealed_and_a_repeat_converts_nothing() {
    let s = store();
    post(&s, "r", "secret", "old plain", 1);
    let dir = crate::store::test_dir::TestDir::new("seal-convert-tomb");
    s.archive_before(
        "r",
        500,
        &dir.join("r-1.db"),
        &crate::store::insecure_test_key(),
    )
    .unwrap();
    s.set_label_keys(ring("secret", 0x33));
    assert_eq!(s.mark_label_sealed_converting("secret", 10).unwrap(), 0);
    assert_eq!(
        raw_bodies(&s, "secret"),
        vec![crate::store::sqlite::archive::TOMBSTONE.to_string()]
    );
    post(&s, "r", "secret", "later", 600);
    assert_eq!(s.mark_label_sealed_converting("secret", 11).unwrap(), 0);
}

// ---- A: moving a label to a new key ----

#[test]
fn after_a_move_the_new_key_reads_every_message_and_the_old_key_opens_none() {
    let s = sealed_store();
    let before = raw_bodies(&s, "secret");
    let n = s.reseal_label("secret", &key(0x33), &key(0x44)).unwrap();
    assert_eq!(n, 4, "three bodies and one payload");
    assert_ne!(raw_bodies(&s, "secret"), before);
    s.set_label_keys(ring("secret", 0x44));
    assert_eq!(
        read_all(&s)[..3],
        ["first secret", "second secret", "third secret"]
    );
    assert_eq!(
        s.tail("r", 0, 50).unwrap()[1].payload.as_deref(),
        Some("payload-two")
    );
    s.set_label_keys(ring("secret", 0x33));
    for m in s.tail("r", 0, 50).unwrap().iter().take(3) {
        assert!(m.body.contains("sealed"), "the old key opened {}", m.body);
    }
    assert!(
        s.tail("r", 0, 50).unwrap()[1]
            .payload
            .as_deref()
            .unwrap()
            .contains("sealed")
    );
}

#[test]
fn a_moved_body_still_fails_to_open_in_another_message() {
    let s = sealed_store();
    s.reseal_label("secret", &key(0x33), &key(0x44)).unwrap();
    let bodies = raw_bodies(&s, "secret");
    s.conn
        .execute("UPDATE messages SET body=?1 WHERE seq=1", [&bodies[2]])
        .unwrap();
    s.set_label_keys(ring("secret", 0x44));
    let first = &read_all(&s)[0];
    assert!(first.contains("sealed"), "{first}");
    assert!(!first.contains("third secret"), "{first}");
}

#[test]
fn a_body_of_another_label_is_never_touched_by_a_move() {
    let s = sealed_store();
    s.policy_set("sealed:other", Some("1"), 1).unwrap();
    s.set_label_keys(
        LabelKeys::new()
            .with("secret", key(0x33))
            .with("other", key(0x55)),
    );
    post(&s, "r", "other", "other label text", 6);
    let other = raw_bodies(&s, "other");
    s.reseal_label("secret", &key(0x33), &key(0x44)).unwrap();
    assert_eq!(raw_bodies(&s, "other"), other);
    assert_eq!(
        raw_bodies(&s, "public"),
        vec!["an ordinary message".to_string()]
    );
}

#[test]
fn a_row_the_old_key_does_not_open_refuses_the_move_and_changes_nothing() {
    let s = sealed_store();
    // the second row was sealed under a key that is neither the old nor the new one
    let foreign = crate::keys::protect_text(
        &key(0x99),
        SqliteStore::sealed_context(
            &s.rows::<String>(
                "SELECT mid FROM messages WHERE label='secret' AND seq=2",
                [],
                |r| r.get(0),
            )
            .unwrap()[0],
            "secret",
            "body",
        )
        .as_bytes(),
        "second secret",
    )
    .unwrap();
    s.conn
        .execute("UPDATE messages SET body=?1 WHERE seq=2", [&foreign])
        .unwrap();
    let before = (raw_bodies(&s, "secret"), raw_payloads(&s, "secret"));
    assert!(
        s.check_label_reseal("secret", &key(0x33), &key(0x44))
            .is_err()
    );
    let err = s
        .reseal_label("secret", &key(0x33), &key(0x44))
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
    assert_eq!(
        (raw_bodies(&s, "secret"), raw_payloads(&s, "secret")),
        before,
        "a row changed although the move was refused"
    );
}

#[test]
fn a_row_already_under_the_new_key_is_skipped_so_a_repeat_finishes_an_earlier_run() {
    let s = sealed_store();
    // one row was moved by an earlier run that died: seq 1 is already under the new key
    let mid: String = s
        .rows(
            "SELECT mid FROM messages WHERE label='secret' AND seq=1",
            [],
            |r| r.get(0),
        )
        .unwrap()
        .remove(0);
    let moved = crate::keys::protect_text(
        &key(0x44),
        SqliteStore::sealed_context(&mid, "secret", "body").as_bytes(),
        "first secret",
    )
    .unwrap();
    s.conn
        .execute("UPDATE messages SET body=?1 WHERE seq=1", [&moved])
        .unwrap();
    let n = s.reseal_label("secret", &key(0x33), &key(0x44)).unwrap();
    assert_eq!(n, 3, "the moved row is not counted");
    assert_eq!(
        raw_bodies(&s, "secret")[0],
        moved,
        "a row already under the new key was written again"
    );
    s.set_label_keys(ring("secret", 0x44));
    assert_eq!(
        read_all(&s)[..3],
        ["first secret", "second secret", "third secret"]
    );
    // and a second repeat has nothing left
    assert_eq!(s.reseal_label("secret", &key(0x33), &key(0x44)).unwrap(), 0);
}

#[test]
fn a_failure_part_way_through_a_move_rolls_every_row_back() {
    let s = sealed_store();
    let before = (raw_bodies(&s, "secret"), raw_payloads(&s, "secret"));
    let err = s
        .reseal_label_hooked("secret", &key(0x33), &key(0x44), &|done| {
            if done == 2 {
                Err(Error::Backend("injected failure".into()))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert!(err.to_string().contains("injected"), "{err}");
    assert_eq!(
        (raw_bodies(&s, "secret"), raw_payloads(&s, "secret")),
        before
    );
    // the old key still opens everything
    assert_eq!(
        read_all(&s)[..3],
        ["first secret", "second secret", "third secret"]
    );
}

#[test]
fn counting_what_each_key_opens_tells_a_promoted_store_from_an_unpromoted_one() {
    let s = sealed_store();
    let all = |a: u8, b: u8| s.label_open_counts("secret", &key(a), &key(b)).unwrap();
    assert_eq!(all(0x33, 0x44), (4, 4, 0));
    s.reseal_label("secret", &key(0x33), &key(0x44)).unwrap();
    assert_eq!(all(0x33, 0x44), (4, 0, 4));
}

// ---- A: the archive files of the label ----

fn archive_with_secret(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, Key) {
    let s = store();
    s.policy_set("sealed:secret", Some("1"), 1).unwrap();
    s.set_label_keys(ring("secret", 0x33));
    s.join("r", "alice", 1).unwrap();
    post(&s, "r", "secret", "archived secret", 10);
    post(&s, "r", "public", "archived public", 11);
    let file = dir.join(name);
    let store_key = crate::store::insecure_test_key();
    s.archive_before("r", 500, &file, &store_key).unwrap();
    (file, store_key)
}

fn archived_bodies(file: &std::path::Path, store_key: &Key) -> Vec<(String, String)> {
    let c = rusqlite::Connection::open(file).unwrap();
    c.pragma_update(None, "key", store_key.sqlcipher_pragma().as_str())
        .unwrap();
    c.prepare("SELECT label, body FROM archived ORDER BY seq")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}

#[test]
fn an_archive_file_is_moved_to_the_new_label_key_and_its_other_rows_are_unchanged() {
    let dir = crate::store::test_dir::TestDir::new("label-archive");
    let (file, store_key) = archive_with_secret(&dir, "r-1.db");
    let before = archived_bodies(&file, &store_key);
    let n = reseal_archives(&dir, &store_key, "secret", &key(0x33), &key(0x44)).unwrap();
    assert_eq!(n, 1);
    let after = archived_bodies(&file, &store_key);
    assert_ne!(after[0].1, before[0].1);
    assert_eq!(after[1], before[1], "a public row changed");
    // the moved row opens only under the new key, bound to its message id
    let mid: String = {
        let c = rusqlite::Connection::open(&file).unwrap();
        c.pragma_update(None, "key", store_key.sqlcipher_pragma().as_str())
            .unwrap();
        c.query_row("SELECT mid FROM archived WHERE label='secret'", [], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let ctx = SqliteStore::sealed_context(&mid, "secret", "body");
    assert!(crate::keys::reveal_text(&key(0x44), ctx.as_bytes(), &after[0].1).is_ok());
    assert!(crate::keys::reveal_text(&key(0x33), ctx.as_bytes(), &after[0].1).is_err());
    // a second pass has nothing to move and leaves no temp file
    assert_eq!(
        reseal_archives(&dir, &store_key, "secret", &key(0x33), &key(0x44)).unwrap(),
        0
    );
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn an_archive_row_no_key_opens_refuses_the_move_before_any_file_is_changed() {
    let dir = crate::store::test_dir::TestDir::new("label-archive-stuck");
    let (good, store_key) = archive_with_secret(&dir, "r-1.db");
    let (bad, _) = archive_with_secret(&dir, "r-2.db");
    {
        let c = rusqlite::Connection::open(&bad).unwrap();
        c.pragma_update(None, "key", store_key.sqlcipher_pragma().as_str())
            .unwrap();
        c.execute(
            "UPDATE archived SET body='klatch-sealed:1:AAAA' WHERE label='secret'",
            [],
        )
        .unwrap();
    }
    let good_before = archived_bodies(&good, &store_key);
    let err = resealable_archives(&dir, &store_key, "secret", &key(0x33), &key(0x44)).unwrap_err();
    assert!(err.to_string().contains("r-2.db"), "{err}");
    let err = reseal_archives(&dir, &store_key, "secret", &key(0x33), &key(0x44)).unwrap_err();
    assert!(err.to_string().contains("r-2.db"), "{err}");
    assert_eq!(archived_bodies(&good, &store_key), good_before);
}
