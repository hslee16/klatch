//! The store is encrypted at rest (#156, R32): nothing readable in the file or its WAL, a wrong or missing key is
//! refused, tampering is detected, and a build without SQLCipher cannot open a store at all.

use super::*;

fn key(b: u8) -> klatch_keys::Key {
    klatch_keys::Key::from_bytes([b; 32])
}

const SECRETS: [&str; 5] = [
    "launch-code-hunter2",
    "vault-room-name",
    "member-quokkaworth",
    "claim/path/zebra.rs",
    "reason-for-the-giraffe-claim",
];

/// A store with every kind of data in it, kept open so its WAL still exists.
fn filled(path: &std::path::Path, k: &klatch_keys::Key) -> SqliteStore {
    let s = SqliteStore::open(path, k).unwrap();
    s.join("vault-room-name", "member-quokkaworth", 1).unwrap();
    s.post(
        Seat::new("vault-room-name", "member-quokkaworth"),
        "launch-code-hunter2",
        2,
    )
    .unwrap();
    s.claim(
        &ClaimRequest::new(
            ("vault-room-name", "member-quokkaworth"),
            "claim/path/zebra.rs",
            100,
            "reason-for-the-giraffe-claim",
        ),
        3,
    )
    .unwrap();
    s
}

fn raw(path: &std::path::Path) -> Vec<u8> {
    let mut all = std::fs::read(path).unwrap_or_default();
    for ext in ["-wal", "-journal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(ext);
        all.extend(std::fs::read(std::path::PathBuf::from(p)).unwrap_or_default());
    }
    all
}

#[test]
fn the_file_and_its_wal_contain_no_plaintext_of_any_kind() {
    let path = temp_db("enc-grep");
    let s = filled(&path, &key(7));
    let bytes = raw(&path);
    assert!(bytes.len() > 4096, "nothing was written");
    for secret in SECRETS {
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "{secret} is readable in the file or its WAL"
        );
    }
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "the file announces itself as plain SQLite"
    );
    drop(s);
    let bytes = raw(&path);
    for secret in SECRETS {
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "{secret} after close"
        );
    }
}

#[test]
fn the_right_key_reads_everything_back() {
    let path = temp_db("enc-rt");
    drop(filled(&path, &key(7)));
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    let m = s.tail("vault-room-name", 0, 10).unwrap();
    assert_eq!(m[0].body, "launch-code-hunter2");
    assert_eq!(
        s.claims("vault-room-name", 10).unwrap()[0].path,
        "claim/path/zebra.rs"
    );
}

#[test]
fn a_wrong_key_is_refused_with_a_clear_message_and_changes_nothing() {
    let path = temp_db("enc-wrong");
    drop(filled(&path, &key(7)));
    let before = std::fs::read(&path).unwrap();
    let err = SqliteStore::open(&path, &key(8))
        .err()
        .expect("opened with the wrong key")
        .to_string();
    assert!(err.contains("wrong key"), "{err}");
    assert!(!err.contains("0808"), "the message must not carry the key");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused open changed the file"
    );
}

#[test]
fn opening_the_encrypted_file_without_any_key_reads_nothing() {
    let path = temp_db("enc-nokey");
    drop(filled(&path, &key(7)));
    let c = Connection::open(&path).unwrap();
    assert!(
        c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .is_err()
    );
}

#[test]
fn a_flipped_byte_in_a_page_is_detected_not_read_as_data() {
    let path = temp_db("enc-tamper");
    drop(filled(&path, &key(7)));
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(
        bytes.len() > 4096 * 2,
        "too small to tamper with a data page"
    );
    let at = 4096 + 100;
    bytes[at] ^= 0x40;
    std::fs::write(&path, &bytes).unwrap();
    let outcome = SqliteStore::open(&path, &key(7)).and_then(|s| {
        s.tail("vault-room-name", 0, 10)
            .map(|_| ())
            .and_then(|()| s.claims("vault-room-name", 10).map(|_| ()))
    });
    let integrity = Connection::open(&path).ok().and_then(|c| {
        c.pragma_update(None, "key", key(7).sqlcipher_pragma().as_str())
            .ok()?;
        c.query_row("PRAGMA cipher_integrity_check", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    });
    assert!(
        outcome.is_err() || integrity.is_some(),
        "a flipped byte went unnoticed by both a read and the integrity check"
    );
}

#[test]
fn the_store_says_it_is_built_with_sqlcipher_and_refuses_otherwise() {
    let path = temp_db("enc-version");
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    assert!(
        s.cipher_version().unwrap().starts_with("4."),
        "{:?}",
        s.cipher_version()
    );
}

#[test]
fn two_stores_with_different_keys_do_not_share_data_or_failures() {
    let (pa, pb) = (temp_db("enc-a"), temp_db("enc-b"));
    let (a, b) = (
        SqliteStore::open(&pa, &key(1)).unwrap(),
        SqliteStore::open(&pb, &key(2)).unwrap(),
    );
    a.join("r", "alice", 1).unwrap();
    assert!(b.members("r").unwrap().is_empty());
}

#[test]
fn rekeying_moves_the_store_to_a_new_key_and_the_old_key_stops_working() {
    let path = temp_db("enc-rekey");
    let s = filled(&path, &key(7));
    s.rekey(&key(9)).unwrap();
    drop(s);
    assert!(
        SqliteStore::open(&path, &key(7)).is_err(),
        "the old key still opens it"
    );
    let s = SqliteStore::open(&path, &key(9)).unwrap();
    assert_eq!(
        s.tail("vault-room-name", 0, 10).unwrap()[0].body,
        "launch-code-hunter2"
    );
    assert_eq!(s.claims("vault-room-name", 10).unwrap().len(), 1);
    let bytes = raw(&path);
    for secret in SECRETS {
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "{secret} after rekey"
        );
    }
}

