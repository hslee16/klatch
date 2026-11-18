//! The archive and key commands through their command function, `key_command` (#727): each one is run against a
//! throwaway store with a throwaway key directory and its effect is asserted (the files, the rows, the keys and the
//! audit). A command that did nothing would fail here. They run in this process because an admin command needs a real
//! terminal at the command line; the environment is shared, so the tests take turns.

use super::route::{ArchiveAct, KeyAct, Keystore};
use super::*;
use klatch::keys::Key;
use klatch::store::areas::*;
use klatch::store::{ArchiveState, Seat, list_archives};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static ENV: Mutex<()> = Mutex::new(());

/// A scratch directory holding the store and its keys. Never the real keystore: `KLATCH_KEY_DIR` points inside it.
struct Env {
    _turn: MutexGuard<'static, ()>,
    dir: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Self {
        let turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("klatch-admin-cmd-{}-{tag}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("keys")).unwrap();
        // SAFETY: the tests of this module hold ENV while they run, and no other thread here reads these names.
        unsafe {
            std::env::set_var("KLATCH_KEY_DIR", dir.join("keys"));
            std::env::remove_var("KLATCH_IDENTITY_FILE");
        }
        Env { _turn: turn, dir }
    }

    fn db(&self) -> PathBuf {
        self.dir.join("rooms.db")
    }

    fn archive_dir(&self) -> PathBuf {
        self.dir.join("archive")
    }

    fn store(&self) -> SqliteStore {
        custody::open_store(&self.db()).unwrap()
    }

    fn key(&self) -> Key {
        custody::current_key(&self.db()).unwrap().expect("a key")
    }

    /// Post `n` messages of ts 10 to `room`, which is older than any retention.
    fn old_messages(&self, room: &str, n: i64) {
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

impl Drop for Env {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn mode(p: &std::path::Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

// retention

#[test]
fn retention_is_stored_and_audited_and_off_removes_it() {
    let e = Env::new("retention");
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
    let e = Env::new("retention-bad");
    assert!(e.retention("lobby", "0").is_err());
    assert!(e.retention("lobby", "soon").is_err());
    assert_eq!(e.store().policy_get("retention:lobby").unwrap(), None);
}

// archive

#[test]
fn archive_moves_old_messages_into_a_private_file_and_audits_it() {
    let e = Env::new("archive");
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
}

#[test]
fn archive_of_one_room_without_retention_is_refused_and_names_the_fix() {
    let e = Env::new("archive-none");
    e.old_messages("lobby", 1);
    let err = e.archive("lobby").unwrap_err().to_string();
    assert!(err.contains("klatch admin retention lobby"), "{err}");
    assert!(e.archive_files().is_empty());
    assert_eq!(e.live("lobby"), 1);
}

#[test]
fn archive_with_nothing_old_enough_writes_no_file_but_still_audits_the_run() {
    let e = Env::new("archive-young");
    e.old_messages("lobby", 2);
    e.retention("lobby", "100000").unwrap();
    e.archive("lobby").unwrap();
    assert!(e.archive_files().is_empty());
    assert_eq!(e.live("lobby"), 2);
    assert!(e.audited().contains(&"archive".to_string()));
}

#[test]
fn archive_all_uses_each_rooms_own_retention_then_the_star_one_and_skips_rooms_with_none() {
    let e = Env::new("archive-all");
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
    let e = Env::new("archive-all-skip");
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
    let e = Env::new("restore-archive");
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
    let e = Env::new("archives-ok");
    e.old_messages("lobby", 1);
    e.archive_cmd(ArchiveAct::Archives).unwrap();
    e.retention("lobby", "1").unwrap();
    e.archive("lobby").unwrap();
    e.archive_cmd(ArchiveAct::Archives).unwrap();
}

#[test]
fn archives_reports_a_corrupted_file_as_an_error_after_listing_the_rest() {
    let e = Env::new("archives-bad");
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
    let e = Env::new("archives-nokey");
    e.old_messages("lobby", 1);
    std::fs::create_dir_all(e.archive_dir()).unwrap();
    let kh = custody::key_home(&e.db());
    kh.source.delete(&kh.name).unwrap();
    let err = e.archive_cmd(ArchiveAct::Archives).unwrap_err().to_string();
    assert!(err.contains("klatch admin recover"), "{err}");
}

#[test]
fn archives_with_no_directory_and_no_key_is_fine() {
    let e = Env::new("archives-nothing");
    e.archive_cmd(ArchiveAct::Archives).unwrap();
}

// key commands

#[test]
fn recovery_needs_a_key_and_audits_the_showing() {
    let e = Env::new("recovery");
    assert!(e.key_cmd(KeyAct::Recovery).is_err(), "no key yet");
    drop(e.store());
    e.key_cmd(KeyAct::Recovery).unwrap();
    assert!(e.audited().contains(&"recovery-show".to_string()));
}

#[test]
fn recover_puts_the_key_back_from_its_code_and_audits_it() {
    let e = Env::new("recover");
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
    let e = Env::new("recover-label");
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
    let e = Env::new("recover-bad");
    let err = e.key_cmd(KeyAct::Recover {
        code: vec!["nonsense".into()],
        label: None,
    });
    assert!(err.is_err());
    assert!(custody::current_key(&e.db()).unwrap().is_none());
}

#[test]
fn seal_label_seals_it_makes_its_key_and_audits_it() {
    let e = Env::new("seal");
    e.key_cmd(KeyAct::SealLabel {
        label: "private".into(),
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

#[test]
fn backup_writes_a_file_that_opens_with_the_key_and_audits_it() {
    let e = Env::new("backup");
    e.old_messages("lobby", 2);
    let file = e.dir.join("backup.db");
    e.key_cmd(KeyAct::Backup { file: file.clone() }).unwrap();
    assert!(file.metadata().unwrap().len() > 0);
    assert!(SqliteStore::open(&file, &e.key()).is_ok());
    assert!(e.audited().contains(&"backup".to_string()));
}

#[test]
fn restore_brings_the_store_back_to_the_backup_and_keeps_the_old_one_aside() {
    let e = Env::new("restore");
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
    let e = Env::new("rotate");
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
    let e = Env::new("run-archive");
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
    let e = Env::new("nokey");
    let file = e.dir.join("b.db");
    assert!(e.key_cmd(KeyAct::Backup { file: file.clone() }).is_err());
    assert!(!file.exists());
    assert!(e.key_cmd(KeyAct::Restore { file }).is_err());
}
