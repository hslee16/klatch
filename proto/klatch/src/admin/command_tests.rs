//! The archive and key commands through their command function, `key_command` (#727): each one is run against a
//! throwaway store with a throwaway key directory and its effect is asserted (the files, the rows, the keys and the
//! audit). A command that did nothing would fail here. They call the command function because an admin command needs a
//! real terminal at the command line.

use super::route::{ArchiveAct, KeyAct, Keystore};
use super::*;
use klatch::keys::Key;
use klatch::store::areas::*;
use klatch::store::{ArchiveState, Seat, list_archives};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

/// A scratch directory holding the store and its keys. Never the real keystore: `KLATCH_KEY_DIR` points inside it.
///
/// The key directory is chosen by the environment, and a test process shares its environment between threads, so each
/// test runs itself again in a child process of its own with that environment (no `unsafe` needed): `begin` is
/// `None` in the parent, which runs the child and checks that it passed, and `Some` in the child, which does the work.
pub(super) struct Env {
    dir: PathBuf,
}

const CHILD_DIR: &str = "KLATCH_ADMIN_CMD_TEST_DIR";

/// The path of the test that calls it, as the test runner names it.
macro_rules! this_test {
    () => {{
        fn here() {}
        let full = std::any::type_name_of_val(&here);
        full.strip_prefix("klatch::")
            .and_then(|n| n.strip_suffix("::here"))
            .unwrap_or(full)
    }};
}
pub(super) use this_test;

impl Env {
    pub(super) fn begin(test: &str, tag: &str) -> Option<Self> {
        Self::begin_saying(test, tag, &[], &[])
    }

    /// `begin`, and in the parent the child's output must hold every line of `says` and none of `silent`.
    pub(super) fn begin_saying(
        test: &str,
        tag: &str,
        says: &[&str],
        silent: &[&str],
    ) -> Option<Self> {
        if let Some(dir) = std::env::var_os(CHILD_DIR) {
            return Some(Env { dir: dir.into() });
        }
        let dir = crate::test_dir::TestDir::new(&format!("admin-cmd-{tag}"));
        std::fs::create_dir_all(dir.join("keys")).unwrap();
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--test-threads=1", "--nocapture"])
            .env(CHILD_DIR, &*dir)
            .env("KLATCH_KEY_DIR", dir.join("keys"))
            .env_remove("KLATCH_IDENTITY_FILE")
            .output()
            .unwrap();
        assert!(
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("1 passed"),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let said = String::from_utf8_lossy(&out.stdout);
        for line in says {
            assert!(said.contains(line), "expected {line:?} in:\n{said}");
        }
        for line in silent {
            assert!(!said.contains(line), "did not expect {line:?} in:\n{said}");
        }
        None
    }

    pub(super) fn db(&self) -> PathBuf {
        self.dir.join("rooms.db")
    }

    pub(super) fn archive_dir(&self) -> PathBuf {
        self.dir.join("archive")
    }

    pub(super) fn store(&self) -> SqliteStore {
        custody::open_store(&self.db()).unwrap()
    }

    pub(super) fn key(&self) -> Key {
        custody::current_key(&self.db()).unwrap().expect("a key")
    }

    /// Post `n` messages of ts 10 to `room`, which is older than any retention.
    pub(super) fn old_messages(&self, room: &str, n: i64) {
        let store = self.store();
        for i in 0..n {
            store
                .post(Seat::new(room, "alice"), &format!("m{i}"), 10 + i)
                .unwrap();
        }
    }

    fn key_cmd(&self, act: KeyAct) -> Result<()> {
        keys::key_command(&self.db(), Keystore::Key(act))
    }

    fn archive_cmd(&self, act: ArchiveAct) -> Result<()> {
        keys::key_command(&self.db(), Keystore::Archive(act))
    }

    fn retention(&self, room: &str, days: &str) -> Result<()> {
        self.archive_cmd(ArchiveAct::Retention {
            room: room.into(),
            days: days.into(),
        })
    }

    fn archive(&self, room: &str) -> Result<()> {
        self.archive_cmd(ArchiveAct::Archive {
            room: Some(room.into()),
            all_rooms: false,
        })
    }

    fn archive_all(&self) -> Result<()> {
        self.archive_cmd(ArchiveAct::Archive {
            room: None,
            all_rooms: true,
        })
    }

    /// The details of the audit entries of `action`, oldest first.
    fn details_of(&self, action: &str) -> Vec<String> {
        let mut d: Vec<String> = self
            .store()
            .audit_log(100)
            .unwrap()
            .into_iter()
            .filter(|r| r.action == action)
            .map(|r| r.detail)
            .collect();
        d.reverse();
        d
    }

    /// The actions in the audit, oldest first.
    fn audited(&self) -> Vec<String> {
        let mut a: Vec<String> = self
            .store()
            .audit_log(100)
            .unwrap()
            .into_iter()
            .map(|r| r.action)
            .collect();
        a.reverse();
        a
    }

    /// The archive files that exist, by name.
    fn archive_files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.archive_dir())
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// The messages of `room` that are still readable in the store (an archived one stays behind as a tombstone).
    fn live(&self, room: &str) -> usize {
        self.store()
            .look(Seat::new(room, "reader"), 1_000_000)
            .unwrap()
            .iter()
            .filter(|m| m.kind.as_deref() != Some("archived"))
            .count()
    }

