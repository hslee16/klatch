//! Rotating a sealed label's key (#378): every message of the label moves to the new key, the old key opens none of
//! them afterwards, and a crash at any step is finished by a repeated rotation or repaired by the next load of the
//! label's key. They use temporary key directories only and never touch a keystore.

use super::label_rotation::{Step, rotate_label_key_in};
use super::*;
use crate::domain::Attribution;
use crate::keys::{FileKeys, label_key_name};
use crate::store::areas::*;
use crate::store::{NewMessage, Seat};

const NAME: &str = "store-test";
const LABEL: &str = "secret";

fn scratch(tag: &str) -> crate::store::test_dir::TestDir {
    crate::store::test_dir::TestDir::new(&format!("labelrot-{tag}"))
}

fn file_home(d: &Path) -> KeyHome {
    KeyHome {
        source: Box::new(FileKeys::new(&d.join("keys"))),
        name: NAME.into(),
        warning: None,
    }
}

fn label_name() -> String {
    label_key_name(NAME, LABEL)
}

fn main_key(kh: &KeyHome) -> Key {
    kh.source.get(&label_name()).unwrap().unwrap()
}

fn next_key(kh: &KeyHome) -> Option<Key> {
    kh.source.get(&next_name(&label_name())).unwrap()
}

/// A store in `d` whose `secret` label is sealed under a key of its own, with three sealed messages (one with a
/// payload), one public message, and an archive file holding one more sealed message.
fn sealed_store(d: &Path) -> (PathBuf, KeyHome) {
    let db = d.join("klatch.db");
    let kh = file_home(d);
    let store = open_store_in(&db, &kh).unwrap();
    let store_key = kh.source.get(NAME).unwrap().unwrap();
    store.policy_set("sealed:secret", Some("1"), 1).unwrap();
    crate::keys::create(kh.source.as_ref(), &label_name()).unwrap();
    store.set_label_keys(label_ring_in(&db, &kh, &[LABEL.to_string()]).unwrap());
    store.join("r", "alice", 1).unwrap();
    fn msg<'a>(label: &'a str, body: &'a str) -> NewMessage<'a> {
        NewMessage::new(("r", "alice"), Attribution::Unattributed, label, body)
    }
    store.post_as(&msg(LABEL, "archived secret"), 10).unwrap();
    let dir = d.join("archive");
    std::fs::create_dir_all(&dir).unwrap();
    store
        .archive_before("r", 500, &dir.join("r-100.db"), &store_key)
        .unwrap();
    store.post_as(&msg(LABEL, "first secret"), 1000).unwrap();
    store
        .post_as(
            &msg(LABEL, "second secret").with_handoff("handoff", "payload-two"),
            1001,
        )
        .unwrap();
    store.post_as(&msg("public", "plain words"), 1002).unwrap();
    store.post_as(&msg(LABEL, "third secret"), 1003).unwrap();
    (db, kh)
}

/// What the label's key in the keystore (and a repair on load) lets this process read of the live store.
fn readable(db: &Path, kh: &KeyHome) -> Vec<String> {
    let store = open_store_in(db, kh).unwrap();
    store.set_label_keys(label_ring_in(db, kh, &[LABEL.to_string()]).unwrap());
    store
        .tail("r", 0, 50)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect()
}

fn live_secrets(shown: &[String]) -> Vec<&str> {
    shown
        .iter()
        .map(String::as_str)
        .filter(|b| b.ends_with("secret"))
        .collect()
}

fn counts(db: &Path, kh: &KeyHome, a: &Key, b: &Key) -> (usize, usize, usize) {
    open_store_in(db, kh)
        .unwrap()
        .label_open_counts(LABEL, a, b)
        .unwrap()
}

