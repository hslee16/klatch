//! Retention and the archive files: `retention`, `archive`, `restore-archive` and `archives`.

use super::keys::KeyCtx;
use super::route::ArchiveAct;
use super::*;
use klatch::keys::Key;
use std::path::{Path, PathBuf};

pub(super) fn archive_command(ctx: &KeyCtx, act: ArchiveAct) -> Result<()> {
    match act {
        ArchiveAct::Retention { room, days } => retention(ctx, &room, &days),
        ArchiveAct::Archive { room, all_rooms } => archive(ctx, room, all_rooms),
        ArchiveAct::RestoreArchive { file } => restore_archive(ctx, &file),
        ArchiveAct::Archives => archives(ctx),
    }
}

fn archive_dir(db: &Path) -> PathBuf {
    db.with_file_name("archive")
}

/// DAYS as a whole number of at least 1.
fn parse_days(days: &str) -> Result<i64> {
    days.parse()
        .ok()
        .filter(|n| *n >= 1)
        .ok_or_else(|| anyhow::anyhow!("days must be a whole number of at least 1, or off"))
}

fn retention(ctx: &KeyCtx, room: &str, days: &str) -> Result<()> {
    let store = custody::open_store(ctx.db)?;
    let key = format!("retention:{room}");
    if days == "off" {
        store.policy_set(&key, None, now())?;
        ctx.note("retention", &format!("{room} off"))?;
        println!("retention for {room} removed");
        return Ok(());
    }
    let n = parse_days(days)?;
    store.policy_set(&key, Some(&n.to_string()), now())?;
    ctx.note("retention", &format!("{room} {n} days"))?;
    println!("messages of {room} older than {n} days will be archived by `klatch admin archive`");
    Ok(())
}

fn restore_archive(ctx: &KeyCtx, file: &Path) -> Result<()> {
    let store = custody::open_store(ctx.db)?;
    let key = ctx.key_or("this store has no key; nothing to restore into")?;
    let n = store.restore_archive(file, &key)?;
    ctx.note("restore-archive", &file.to_string_lossy())?;
    println!("restored {n} messages from {}", file.display());
    Ok(())
}

fn archives(ctx: &KeyCtx) -> Result<()> {
    let dir = archive_dir(ctx.db);
    match custody::current_key(ctx.db)? {
        Some(key) => list_archive_files(&dir, &key),
        None if !dir.exists() => {
            println!("no archives in {}", dir.display());
            Ok(())
        }
        None => bail!(
            "{} exists but the store key is missing, so no file can be checked; see `klatch admin recover`",
            dir.display()
        ),
    }
}

/// The days of retention for a room: its own setting, else the `*` one.
fn retention_days(store: &SqliteStore, room: &str) -> Result<Option<i64>> {
    let own = store.policy_get(&format!("retention:{room}"))?;
    let days = match own {
        Some(d) => Some(d),
        None => store.policy_get("retention:*")?,
    };
    Ok(days.and_then(|d| d.parse::<i64>().ok()))
}

/// A character a file name may keep.
fn is_file_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.')
}

/// A room name made safe for a file name.
fn safe_file_stem(room: &str) -> String {
    room.chars()
        .map(|c| if is_file_safe(c) { c } else { '_' })
        .collect()
}

/// Make the archive directory, readable by the owner only.
fn ensure_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// One `admin archive` run: the files it made so far and the messages moved.
struct ArchiveRun<'a> {
    ctx: &'a KeyCtx<'a>,
    store: SqliteStore,
    key: Key,
    dir: PathBuf,
    at: i64,
    total: usize,
    made: Vec<PathBuf>,
}

impl ArchiveRun<'_> {
    /// Archive one room's messages older than `days` into a file of its own.
    fn archive_room(&mut self, room: &str, days: i64) -> Result<()> {
        ensure_private_dir(&self.dir)?;
        let file = self
            .dir
            .join(format!("{}-{}.db", safe_file_stem(room), self.at));
        let before = self.at - days * 86_400;
        match self.store.archive_before(room, before, &file, &self.key) {
            Ok(n) => {
                if n > 0 {
                    println!("{room}: archived {n} messages to {}", file.display());
                    self.total += n;
                    self.made.push(file);
                }
                Ok(())
            }
            Err(e) => {
                self.settle_after_failure(file);
                Err(e.into())
            }
        }
    }

    /// The file may be in place even when a later step failed (a store re-keyed under this process); it must not be
    /// left under a key that is no longer the store's.
    fn settle_after_failure(&mut self, file: PathBuf) {
        self.made.extend(file.exists().then_some(file));
        let db = self.ctx.db;
        let _ = custody::settle_archive_keys(db, &custody::key_home(db), &self.key, &self.made);
    }

    /// A key rotation that finished while this ran never saw these files (#629).
    fn finish(&self) -> Result<()> {
        let db = self.ctx.db;
        custody::settle_archive_keys(db, &custody::key_home(db), &self.key, &self.made)?;
        self.ctx
            .note("archive", &format!("{} messages", self.total))?;
        if self.total == 0 {
            println!("nothing is older than its retention");
        }
        Ok(())
    }
}