    fn archived_messages(&self) -> usize {
        list_archives(&self.archive_dir(), &self.key())
            .unwrap()
            .iter()
            .map(|a| match &a.state {
                ArchiveState::Ok { messages, .. } => *messages,
                _ => 0,
            })
            .sum()
    }
}

fn mode(p: &std::path::Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

// retention

#[test]
fn retention_is_stored_and_audited_and_off_removes_it() {
    let Some(e) = Env::begin(this_test!(), "retention") else {
        return;
    };
    e.retention("lobby", "7").unwrap();
    assert_eq!(
        e.store().policy_get("retention:lobby").unwrap().as_deref(),
        Some("7")
    );
    assert!(e.audited().contains(&"retention".to_string()));
    e.retention("lobby", "off").unwrap();
    assert_eq!(e.store().policy_get("retention:lobby").unwrap(), None);
}

#[test]
fn retention_refuses_days_that_are_not_a_whole_number_and_stores_nothing() {
    let Some(e) = Env::begin(this_test!(), "retention-bad") else {
        return;
    };
    assert!(e.retention("lobby", "0").is_err());
    assert!(e.retention("lobby", "soon").is_err());
    assert_eq!(e.store().policy_get("retention:lobby").unwrap(), None);
}

// archive

#[test]
fn archive_moves_old_messages_into_a_private_file_and_audits_it() {
    let Some(e) = Env::begin_saying(
        this_test!(),
        "archive",
        &["lobby: archived 3 messages to "],
        &["nothing is older"],
    ) else {
        return;
    };
    e.old_messages("lobby", 3);
    e.retention("lobby", "1").unwrap();
    // an archive directory left open to others is closed again
    std::fs::create_dir_all(e.archive_dir()).unwrap();
    std::fs::set_permissions(e.archive_dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    e.archive("lobby").unwrap();
    let files = e.archive_files();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(
        files[0].starts_with("lobby-") && files[0].ends_with(".db"),
        "{files:?}"
    );
    assert_eq!(mode(&e.archive_dir()), 0o700);
    assert_eq!(e.archived_messages(), 3);
    assert_eq!(e.live("lobby"), 0, "the messages left the store");
    assert!(e.audited().contains(&"archive".to_string()));
    assert_eq!(e.details_of("archive"), vec!["3 messages".to_string()]);
}

#[test]
fn archive_of_one_room_without_retention_is_refused_and_names_the_fix() {
    let Some(e) = Env::begin(this_test!(), "archive-none") else {
        return;
    };
    e.old_messages("lobby", 1);
    let err = e.archive("lobby").unwrap_err().to_string();
    assert!(err.contains("klatch admin retention lobby"), "{err}");
    assert!(e.archive_files().is_empty());
    assert_eq!(e.live("lobby"), 1);
}

#[test]
fn archive_with_nothing_old_enough_writes_no_file_but_still_audits_the_run() {
    let Some(e) = Env::begin_saying(
        this_test!(),
        "archive-young",
        &["nothing is older than its retention"],
        &["lobby: archived"],
    ) else {
        return;
    };
    e.old_messages("lobby", 2);
    e.retention("lobby", "100000").unwrap();
    e.archive("lobby").unwrap();
    assert!(e.archive_files().is_empty());
    assert_eq!(e.live("lobby"), 2);
    assert!(e.audited().contains(&"archive".to_string()));
}

#[test]
fn archive_all_uses_each_rooms_own_retention_then_the_star_one_and_skips_rooms_with_none() {
    let Some(e) = Env::begin(this_test!(), "archive-all") else {
        return;
    };
    e.old_messages("own-long", 1);
    e.old_messages("falls-back", 2);
    e.retention("own-long", "100000").unwrap();
    e.retention("*", "1").unwrap();
    e.archive_all().unwrap();
    // `falls-back` follows `*` and is archived; `own-long` keeps its own long retention and stays
    let files = e.archive_files();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(
        files.iter().any(|f| f.starts_with("falls-back-")),
        "{files:?}"
    );
    assert_eq!(e.archived_messages(), 2);
    assert_eq!((e.live("own-long"), e.live("falls-back")), (1, 0));
}

#[test]
fn archive_all_skips_a_room_with_no_retention_at_all() {
    let Some(e) = Env::begin(this_test!(), "archive-all-skip") else {
        return;
    };
    e.old_messages("kept", 2);
    e.old_messages("moved", 1);
    e.retention("moved", "1").unwrap();
    e.archive_all().unwrap();
    assert_eq!(e.archived_messages(), 1);
    assert_eq!((e.live("kept"), e.live("moved")), (2, 0));
}

// restore-archive and archives

#[test]
fn restore_archive_puts_the_archived_messages_back_and_audits_it() {
    let Some(e) = Env::begin(this_test!(), "restore-archive") else {
        return;
    };
    e.old_messages("lobby", 3);
    e.retention("lobby", "1").unwrap();
    e.archive("lobby").unwrap();
    let file = e.archive_dir().join(&e.archive_files()[0]);
    assert_eq!(e.live("lobby"), 0);
    e.archive_cmd(ArchiveAct::RestoreArchive { file }).unwrap();
    assert_eq!(e.live("lobby"), 3);
    assert!(e.audited().contains(&"restore-archive".to_string()));
}

#[test]
fn archives_lists_a_good_directory_and_an_empty_or_missing_one() {
    let Some(e) = Env::begin(this_test!(), "archives-ok") else {
        return;
    };
    e.old_messages("lobby", 1);
    e.archive_cmd(ArchiveAct::Archives).unwrap();
    e.retention("lobby", "1").unwrap();
    e.archive("lobby").unwrap();
    e.archive_cmd(ArchiveAct::Archives).unwrap();
}

#[test]
fn archives_reports_a_corrupted_file_as_an_error_after_listing_the_rest() {
    let Some(e) = Env::begin(this_test!(), "archives-bad") else {
        return;
    };
    e.old_messages("lobby", 2);
    e.retention("lobby", "1").unwrap();
    e.archive("lobby").unwrap();
    let file = e.archive_dir().join(&e.archive_files()[0]);
    let mut bytes = std::fs::read(&file).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&file, bytes).unwrap();
    let err = e.archive_cmd(ArchiveAct::Archives).unwrap_err().to_string();
    assert!(err.contains("1 archive file(s) cannot be used"), "{err}");
}

#[test]
fn archives_with_a_directory_but_no_key_is_an_error_naming_recover() {
    let Some(e) = Env::begin(this_test!(), "archives-nokey") else {
        return;
    };
    e.old_messages("lobby", 1);
    std::fs::create_dir_all(e.archive_dir()).unwrap();
    let kh = custody::key_home(&e.db());
    kh.source.delete(&kh.name).unwrap();
    let err = e.archive_cmd(ArchiveAct::Archives).unwrap_err().to_string();
    assert!(err.contains("klatch admin recover"), "{err}");
}

#[test]
fn archives_with_no_directory_and_no_key_is_fine() {
    let Some(e) = Env::begin(this_test!(), "archives-nothing") else {
        return;
    };
    e.archive_cmd(ArchiveAct::Archives).unwrap();
}

// key commands

#[test]
fn the_integrity_check_finds_the_archive_of_every_tombstone_and_names_one_that_is_gone() {
    let Some(e) = Env::begin(this_test!(), "integrity") else {
        return;
    };
    e.old_messages("lobby", 3);
    e.retention("lobby", "1").unwrap();
    e.archive("lobby").unwrap();
    drop(e.store());
    let sound = klatch::store::check_store(&e.db(), &e.key()).unwrap();
    assert!(sound.is_empty(), "{sound:?}");
    // an interrupted copy is not an archive and a file that is not `.db` is not one either
    let file = e.archive_dir().join(&e.archive_files()[0]);
    let moved = e.dir.join("moved.db");
    std::fs::rename(&file, &moved).unwrap();
    std::fs::write(e.archive_dir().join("lobby-9.db.partial"), "half").unwrap();
    let lost = klatch::store::check_store(&e.db(), &e.key()).unwrap();
    assert_eq!(lost.len(), 3, "{lost:?}");
    assert!(
        lost[0].contains("lobby#") && lost[0].contains("no message in any archive"),
        "{lost:?}"
    );
}

#[test]
fn an_archive_directory_that_cannot_be_read_is_an_error_not_an_empty_listing() {
    let Some(e) = Env::begin(this_test!(), "archives-unreadable") else {
        return;
    };
    e.old_messages("lobby", 1);
    // a file where the directory should be: reading it fails with something other than "not found"
    std::fs::write(e.archive_dir(), "not a directory").unwrap();
    let err = e.archive_cmd(ArchiveAct::Archives).unwrap_err().to_string();
    assert!(err.contains("archive"), "{err}");
    // and a rotation refuses before it changes anything
    let key = e.key();
    assert!(e.key_cmd(KeyAct::RotateKey).is_err());
    assert!(e.key() == key, "the key is unchanged");
}

#[test]
fn recovery_needs_a_key_and_audits_the_showing() {
    let Some(e) = Env::begin(this_test!(), "recovery") else {
        return;
    };
    assert!(e.key_cmd(KeyAct::Recovery).is_err(), "no key yet");
    drop(e.store());
    e.key_cmd(KeyAct::Recovery).unwrap();
    assert!(e.audited().contains(&"recovery-show".to_string()));
}

#[test]
fn recover_puts_the_key_back_from_its_code_and_audits_it() {
    let Some(e) = Env::begin(this_test!(), "recover") else {
        return;
    };
    let key = Key::generate().unwrap();
    let code = klatch::keys::recovery_code(&key);
    let words: Vec<String> = code.split(' ').map(String::from).collect();
    assert!(custody::current_key(&e.db()).unwrap().is_none());
    e.key_cmd(KeyAct::Recover {
        code: words,
        label: None,
    })
    .unwrap();
    assert!(custody::current_key(&e.db()).unwrap() == Some(key));
    assert!(e.audited().contains(&"recover-key".to_string()));
}

#[test]
fn recover_with_a_label_puts_back_that_labels_key_only() {
    let Some(e) = Env::begin(this_test!(), "recover-label") else {
        return;
    };
    let key = Key::generate().unwrap();
    let code = klatch::keys::recovery_code(&key);
    let words: Vec<String> = code.split(' ').map(String::from).collect();
    e.key_cmd(KeyAct::Recover {
        code: words,
        label: Some("private".into()),
    })
    .unwrap();
    let label = custody::label_key(&e.db(), "private")
        .unwrap()
        .expect("the label key");
    assert_eq!(klatch::keys::recovery_code(&label), code);
    // the store key is a different one (the audit note made it), never the label's
    let store_key = custody::current_key(&e.db()).unwrap().expect("a store key");
    assert_ne!(klatch::keys::recovery_code(&store_key), code);
}

#[test]
fn recover_refuses_a_bad_code_and_changes_nothing() {
    let Some(e) = Env::begin(this_test!(), "recover-bad") else {
        return;
    };
    let err = e.key_cmd(KeyAct::Recover {
        code: vec!["nonsense".into()],
        label: None,
    });
    assert!(err.is_err());
    assert!(custody::current_key(&e.db()).unwrap().is_none());
}

#[test]
fn sealing_a_label_makes_its_key_and_audits_it() {
    let Some(e) = Env::begin(this_test!(), "seal") else {
        return;
    };
    e.key_cmd(KeyAct::SealLabel {
        label: "private".into(),
        keep_plain: false,
    })
    .unwrap();
    let gateway = Gateway::new(e.store(), CliClock, AllowAll);
    assert_eq!(
        gateway.sealed_labels().unwrap(),
        vec!["private".to_string()]
    );
    assert!(custody::label_key(&e.db(), "private").unwrap().is_some());
    assert!(e.audited().iter().any(|a| a.contains("seal")));
}

impl Env {
    /// Post `body` to room `r` under `label` as plain text, before the label is sealed.
    fn post_under(&self, label: &str, body: &str, ts: i64) {
        use klatch::domain::Attribution;
        self.store()
            .post_as(
                &klatch::store::NewMessage::new(
                    ("r", "alice"),
                    Attribution::Unattributed,
                    label,
                    body,
                ),
                ts,
            )
            .unwrap();
    }

    /// The messages of room `r` as a holder of every label key reads them.
    fn read_with_keys(&self) -> Vec<String> {
        let store = self.store();
        store.set_label_keys(custody::label_ring(&self.db(), &["private".to_string()]).unwrap());
        store
            .tail("r", 0, 50)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect()
    }

    /// The messages of room `r` as a holder of no label key reads them.
    fn read_without_keys(&self) -> Vec<String> {
        self.store()
            .tail("r", 0, 50)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect()
    }
}

#[test]
fn sealing_a_label_converts_the_messages_already_in_it_by_default() {
    let Some(e) = Env::begin_saying(
        this_test!(),
        "seal-convert",
        &["converted 2 earlier message text(s)"],
        &[],
    ) else {
        return;
    };
    e.post_under("private", "earlier one", 1);
    e.post_under("private", "earlier two", 2);
    e.post_under("public", "left plain", 3);
    e.key_cmd(KeyAct::SealLabel {
        label: "private".into(),
        keep_plain: false,
    })
    .unwrap();
    let blind = e.read_without_keys();
    assert!(
        blind[0].contains("sealed") && blind[1].contains("sealed"),
        "{blind:?}"
    );
    assert_eq!(blind[2], "left plain");
    assert_eq!(e.read_with_keys()[..2], ["earlier one", "earlier two"]);
    assert!(e.audited().contains(&"seal-label".to_string()));
}

#[test]
fn sealing_a_label_also_seals_its_plain_text_in_earlier_archive_files() {
    // #1153: an archive made before the label was sealed must not keep, or bring back, plain text
    let Some(e) = Env::begin(this_test!(), "seal-archives") else {
        return;
    };
    e.post_under("private", "archived earlier", 10);
    e.retention("r", "1").unwrap();
    e.archive("r").unwrap();
    let file = e.archive_dir().join(&e.archive_files()[0]);
    let raw = |file: &std::path::Path| -> String {
        let c = rusqlite::Connection::open(file).unwrap();
        c.pragma_update(None, "key", e.key().sqlcipher_pragma().as_str())
            .unwrap();
        c.query_row("SELECT body FROM archived", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(raw(&file), "archived earlier");
    e.key_cmd(KeyAct::SealLabel {
        label: "private".into(),
        keep_plain: false,
    })
    .unwrap();
    assert!(
        klatch::keys::is_sealed_text(&raw(&file)),
        "the archive kept plain text"
    );
    e.archive_cmd(ArchiveAct::RestoreArchive { file }).unwrap();
    assert_eq!(e.read_with_keys(), ["archived earlier"]);
    assert!(e.read_without_keys()[0].contains("sealed"));
}

#[test]
fn sealing_a_label_with_keep_plain_leaves_the_earlier_messages_as_they_were() {
    let Some(e) = Env::begin_saying(
        this_test!(),
        "seal-keep-plain",
        &["earlier messages were left as plain text"],
        &[],
    ) else {
        return;
    };
    e.post_under("private", "earlier one", 1);
    e.key_cmd(KeyAct::SealLabel {
        label: "private".into(),
        keep_plain: true,
    })
    .unwrap();
    assert_eq!(e.read_without_keys(), ["earlier one"]);
}

#[test]
fn sealing_a_label_again_converts_what_a_keep_plain_run_left() {
    let Some(e) = Env::begin(this_test!(), "seal-again") else {
        return;
    };
    e.post_under("private", "earlier one", 1);
    let seal = |keep_plain| {
        e.key_cmd(KeyAct::SealLabel {
            label: "private".into(),
            keep_plain,
        })
    };
    seal(true).unwrap();
    seal(false).unwrap();
    assert!(e.read_without_keys()[0].contains("sealed"));
    assert_eq!(e.read_with_keys(), ["earlier one"]);
}

#[test]
fn sealing_the_lowest_or_an_unknown_label_makes_no_key() {
    let Some(e) = Env::begin(this_test!(), "seal-refused") else {
        return;
    };
    for label in ["public", "no-such-label"] {
        let err = e
            .key_cmd(KeyAct::SealLabel {
                label: label.into(),
                keep_plain: false,
            })
            .unwrap_err()
            .to_string();
        assert!(!err.is_empty());
        assert!(
            custody::label_key(&e.db(), label).unwrap().is_none(),
            "{label}"
        );
    }
}

#[test]
fn rotate_label_key_moves_the_messages_audits_it_and_says_what_to_do_next() {
    let Some(e) = Env::begin_saying(
        this_test!(),
        "rotate-label",
        &[
            "relaunch workers that hold this label",
            "New recovery code for label private:",
            "Backups and archive copies made earlier",
        ],
        &[],
    ) else {
        return;
    };
    e.post_under("private", "earlier one", 1);
    e.key_cmd(KeyAct::SealLabel {
        label: "private".into(),
        keep_plain: false,
    })
    .unwrap();
    let old = custody::label_key(&e.db(), "private").unwrap().unwrap();
    e.key_cmd(KeyAct::RotateLabelKey {
        label: "private".into(),
    })
    .unwrap();
    let new = custody::label_key(&e.db(), "private").unwrap().unwrap();
    assert!(old != new, "the label key did not change");
    assert_eq!(e.read_with_keys(), ["earlier one"]);
    assert_eq!(
        e.store().label_open_counts("private", &old, &new).unwrap(),
        (1, 0, 1)
    );
    assert_eq!(e.details_of("rotate-label-key"), ["private"]);
}

#[test]
fn rotate_label_key_refuses_the_lowest_label_an_unsealed_one_and_an_unknown_one() {
    let Some(e) = Env::begin(this_test!(), "rotate-label-refused") else {
        return;
    };
    for label in ["public", "private", "no-such-label"] {
        let err = e
            .key_cmd(KeyAct::RotateLabelKey {
                label: label.into(),
            })
            .unwrap_err()
            .to_string();
        assert!(!err.is_empty(), "{label}");
        assert!(
            custody::label_key(&e.db(), label).unwrap().is_none(),
            "{label}"
        );
    }
    assert!(!e.audited().contains(&"rotate-label-key".to_string()));
}

#[test]
fn backup_writes_a_file_that_opens_with_the_key_and_audits_it() {
    let Some(e) = Env::begin(this_test!(), "backup") else {
        return;
    };
    e.old_messages("lobby", 2);
    let file = e.dir.join("backup.db");
    e.key_cmd(KeyAct::Backup { file: file.clone() }).unwrap();
    assert!(file.metadata().unwrap().len() > 0);
    assert!(SqliteStore::open(&file, &e.key()).is_ok());
    assert!(e.audited().contains(&"backup".to_string()));
}

#[test]
fn restore_brings_the_store_back_to_the_backup_and_keeps_the_old_one_aside() {
    let Some(e) = Env::begin(this_test!(), "restore") else {
        return;
    };
    e.old_messages("lobby", 1);
    let file = e.dir.join("backup.db");
    e.key_cmd(KeyAct::Backup { file: file.clone() }).unwrap();
    e.old_messages("later", 1);
    assert_eq!(e.store().message_rooms().unwrap().len(), 2);
    e.key_cmd(KeyAct::Restore { file }).unwrap();
    assert_eq!(
        e.store().message_rooms().unwrap(),
        vec!["lobby".to_string()]
    );
    assert!(e.dir.join("rooms.db.before-restore").exists());
    assert!(e.audited().contains(&"restore".to_string()));
}

#[test]
fn rotate_key_changes_the_key_and_moves_the_archives_to_it() {
    let Some(e) = Env::begin(this_test!(), "rotate") else {
        return;
    };
    e.old_messages("lobby", 2);
    e.retention("lobby", "1").unwrap();
    e.archive("lobby").unwrap();
    let old = e.key();
    e.key_cmd(KeyAct::RotateKey).unwrap();
    let new = e.key();
    assert!(old != new, "the key changed");
    assert!(SqliteStore::open(&e.db(), &new).is_ok());
    assert!(
        SqliteStore::open(&e.db(), &old).is_err(),
        "the old key no longer opens the store"
    );
    assert_eq!(
        e.archived_messages(),
        2,
        "the archive opens with the new key"
    );
    assert!(e.audited().contains(&"rotate-key".to_string()));
}

#[test]
fn run_archive_reads_an_archive_of_a_run_and_audits_it() {
    let Some(e) = Env::begin(this_test!(), "run-archive") else {
        return;
    };
    drop(e.store());
    let file = e.dir.join("run-1.archive");
    crate::run::write_archive(&file, &e.key(), "r1", &[]).unwrap();
    e.key_cmd(KeyAct::RunArchive {
        file: file.clone(),
        run: "r1".into(),
    })
    .unwrap();
    assert!(e.audited().contains(&"run-archive".to_string()));
    // another run's id does not open it, and nothing more is audited
    let before = e.audited().len();
    assert!(
        e.key_cmd(KeyAct::RunArchive {
            file,
            run: "other".into()
        })
        .is_err()
    );
    assert_eq!(e.audited().len(), before);
}

#[test]
fn restore_and_backup_without_a_key_say_so() {
    let Some(e) = Env::begin(this_test!(), "nokey") else {
        return;
    };
    let file = e.dir.join("b.db");
    assert!(e.key_cmd(KeyAct::Backup { file: file.clone() }).is_err());
    assert!(!file.exists());
    assert!(e.key_cmd(KeyAct::Restore { file }).is_err());
}