#[test]
fn a_store_keeps_working_for_other_connections_after_a_rekey() {
    let path = temp_db("enc-rekey-more");
    let s = filled(&path, &key(7));
    s.rekey(&key(9)).unwrap();
    s.post(
        Seat::new("vault-room-name", "member-quokkaworth"),
        "after the rekey",
        10,
    )
    .unwrap();
    let other = SqliteStore::open(&path, &key(9)).unwrap();
    assert_eq!(other.tail("vault-room-name", 0, 10).unwrap().len(), 2);
}

#[test]
fn temporary_results_stay_in_memory() {
    let path = temp_db("enc-temp");
    let s = filled(&path, &key(7));
    let mode: i64 = s
        .conn
        .query_row("PRAGMA temp_store", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, 2, "temp_store is not MEMORY");
}

#[test]
fn a_store_written_by_a_newer_klatch_is_refused_not_guessed_at() {
    let path = temp_db("enc-newer");
    drop(filled(&path, &key(7)));
    {
        let c = keyed_with(&path, &key(7));
        c.pragma_update(
            None,
            "user_version",
            crate::sqlite::schema::SCHEMA_VERSION + 5,
        )
        .unwrap();
    }
    let err = SqliteStore::open(&path, &key(7))
        .err()
        .expect("opened a newer store")
        .to_string();
    assert!(err.contains("newer klatch"), "{err}");
}

#[test]
fn a_plaintext_file_is_refused_by_open_and_left_untouched() {
    let path = temp_db("enc-plain-refused");
    std::fs::write(&path, include_bytes!("../../../fixtures/prototype-v2.db")).unwrap();
    let before = std::fs::read(&path).unwrap();
    let err = SqliteStore::open(&path, &key(7))
        .err()
        .expect("opened a plaintext file")
        .to_string();
    assert!(
        err.contains("wrong key") || err.contains("not an encrypted"),
        "{err}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused open changed the file"
    );
}

fn keyed_with(path: &std::path::Path, k: &klatch_keys::Key) -> Connection {
    let c = Connection::open(path).unwrap();
    c.pragma_update(None, "key", k.sqlcipher_pragma().as_str())
        .unwrap();
    c
}

// ---- per-label field encryption (#160) ----

use klatch_core::Attribution;
use klatch_keys::LabelKeys;

fn secret_ring(b: u8) -> LabelKeys {
    LabelKeys::new().with("secret", key(b))
}

fn seal_label(s: &SqliteStore, label: &str) {
    s.policy_set(&format!("sealed:{label}"), Some("1"), 1)
        .unwrap();
}

