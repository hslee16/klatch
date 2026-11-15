//! Retention and archive (#69, #70): old messages move to an encrypted archive file and leave a tombstone, so `seq`
//! stays gapless; a restore puts them back.

use super::*;
use crate::domain::Attribution;
use crate::store::sqlite::archive::TOMBSTONE;

fn old_and_new(s: &SqliteStore) {
    s.join("r", "alice", 1).unwrap();
    s.join("r", "bob", 1).unwrap();
    s.post(Seat::new("r", "alice"), "old one", 10).unwrap();
    s.post_as(
        &NewMessage::new(("r", "alice"), Attribution::Verified, "public", "old two")
            .with_handoff("note", "payload two"),
        20,
    )
    .unwrap();
    s.post(Seat::new("r", "bob"), "old three", 30).unwrap();
    s.post(Seat::new("r", "alice"), "new one", 1000).unwrap();
    s.post(Seat::new("r", "bob"), "new two", 1001).unwrap();
}

fn archive_path(tag: &str) -> std::path::PathBuf {
    let p = temp_db(tag).with_file_name(format!("arch-{tag}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn old_messages_move_to_an_archive_and_leave_tombstones_so_seq_stays_gapless() {
    let s = store();
    old_and_new(&s);
    let out = archive_path("move");
    let n = s
        .archive_before("r", 500, &out, &crate::store::insecure_test_key())
        .unwrap();
    assert_eq!(n, 3);
    let all = s.tail("r", 0, 100).unwrap();
    assert_eq!(
        all.iter().map(|m| m.seq).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    for m in &all[..3] {
        assert_eq!(m.body, TOMBSTONE);
        assert_eq!(m.kind.as_deref(), Some("archived"));
        assert!(m.payload.is_none());
    }
    assert_eq!(all[3].body, "new one");
    assert_eq!(s.head("r").unwrap(), 5);
    assert_eq!(all[0].sender, "alice", "who and when stay on the tombstone");
    assert_eq!(all[0].ts, 10);
    let _ = std::fs::remove_file(out);
}

#[test]
fn the_archive_file_is_encrypted_and_never_overwrites() {
    let s = store();
    old_and_new(&s);
    let out = archive_path("enc");
    s.archive_before("r", 500, &out, &crate::store::insecure_test_key())
        .unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "a plaintext archive"
    );
    assert!(
        !String::from_utf8_lossy(&bytes).contains("old three"),
        "message text is readable in the archive file"
    );
    let again = s.archive_before("r", 5000, &out, &crate::store::insecure_test_key());
    assert!(again.is_err(), "an existing archive was overwritten");
    let _ = std::fs::remove_file(out);
}

#[test]
fn nothing_to_archive_writes_no_file() {
    let s = store();
    old_and_new(&s);
    let out = archive_path("none");
    let n = s
        .archive_before("r", 5, &out, &crate::store::insecure_test_key())
        .unwrap();
    assert_eq!(n, 0);
    assert!(!out.exists());
}

#[test]
fn a_restore_brings_the_messages_back_exactly() {
    let s = store();
    old_and_new(&s);
    let before = s.tail("r", 0, 100).unwrap();
    let out = archive_path("restore");
    s.archive_before("r", 500, &out, &crate::store::insecure_test_key())
        .unwrap();
    let n = s
        .restore_archive(&out, &crate::store::insecure_test_key())
        .unwrap();
    assert_eq!(n, 3);
    assert_eq!(s.tail("r", 0, 100).unwrap(), before);
    assert!(out.exists(), "a restore keeps the archive");
    assert_eq!(
        s.restore_archive(&out, &crate::store::insecure_test_key())
            .unwrap(),
        0,
        "a second restore has nothing to do"
    );
    let _ = std::fs::remove_file(out);
}

#[test]
fn archiving_twice_does_not_archive_a_tombstone() {
    let s = store();
    old_and_new(&s);
    let a = archive_path("twice-a");
    let b = archive_path("twice-b");
    let key = crate::store::insecure_test_key();
    s.archive_before("r", 500, &a, &key).unwrap();
    assert_eq!(s.archive_before("r", 500, &b, &key).unwrap(), 0);
    let _ = std::fs::remove_file(a);
}

#[test]
fn a_restore_ignores_rows_that_do_not_match_a_tombstone() {
    let s = store();
    old_and_new(&s);
    let out = archive_path("other");
    let key = crate::store::insecure_test_key();
    s.archive_before("r", 500, &out, &key).unwrap();
    // another store with different messages at the same seqs must not take the archive's text
    let other = store();
    other.post(Seat::new("r", "mallory"), "mine", 1).unwrap();
    assert_eq!(other.restore_archive(&out, &key).unwrap(), 0);
    assert_eq!(other.tail("r", 0, 10).unwrap()[0].body, "mine");
    let _ = std::fs::remove_file(out);
}

#[test]
fn a_member_is_not_shown_a_tombstone_as_new_mail() {
    let s = store();
    old_and_new(&s);
    let out = archive_path("unread");
    s.archive_before("r", 500, &out, &crate::store::insecure_test_key())
        .unwrap();
    let got = s.read(Seat::new("r", "bob"), true, 2000).unwrap();
    assert_eq!(
        got.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["new one"],
        "bob sees the live message, not the archived ones"
    );
    let _ = std::fs::remove_file(out);
}

#[test]
fn nothing_due_is_not_an_error_even_if_the_archive_name_is_taken() {
    let s = store();
    old_and_new(&s);
    let out = archive_path("taken");
    let key = crate::store::insecure_test_key();
    s.archive_before("r", 500, &out, &key).unwrap();
    assert_eq!(s.archive_before("r", 500, &out, &key).unwrap(), 0);
    let _ = std::fs::remove_file(out);
}
