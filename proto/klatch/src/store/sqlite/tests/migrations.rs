use super::*;
use crate::domain::Attribution;

fn version(s: &SqliteStore) -> i64 {
    s.conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

/// A database written by the original single-file prototype (commit 46e4e08, schema version 0):
/// room `core` with bob's messages alpha, beta, gamma and alice's cursor after beta; room `side` with
/// carol's `other`; and bob's claim on `src/a.rs`. Regenerate only by building that commit.
const PROTOTYPE_V0: &[u8] = include_bytes!("../../../../fixtures/store/prototype-v0.db");

/// A database written by the binary at schema version 2 (after PR #151): room `core`, alice's message
/// `hello from v2` and bob's claim on `src/lib.rs`. Regenerate only by building that release.
const RELEASE_V2: &[u8] = include_bytes!("../../../../fixtures/store/prototype-v2.db");

#[test]
fn an_old_database_is_migrated_in_place() {
    let path = temp_db("migrate");
    {
        let c = keyed(&path);
        c.execute_batch(
            "CREATE TABLE members(room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_id INTEGER DEFAULT 0, PRIMARY KEY(room,name));
             CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, sender TEXT, body TEXT, ts INTEGER);
             CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, path TEXT, owner TEXT, reason TEXT, expires INTEGER);
             INSERT INTO messages(room,sender,body,ts) VALUES('r','bob','m1',1),('r','bob','m2',2),('r2','bob','m3',3),('r','bob','m4',4);
             INSERT INTO members(room,name,joined,last_seen,last_id) VALUES('r','alice',1,1,2);
             INSERT INTO claims(room,path,owner,reason,expires) VALUES('r','src','bob','why',100);",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let unread = s.read(Seat::new("r", "alice"), false, 5).unwrap();
    assert_eq!(unread.len(), 1, "cursor must survive the migration");
    assert_eq!((unread[0].body.as_str(), unread[0].seq), ("m4", 3));
    assert_eq!(unread[0].id.len(), 26);
    assert_eq!(s.post(Seat::new("r2", "bob"), "m5", 6).unwrap().seq, 2);
    assert_eq!(s.claims("r", 1).unwrap()[0].owner, "bob");
    let local: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE tenant='local' AND member=sender",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(local, 5);
    drop(s);
    SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap(); // reopening a migrated database is a no-op
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_25_database_gains_the_claim_ttl_and_its_old_claims_are_not_renewed() {
    let path = temp_db("migrate-v25-ttl");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "old.rs", 600, ""), 0)
            .unwrap();
        // Make it look like a store written before the column existed.
        s.conn
            .execute_batch("ALTER TABLE claims DROP COLUMN ttl; PRAGMA user_version = 25;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    assert_eq!(s.claims("r", 1).unwrap()[0].path, "old.rs");
    // Its original ttl is unknown (0), so activity leaves it to lapse at its own lease.
    s.touch("r", "bob", 500).unwrap();
    assert_eq!(s.claims("r", 1).unwrap()[0].expires, 600);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_26_database_gains_the_member_mode_table_and_keeps_its_data() {
    let path = temp_db("migrate-v26-mode");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.join("r", "alice", 1).unwrap();
        s.conn
            .execute_batch("DROP TABLE member_mode; PRAGMA user_version = 26;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    assert_eq!(s.members("r").unwrap()[0].name, "alice");
    assert!(s.member_modes().unwrap().is_empty());
    s.note_member_mode("alice", "default", 5).unwrap();
    assert_eq!(s.member_modes().unwrap().len(), 1);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_fresh_database_is_at_schema_version_33() {
    assert_eq!(version(&store()), 33);
}

#[test]
fn a_version_1_database_is_migrated_to_33_and_keeps_its_messages() {
    let path = temp_db("migrate-v1");
    {
        let c = keyed(&path);
        c.execute_batch(
            "CREATE TABLE members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, PRIMARY KEY(room,name));
             CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER);
             CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
             CREATE UNIQUE INDEX messages_mid ON messages(mid);
             CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT, path TEXT, owner TEXT, member TEXT, reason TEXT, expires INTEGER);
             INSERT INTO messages(room,seq,mid,sender,member,body,ts) VALUES('r',1,'01HZX3K9Q2ABCDEFGHJKMNPQRS','bob','bob','old one',1),('r',2,'01HZX3K9Q3ABCDEFGHJKMNPQRT','bob','bob','old two',2);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    let old = s.tail("r", 0, 10).unwrap();
    assert_eq!(old.len(), 2);
    assert!(
        old.iter()
            .all(|m| m.attribution == Attribution::Unattributed && m.label == "public"),
        "unknown history is reported as unattributed and public"
    );
    assert_eq!(old[0].id, "01HZX3K9Q2ABCDEFGHJKMNPQRS", "ids survive");
    let p = s
        .post_as(
            &NewMessage::new(("r", "carol"), Attribution::Verified, "internal", "new"),
            3,
        )
        .unwrap();
    assert_eq!(p.seq, 3);
    assert_eq!(
        s.tail("r", 2, 10).unwrap()[0].attribution,
        Attribution::Verified
    );
    drop(s);
    let again = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(
        version(&again),
        33,
        "reopening a migrated database is a no-op"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_0_database_now_migrates_all_the_way_to_23() {
    let path = temp_db("migrate-v0-to-2");
    {
        let c = keyed(&path);
        c.execute_batch(
            "CREATE TABLE members(room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_id INTEGER DEFAULT 0, PRIMARY KEY(room,name));
             CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, sender TEXT, body TEXT, ts INTEGER);
             CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, room TEXT, path TEXT, owner TEXT, reason TEXT, expires INTEGER);
             INSERT INTO messages(room,sender,body,ts) VALUES('r','bob','m1',1);",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    let m = &s.tail("r", 0, 10).unwrap()[0];
    assert_eq!(
        (m.attribution, m.label.as_str()),
        (Attribution::Unattributed, "public")
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_two_database_migrates_to_seventeen_with_everything_intact() {
    let path = temp_db("real-v2");
    std::fs::write(&path, RELEASE_V2).unwrap();
    crate::store::encrypt::ensure_encrypted(&path, &crate::store::insecure_test_key()).unwrap();
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    let core = s.tail("core", 0, 10).unwrap();
    assert_eq!(core.len(), 1);
    assert_eq!(core[0].body, "hello from v2");
    assert_eq!(s.claims("core", 1).unwrap().len(), 1);
    // the new table works on the migrated database
    let (added, released) = changes(&s, "core", "alice", 1);
    assert_eq!(added, vec![pair("bob", "src/lib.rs")]);
    assert!(released.is_empty());
    assert_eq!(changes(&s, "core", "alice", 2), (vec![], vec![]));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_database_written_by_the_real_prototype_migrates_with_everything_intact() {
    let path = temp_db("real-v0");
    std::fs::write(&path, PROTOTYPE_V0).unwrap();
    crate::store::encrypt::ensure_encrypted(&path, &crate::store::insecure_test_key()).unwrap();
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    // messages: per-room seq backfilled in the original order, ids assigned and distinct
    let core = s.tail("core", 0, 10).unwrap();
    assert_eq!(
        core.iter()
            .map(|m| (m.seq, m.sender.as_str(), m.body.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "bob", "alpha"), (2, "bob", "beta"), (3, "bob", "gamma")]
    );
    let side = s.tail("side", 0, 10).unwrap();
    assert_eq!((side[0].seq, side[0].body.as_str()), (1, "other"));
    let ids: std::collections::HashSet<_> = core
        .iter()
        .chain(side.iter())
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(ids.len(), 4);
    assert!(ids.iter().all(|id| id.len() == 26));
    assert!(
        core.iter()
            .chain(side.iter())
            .all(|m| m.attribution == Attribution::Unattributed && m.label == "public")
    );
    // alice had read through beta in the prototype, so only gamma is unread
    let unread = s.read(Seat::new("core", "alice"), false, 1).unwrap();
    assert_eq!(
        unread.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        vec!["gamma"],
        "the cursor survived"
    );
    // members, and the claim with its expiry
    let members: Vec<String> = s
        .members("core")
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(members, vec!["alice", "bob"]);
    assert_eq!(s.members("side").unwrap()[0].name, "carol");
    let expires: i64 = s
        .conn
        .query_row("SELECT expires FROM claims", [], |r| r.get(0))
        .unwrap();
    let claims = s.claims("core", expires - 100).unwrap();
    assert_eq!(
        (
            claims[0].owner.as_str(),
            claims[0].path.as_str(),
            claims[0].reason.as_str()
        ),
        ("bob", "src/a.rs", "prototype claim")
    );
    // the migrated database keeps working
    assert_eq!(
        s.post(Seat::new("core", "dave"), "after the migration", 5)
            .unwrap()
            .seq,
        4
    );
    let tenants: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE tenant='local' AND member=sender",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tenants, 5);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_27_database_gains_the_claim_age_and_its_old_claims_are_never_reported() {
    let path = temp_db("migrate-v27-age");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.claim(
            &ClaimRequest::new(("r", "bob"), "old.rs", 86_400, ""),
            1_000,
        )
        .unwrap();
        s.conn
            .execute_batch(
                "ALTER TABLE claims DROP COLUMN since; ALTER TABLE claims DROP COLUMN age_noted; PRAGMA user_version = 27;",
            )
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    assert!(s.claim_ages("r", "bob", 100_000).unwrap().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_28_database_gains_the_wait_mark_and_its_old_waits_tell_nobody() {
    let path = temp_db("migrate-v28-told");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.claim(&ClaimRequest::new(("r", "bob"), "f.rs", 1_000, ""), 10)
            .unwrap();
        assert!(
            s.claim(&ClaimRequest::new(("r", "alice"), "f.rs", 1_000, ""), 11)
                .is_err()
        );
        s.conn
            .execute_batch("ALTER TABLE claim_waits DROP COLUMN told; PRAGMA user_version = 28;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    assert!(s.deadlock_notices("r", "alice", 12).unwrap().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// How many times another connection has seen the file change: a commit that wrote nothing does not count.
fn data_version(c: &Connection) -> i64 {
    c.query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_store_already_at_the_current_version_is_not_written_to_when_opened() {
    let path = temp_db("gate-current");
    drop(SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap());
    let watcher = keyed(&path);
    let before = data_version(&watcher);
    let s = SqliteStore::init(keyed(&path)).unwrap();
    assert_eq!(version(&s), crate::store::sqlite::schema::SCHEMA_VERSION);
    assert_eq!(
        data_version(&watcher),
        before,
        "opening a current store wrote to it (it was migrated again)"
    );
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_store_one_version_behind_is_written_to_when_opened() {
    // the other side of the boundary: the same watcher sees the migration commit
    let path = temp_db("gate-behind");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.conn
            .execute_batch("ALTER TABLE claim_waits DROP COLUMN told; PRAGMA user_version = 28;")
            .unwrap();
    }
    let watcher = keyed(&path);
    let before = data_version(&watcher);
    let s = SqliteStore::init(keyed(&path)).unwrap();
    assert_eq!(version(&s), crate::store::sqlite::schema::SCHEMA_VERSION);
    assert_ne!(data_version(&watcher), before, "the migration did not run");
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_29_database_gains_the_member_attributes_table_and_keeps_its_data() {
    let path = temp_db("migrate-v29-attrs");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.join("r", "alice", 1).unwrap();
        s.set_member_scope("alice", "git:a", 2).unwrap();
        s.conn
            .execute_batch("DROP TABLE member_attrs; PRAGMA user_version = 29;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    assert_eq!(s.members("r").unwrap()[0].name, "alice");
    assert_eq!(s.member_scope("alice").unwrap().as_deref(), Some("git:a"));
    assert_eq!(s.member_attrs("alice").unwrap(), None);
    let attrs = crate::domain::MemberAttrs::new("m-1", "git:a-1", "/w/a");
    s.note_member_attrs("alice", &attrs, 5).unwrap();
    assert_eq!(s.member_attrs("alice").unwrap(), Some(attrs));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_version_30_database_gains_the_credential_renewal_columns_and_its_credential_never_renews() {
    let path = temp_db("migrate-v30-renewal");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.issue_credential(
            &crate::store::NewCredential {
                member: "w-1",
                token_hash: &[1; 32],
                clearance: "private",
                parent: "lead",
                ttl: 600,
                max_life: 0,
            },
            1_000,
        )
        .unwrap();
        // make it look like a store written before the columns existed
        s.conn
            .execute_batch(
                "ALTER TABLE credentials DROP COLUMN idle; ALTER TABLE credentials DROP COLUMN cap; PRAGMA user_version = 30;",
            )
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    let c = s.credential("w-1").unwrap().unwrap();
    assert_eq!((c.expires, c.idle, c.cap), (1_600, 0, 0));
    // an old credential lapses at its own expiry: use does not renew it
    assert_eq!(s.renew_credential("w-1", 1_300, 60).unwrap(), None);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_database_from_before_status_ages_gains_the_column_and_keeps_its_statuses() {
    let path = temp_db("migrate-v31-status-ts");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.set_status(Seat::new("r", "alice"), "done", 5).unwrap();
        s.conn
            .execute_batch("ALTER TABLE members DROP COLUMN status_ts; PRAGMA user_version = 31;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    let m = &s.members("r").unwrap()[0];
    assert_eq!((m.status.as_deref(), m.status_ts), (Some("done"), 0));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn a_database_from_before_the_sender_index_gains_it_and_keeps_its_messages() {
    let path = temp_db("migrate-v32-sender-ts");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.post(Seat::new("r", "bob"), "kept", 5).unwrap();
        s.conn
            .execute_batch("DROP INDEX messages_sender_ts; PRAGMA user_version = 32;")
            .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(version(&s), 33);
    let n: i64 = s
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='messages_sender_ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(s.posts_since("bob", 0).unwrap(), vec![5]);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}