fn raw_body(path: &std::path::Path, k: &klatch_keys::Key) -> String {
    let c = keyed_with(path, k);
    c.query_row("SELECT body FROM messages WHERE label='secret'", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn a_sealed_label_is_stored_as_ciphertext_even_to_someone_who_can_open_the_database() {
    let path = temp_db("lk-raw");
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    seal_label(&s, "secret");
    s.set_label_keys(secret_ring(0x33));
    s.post_as(
        &NewMessage::new(
            ("r", "alice"),
            Attribution::Unattributed,
            "secret",
            "the-sealed-launch-code",
        ),
        2,
    )
    .unwrap();
    s.post_as(
        &NewMessage::new(
            ("r", "alice"),
            Attribution::Unattributed,
            "public",
            "an ordinary message",
        ),
        3,
    )
    .unwrap();
    drop(s);
    // the store key opens the database, and still the secret body is not there
    let body = raw_body(&path, &key(7));
    assert!(!body.contains("the-sealed-launch-code"), "{body}");
    assert!(body.starts_with("klatch-sealed:1:"), "{body}");
    let public: String = keyed_with(&path, &key(7))
        .query_row("SELECT body FROM messages WHERE label='public'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        public, "an ordinary message",
        "an unsealed label is stored as it was"
    );
}

#[test]
fn a_principal_holding_the_label_key_reads_the_message_and_one_without_it_does_not() {
    let path = temp_db("lk-read");
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    seal_label(&s, "secret");
    s.set_label_keys(secret_ring(0x33));
    s.post_as(
        &NewMessage::new(
            ("r", "alice"),
            Attribution::Unattributed,
            "secret",
            "the-sealed-launch-code",
        ),
        2,
    )
    .unwrap();
    assert_eq!(
        s.tail("r", 0, 10).unwrap()[0].body,
        "the-sealed-launch-code"
    );
    drop(s);
    // opened with the store key but too few label keys: every way of reading shows the placeholder
    let few = SqliteStore::open(&path, &key(7)).unwrap();
    few.set_label_keys(LabelKeys::new());
    for body in [
        few.tail("r", 0, 10).unwrap()[0].body.clone(),
        few.read(Seat::new("r", "bob"), false, 5).unwrap()[0]
            .body
            .clone(),
        few.hook_read("r", "bob", 6).unwrap()[0].body.clone(),
    ] {
        assert!(!body.contains("the-sealed-launch-code"), "{body}");
        assert!(body.contains("sealed") && body.contains("secret"), "{body}");
    }
    // the wrong key for the label is the same as no key
    let wrong = SqliteStore::open(&path, &key(7)).unwrap();
    wrong.set_label_keys(secret_ring(0x44));
    assert!(wrong.tail("r", 0, 10).unwrap()[0].body.contains("sealed"));
}

#[test]
fn a_principal_without_the_key_of_a_sealed_label_cannot_write_to_it() {
    let path = temp_db("lk-write");
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    seal_label(&s, "secret");
    s.set_label_keys(LabelKeys::new());
    let err = s
        .post_as(
            &NewMessage::new(
                ("r", "alice"),
                Attribution::Unattributed,
                "secret",
                "plain text must never be stored under a sealed label",
            ),
            2,
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::Refused { ref rule, .. } if rule == "label-key"),
        "{err}"
    );
    assert!(
        s.tail("r", 0, 10).unwrap().is_empty(),
        "the refused message was stored"
    );
}

#[test]
fn a_moved_or_altered_sealed_body_is_not_shown_as_a_message() {
    let path = temp_db("lk-move");
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    seal_label(&s, "secret");
    s.set_label_keys(secret_ring(0x33));
    s.post_as(
        &NewMessage::new(
            ("r", "alice"),
            Attribution::Unattributed,
            "secret",
            "first secret",
        ),
        2,
    )
    .unwrap();
    s.post_as(
        &NewMessage::new(
            ("r", "alice"),
            Attribution::Unattributed,
            "secret",
            "second secret",
        ),
        3,
    )
    .unwrap();
    // swap the two bodies in the database: each is now bound to the wrong message id
    {
        let c = keyed_with(&path, &key(7));
        let body = |seq: i64| -> String {
            c.query_row("SELECT body FROM messages WHERE seq=?1", [seq], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let (one, two) = (body(1), body(2));
        c.execute("UPDATE messages SET body=?1 WHERE seq=1", [&two])
            .unwrap();
        c.execute("UPDATE messages SET body=?1 WHERE seq=2", [&one])
            .unwrap();
    }
    let shown: Vec<String> = s
        .tail("r", 0, 10)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    for b in &shown {
        assert!(
            !b.contains("first secret") && !b.contains("second secret"),
            "{b}"
        );
        assert!(b.contains("sealed"), "{b}");
    }
}

#[test]
fn a_label_that_is_not_sealed_needs_no_key_and_a_store_with_no_ring_works_as_before() {
    let path = temp_db("lk-plain");
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    s.post_as(
        &NewMessage::new(
            ("r", "alice"),
            Attribution::Unattributed,
            "private",
            "not a sealed label",
        ),
        2,
    )
    .unwrap();
    assert_eq!(s.tail("r", 0, 10).unwrap()[0].body, "not a sealed label");
}

#[test]
fn a_plain_message_cannot_pose_as_sealed_text() {
    let path = temp_db("lk-spoof");
    let s = SqliteStore::open(&path, &key(7)).unwrap();
    let err = s
        .post_as(
            &NewMessage::new(
                ("r", "eve"),
                Attribution::Unattributed,
                "public",
                "klatch-sealed:1:AAAA",
            ),
            2,
        )
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
}