fn archive(ctx: &KeyCtx, room: Option<String>, all_rooms: bool) -> Result<()> {
    let store = custody::open_store(ctx.db)?;
    let key = ctx.key_or("this store has no key yet; nothing to archive")?;
    let rooms = if all_rooms {
        store.message_rooms()?
    } else {
        vec![room.unwrap_or_default()]
    };
    let mut run = ArchiveRun {
        ctx,
        dir: archive_dir(ctx.db),
        at: now(),
        total: 0,
        made: Vec::new(),
        store,
        key,
    };
    for r in rooms {
        match retention_days(&run.store, &r)? {
            Some(days) => run.archive_room(&r, days)?,
            None if all_rooms => {}
            None => bail!("{r} has no retention; set one first: klatch admin retention {r} <days>"),
        }
    }
    run.finish()
}

/// What `admin archives` totals up over the files it lists.
#[derive(Default)]
struct Totals {
    files: usize,
    messages: usize,
    bytes: u64,
    unusable: usize,
}

impl Totals {
    fn summary(&self) -> String {
        format!(
            "{} archive{}, {} messages, {} bytes, {} unusable; nothing is pruned automatically",
            self.files,
            if self.files == 1 { "" } else { "s" },
            self.messages,
            self.bytes,
            self.unusable
        )
    }
}

/// One archive file as its listing line, counted into `totals`.
fn archive_line(a: &klatch::store::ArchiveInfo, at: i64, totals: &mut Totals) -> String {
    use klatch::store::ArchiveState;
    let days = |t: i64| format!("{}d ago", (at - t).max(0) / 86_400);
    let size = format!("{} bytes", a.bytes);
    match &a.state {
        ArchiveState::Ok {
            rooms,
            messages,
            oldest,
            newest,
        } => {
            totals.files += 1;
            totals.messages += messages;
            totals.bytes += a.bytes;
            format!(
                "{}\tok\t{}\t{messages} messages\toldest {}, newest {}\t{size}",
                a.path.display(),
                rooms.join(","),
                days(*oldest),
                days(*newest)
            )
        }
        ArchiveState::Unreadable(why) => {
            totals.unusable += 1;
            format!("{}\tUNUSABLE\t{why}\t{size}", a.path.display())
        }
        ArchiveState::Partial => format!(
            "{}\tpartial\tleft by an interrupted archive; safe to remove\t{size}",
            a.path.display()
        ),
    }
}