fn archived_opens_with(d: &Path, label_key: &Key, kh: &KeyHome) -> bool {
    let store_key = kh.source.get(NAME).unwrap().unwrap();
    let file = d.join("archive").join("r-100.db");
    let c = rusqlite::Connection::open(&file).unwrap();
    c.pragma_update(None, "key", store_key.sqlcipher_pragma().as_str())
        .unwrap();
    let (mid, body): (String, String) = c
        .query_row(
            "SELECT mid, body FROM archived WHERE label='secret'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let ctx = format!("klatch message {mid} under {LABEL} field body");
    crate::keys::reveal_text(label_key, ctx.as_bytes(), &body).is_ok()
}

fn no_hook(_: Step) -> Result<()> {
    Ok(())
}

#[test]
fn rotating_a_label_key_moves_every_message_and_the_archive_and_the_old_key_opens_none() {
    let d = scratch("all");
    let (db, kh) = sealed_store(&d);
    let old = main_key(&kh);
    let new = rotate_label_key_in(&db, &kh, LABEL, &no_hook).unwrap();
    assert!(old != new);
    assert!(main_key(&kh) == new, "the new key was not promoted");
    assert!(next_key(&kh).is_none(), "the second name was left behind");
    assert_eq!(
        live_secrets(&readable(&db, &kh)),
        ["first secret", "second secret", "third secret"]
    );
    // three bodies and one payload, all under the new key, none under the old one
    assert_eq!(counts(&db, &kh, &old, &new), (4, 0, 4));
    assert!(archived_opens_with(&d, &new, &kh));
    assert!(!archived_opens_with(&d, &old, &kh));
    // the other keys are untouched: the store key still opens the store
    assert!(kh.source.get(NAME).unwrap().is_some());
    // and a second rotation works on the new key
    let third = rotate_label_key_in(&db, &kh, LABEL, &no_hook).unwrap();
    assert!(third != new);
    assert_eq!(counts(&db, &kh, &new, &third), (4, 0, 4));
}

#[test]
fn a_crash_after_any_step_is_finished_by_a_repeated_rotation() {
    let steps = [
        Step::NextStored,
        Step::ArchivesMoved,
        Step::Row(1),
        Step::Row(2),
        Step::StoreMoved,
        Step::Promoted,
    ];
    for crash in steps {
        let d = scratch(&format!("crash-{crash:?}"));
        let (db, kh) = sealed_store(&d);
        let old = main_key(&kh);
        let err = rotate_label_key_in(&db, &kh, LABEL, &|s| {
            if s == crash {
                Err(crate::domain::Error::Backend("injected crash".into()).into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(err.to_string().contains("injected crash"), "{crash:?}: {err}");
        // whatever the state, what a process reads is whole: the label's key in the keystore opens every message
        assert_eq!(
            live_secrets(&readable(&db, &kh)),
            ["first secret", "second secret", "third secret"],
            "unreadable after a crash at {crash:?}"
        );
        // and the repeat finishes the rotation
        let done = rotate_label_key_in(&db, &kh, LABEL, &no_hook).unwrap();
        assert!(main_key(&kh) == done, "{crash:?}: not promoted");
        assert!(next_key(&kh).is_none(), "{crash:?}: second name left");
        assert!(done != old, "{crash:?}: the key did not change");
        assert_eq!(counts(&db, &kh, &old, &done), (4, 0, 4), "{crash:?}");
        assert!(archived_opens_with(&d, &done, &kh), "{crash:?}: archive");
        assert_eq!(
            live_secrets(&readable(&db, &kh)),
            ["first secret", "second secret", "third secret"]
        );
    }
}

#[test]
fn a_crash_between_the_move_and_the_promotion_is_repaired_by_the_next_load_of_the_key() {
    let d = scratch("repair");
    let (db, kh) = sealed_store(&d);
    let old = main_key(&kh);
    rotate_label_key_in(&db, &kh, LABEL, &|s| {
        if s == Step::StoreMoved {
            Err(crate::domain::Error::Backend("crash".into()).into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    // the store is under the new key, the keystore still names the old one as the main key
    let next = next_key(&kh).expect("the second key is kept");
    assert!(main_key(&kh) == old);
    let ring = label_ring_in(&db, &kh, &[LABEL.to_string()]).unwrap();
    assert!(
        ring.get(LABEL).unwrap() == &next,
        "the load did not take the key that opens the store"
    );
    assert!(main_key(&kh) == next, "the key was not promoted");
    assert!(next_key(&kh).is_none(), "the second name was not removed");
}

#[test]
fn a_load_does_not_promote_a_second_key_while_the_main_key_still_opens_the_messages() {
    let d = scratch("no-promote");
    let (db, kh) = sealed_store(&d);
    let old = main_key(&kh);
    rotate_label_key_in(&db, &kh, LABEL, &|s| {
        if s == Step::ArchivesMoved {
            Err(crate::domain::Error::Backend("crash".into()).into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    let ring = label_ring_in(&db, &kh, &[LABEL.to_string()]).unwrap();
    assert!(ring.get(LABEL).unwrap() == &old, "promoted too early");
    assert!(main_key(&kh) == old);
    assert!(next_key(&kh).is_some(), "the second key was dropped");
}

#[test]
fn a_load_does_not_promote_a_second_key_that_opens_only_some_of_the_messages() {
    let d = scratch("some");
    let (db, kh) = sealed_store(&d);
    let old = main_key(&kh);
    // a second key that opens nothing the store holds
    kh.source
        .put(&next_name(&label_name()), &Key::generate().unwrap())
        .unwrap();
    let ring = label_ring_in(&db, &kh, &[LABEL.to_string()]).unwrap();
    assert!(ring.get(LABEL).unwrap() == &old);
    assert!(main_key(&kh) == old);
}

#[test]
fn a_second_name_equal_to_the_main_key_is_just_removed_by_the_next_load() {
    let d = scratch("same");
    let (db, kh) = sealed_store(&d);
    let old = main_key(&kh);
    kh.source.put(&next_name(&label_name()), &old).unwrap();
    label_ring_in(&db, &kh, &[LABEL.to_string()]).unwrap();
    assert!(next_key(&kh).is_none());
    assert!(main_key(&kh) == old);
}

#[test]
fn a_label_that_is_not_sealed_is_refused_and_nothing_is_made() {
    let d = scratch("unsealed");
    let (db, kh) = sealed_store(&d);
    let err = rotate_label_key_in(&db, &kh, "public", &no_hook)
        .unwrap_err()
        .to_string();
    assert!(err.contains("not sealed"), "{err}");
    assert!(
        kh.source
            .get(&label_key_name(NAME, "public"))
            .unwrap()
            .is_none()
    );
    assert!(
        kh.source
            .get(&next_name(&label_key_name(NAME, "public")))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_sealed_label_with_no_key_in_the_keystore_is_refused() {
    let d = scratch("nokey");
    let (db, kh) = sealed_store(&d);
    kh.source.delete(&label_name()).unwrap();
    let err = rotate_label_key_in(&db, &kh, LABEL, &no_hook)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no key") && err.contains("recover"), "{err}");
    assert!(next_key(&kh).is_none());
}

#[test]
fn a_sealed_row_that_does_not_open_refuses_the_rotation_and_changes_nothing() {
    let d = scratch("stuck");
    let (db, kh) = sealed_store(&d);
    let old = main_key(&kh);
    {
        let store_key = kh.source.get(NAME).unwrap().unwrap();
        let c = rusqlite::Connection::open(&db).unwrap();
        c.pragma_update(None, "key", store_key.sqlcipher_pragma().as_str())
            .unwrap();
        c.execute(
            "UPDATE messages SET body='klatch-sealed:1:AAAA' WHERE body LIKE 'klatch-sealed:1:%' AND seq=3",
            [],
        )
        .unwrap();
    }
    let before = open_store_in(&db, &kh)
        .unwrap()
        .label_open_counts(LABEL, &old, &old)
        .unwrap();
    let err = rotate_label_key_in(&db, &kh, LABEL, &no_hook)
        .unwrap_err()
        .to_string();
    assert!(err.contains("neither") && err.contains("not changed"), "{err}");
    assert!(main_key(&kh) == old, "the key changed");
    assert!(next_key(&kh).is_none(), "a second key was stored");
    assert_eq!(
        open_store_in(&db, &kh)
            .unwrap()
            .label_open_counts(LABEL, &old, &old)
            .unwrap(),
        before
    );
    assert!(archived_opens_with(&d, &old, &kh), "the archive was moved");
}

#[test]
fn an_archive_row_that_does_not_open_refuses_the_rotation_and_changes_nothing() {
    let d = scratch("stuck-archive");
    let (db, kh) = sealed_store(&d);
    let old = main_key(&kh);
    let store_key = kh.source.get(NAME).unwrap().unwrap();
    {
        let c = rusqlite::Connection::open(d.join("archive").join("r-100.db")).unwrap();
        c.pragma_update(None, "key", store_key.sqlcipher_pragma().as_str())
            .unwrap();
        c.execute(
            "UPDATE archived SET body='klatch-sealed:1:AAAA' WHERE label='secret'",
            [],
        )
        .unwrap();
    }
    let err = rotate_label_key_in(&db, &kh, LABEL, &no_hook)
        .unwrap_err()
        .to_string();
    assert!(err.contains("r-100.db"), "{err}");
    assert!(main_key(&kh) == old);
    assert!(next_key(&kh).is_none());
    assert_eq!(
        counts(&db, &kh, &old, &old),
        (4, 4, 4),
        "the live store changed although the rotation was refused"
    );
}

#[test]
fn rotating_one_label_leaves_another_sealed_label_alone() {
    let d = scratch("other");
    let (db, kh) = sealed_store(&d);
    let other = label_key_name(NAME, "other");
    let other_key = crate::keys::create(kh.source.as_ref(), &other).unwrap().0;
    {
        let store = open_store_in(&db, &kh).unwrap();
        store.policy_set("sealed:other", Some("1"), 1).unwrap();
        store.set_label_keys(
            label_ring_in(&db, &kh, &[LABEL.to_string(), "other".to_string()]).unwrap(),
        );
        store
            .post_as(
                &NewMessage::new(("r", "alice"), Attribution::Unattributed, "other", "other text"),
                2000,
            )
            .unwrap();
        let _ = Seat::new("r", "alice");
    }
    rotate_label_key_in(&db, &kh, LABEL, &no_hook).unwrap();
    assert!(kh.source.get(&other).unwrap().unwrap() == other_key);
    let store = open_store_in(&db, &kh).unwrap();
    store.set_label_keys(
        label_ring_in(&db, &kh, &[LABEL.to_string(), "other".to_string()]).unwrap(),
    );
    let shown: Vec<String> = store
        .tail("r", 0, 50)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect();
    assert!(shown.contains(&"other text".to_string()), "{shown:?}");
}