/// `admin archives` (#487): one tab-separated line per file, then a total. Counts and ages only, never text. A file
/// that does not open, or whose pages fail the check, is shown as UNUSABLE with why and makes the exit status an error,
/// so a cron job notices; the listing still covers every other file first.
fn list_archive_files(dir: &Path, key: &Key) -> Result<()> {
    let found = klatch::store::list_archives(dir, key)?;
    if found.is_empty() {
        println!("no archives in {}", dir.display());
        return Ok(());
    }
    let at = now();
    let mut totals = Totals::default();
    for a in &found {
        println!("{}", archive_line(a, at, &mut totals));
    }
    println!("{}", totals.summary());
    if totals.unusable > 0 {
        bail!(
            "{} archive file(s) cannot be used; restore them from a backup",
            totals.unusable
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_name_becomes_a_safe_file_stem() {
        assert_eq!(safe_file_stem("run.1/a b-c"), "run.1_a_b-c");
    }

    fn info(state: klatch::store::ArchiveState) -> klatch::store::ArchiveInfo {
        klatch::store::ArchiveInfo {
            path: PathBuf::from("/a/lobby-1.db"),
            bytes: 100,
            modified: None,
            state,
        }
    }

    #[test]
    fn the_summary_counts_and_pluralizes() {
        let t = Totals {
            files: 1,
            messages: 3,
            bytes: 100,
            unusable: 0,
        };
        assert_eq!(
            t.summary(),
            "1 archive, 3 messages, 100 bytes, 0 unusable; nothing is pruned automatically"
        );
        let t = Totals {
            files: 2,
            messages: 5,
            bytes: 7,
            unusable: 1,
        };
        assert_eq!(
            t.summary(),
            "2 archives, 5 messages, 7 bytes, 1 unusable; nothing is pruned automatically"
        );
    }

    #[test]
    fn a_good_archive_is_a_line_with_its_ages_and_is_counted() {
        let mut t = Totals::default();
        let line = archive_line(
            &info(klatch::store::ArchiveState::Ok {
                rooms: vec!["a".into(), "b".into()],
                messages: 3,
                oldest: 0,
                newest: 86_400 * 2,
            }),
            86_400 * 5,
            &mut t,
        );
        assert_eq!(
            line,
            "/a/lobby-1.db\tok\ta,b\t3 messages\toldest 5d ago, newest 3d ago\t100 bytes"
        );
        assert_eq!((t.files, t.messages, t.bytes, t.unusable), (1, 3, 100, 0));
    }

    #[test]
    fn an_unreadable_archive_is_marked_and_counted_unusable_and_a_partial_one_is_not_counted() {
        let mut t = Totals::default();
        let line = archive_line(
            &info(klatch::store::ArchiveState::Unreadable("bad pages".into())),
            0,
            &mut t,
        );
        assert_eq!(line, "/a/lobby-1.db\tUNUSABLE\tbad pages\t100 bytes");
        assert_eq!((t.files, t.messages, t.bytes, t.unusable), (0, 0, 0, 1));
        let line = archive_line(&info(klatch::store::ArchiveState::Partial), 0, &mut t);
        assert_eq!(
            line,
            "/a/lobby-1.db\tpartial\tleft by an interrupted archive; safe to remove\t100 bytes"
        );
        assert_eq!((t.files, t.unusable), (0, 1));
    }

    /// A rotation finished after this run read the key and before `finish`: the store and the keystore hold a newer key
    /// and the files this run made must follow it (#629), the ones it made and only those (a room with nothing old
    /// enough made none).
    #[test]
    fn finish_moves_the_files_this_run_made_to_a_key_that_replaced_its_own() {
        use super::super::command_tests::{Env, this_test};
        let Some(e) = Env::begin(this_test!(), "settle") else {
            return;
        };
        e.old_messages("lobby", 2);
        e.old_messages("young", 1);
        let ctx = KeyCtx {
            db: &e.db(),
            who: Principal::verified("human"),
        };
        let mut run = ArchiveRun {
            ctx: &ctx,
            store: e.store(),
            key: e.key(),
            dir: archive_dir(&e.db()),
            at: now(),
            total: 0,
            made: Vec::new(),
        };
        run.archive_room("lobby", 1).unwrap();
        run.archive_room("young", 100_000).unwrap();
        assert_eq!((run.total, run.made.len()), (2, 1));
        let newer = Key::generate().unwrap();
        run.store.rekey(&newer).unwrap();
        let kh = custody::key_home(&e.db());
        kh.source.put(&kh.name, &newer).unwrap();
        run.finish().unwrap();
        let listed = klatch::store::list_archives(&e.archive_dir(), &newer).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(matches!(
            listed[0].state,
            klatch::store::ArchiveState::Ok { messages: 2, .. }
        ));
    }

    /// A run that fails on its file (one is already there) still moves a file of its own that a finished rotation
    /// missed to the newer key, and says why it failed.
    #[test]
    fn a_failed_archive_still_settles_the_file_to_the_newer_key() {
        use super::super::command_tests::{Env, this_test};
        let Some(e) = Env::begin(this_test!(), "settle-failure") else {
            return;
        };
        e.old_messages("lobby", 2);
        e.old_messages("other", 1);
        let ctx = KeyCtx {
            db: &e.db(),
            who: Principal::verified("human"),
        };
        let at = now();
        let mut run = ArchiveRun {
            ctx: &ctx,
            store: e.store(),
            key: e.key(),
            dir: archive_dir(&e.db()),
            at,
            total: 0,
            made: Vec::new(),
        };
        // a whole archive, under the key this run holds, sits where this run's file for `lobby` would go
        ensure_private_dir(&run.dir).unwrap();
        let taken = run.dir.join(format!("lobby-{at}.db"));
        run.store
            .archive_before("other", at, &taken, &run.key)
            .unwrap();
        let newer = Key::generate().unwrap();
        run.store.rekey(&newer).unwrap();
        let kh = custody::key_home(&e.db());
        kh.source.put(&kh.name, &newer).unwrap();
        let err = run.archive_room("lobby", 1).unwrap_err().to_string();
        assert!(err.contains("already exists"), "{err}");
        let listed = klatch::store::list_archives(&e.archive_dir(), &newer).unwrap();
        assert!(matches!(
            listed[0].state,
            klatch::store::ArchiveState::Ok { messages: 1, .. }
        ));
    }

    #[test]
    fn retention_days_must_be_a_whole_number_of_at_least_one() {
        assert_eq!(parse_days("7").unwrap(), 7);
        assert!(parse_days("0").is_err());
        assert!(parse_days("x").is_err());
    }
}
