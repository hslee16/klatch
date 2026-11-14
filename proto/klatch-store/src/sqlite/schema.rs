//! The SQLite schema and its migrations: `SCHEMA` for a fresh database and one `migrate_vN` step per version, run in
//! order by `migrate` inside the write lock that `SqliteStore::init` holds.

use super::be;
use crate::ids::new_id;
use klatch_core::Result;
use rusqlite::{Connection, params};

pub(super) const SCHEMA_VERSION: i64 = 18;

pub(super) const SCHEMA: &str = "CREATE TABLE members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, hook_seq INTEGER NOT NULL DEFAULT 0, wait_seq INTEGER NOT NULL DEFAULT 0, status TEXT, muted INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(room,name));
 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER, attribution TEXT NOT NULL DEFAULT 'unattributed', label TEXT NOT NULL DEFAULT 'public');
 CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
 CREATE UNIQUE INDEX messages_mid ON messages(mid);
 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT, path TEXT, owner TEXT, member TEXT, reason TEXT, expires INTEGER, scope TEXT NOT NULL DEFAULT '');
 CREATE TABLE claims_seen(room TEXT NOT NULL, member TEXT NOT NULL, owner TEXT NOT NULL, path TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,owner,path));
 CREATE TABLE asks_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));
 CREATE TABLE asks_escalated(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));
 CREATE TABLE hook_runs(member TEXT NOT NULL, event TEXT NOT NULL, last_ts INTEGER NOT NULL, runs INTEGER NOT NULL, last_bytes INTEGER NOT NULL, PRIMARY KEY(member,event));
 CREATE TABLE claim_waits(room TEXT NOT NULL, member TEXT NOT NULL, path TEXT NOT NULL, ts INTEGER NOT NULL, scope TEXT NOT NULL DEFAULT '', PRIMARY KEY(room,member,path));
 CREATE TABLE sessions(session_id TEXT PRIMARY KEY, member TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE member_scope(member TEXT PRIMARY KEY, scope TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE ask_leases(room TEXT NOT NULL, seq INTEGER NOT NULL, owner TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,seq));
 CREATE TABLE asks_lapse_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,seq,expires));
 CREATE TABLE claim_notes(room TEXT NOT NULL, path TEXT NOT NULL, owner TEXT NOT NULL, note TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,path));
 CREATE TABLE member_parent(member TEXT PRIMARY KEY, parent TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE room_state(room TEXT PRIMARY KEY, state TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE member_blocks(room TEXT NOT NULL, member TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,member));
 CREATE TABLE admin_audit(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, detail TEXT NOT NULL);
 CREATE TABLE policy(key TEXT PRIMARY KEY, value TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE refusals(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, member TEXT NOT NULL, room TEXT NOT NULL, operation TEXT NOT NULL, rule TEXT NOT NULL);";

pub(super) fn user_version(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(be)
}

pub(super) fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![name],
            |r| r.get(0),
        )
        .map_err(be)?;
    Ok(n > 0)
}

/// Bring a version 0 database (the prototype schema) up to version 1 in place, keeping messages,
/// cursors and claims.
pub(super) fn migrate_v0(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE messages ADD COLUMN tenant TEXT NOT NULL DEFAULT 'local';
         ALTER TABLE messages ADD COLUMN seq INTEGER;
         ALTER TABLE messages ADD COLUMN mid TEXT;
         ALTER TABLE messages ADD COLUMN member TEXT;
         UPDATE messages SET member = sender;
         UPDATE messages SET seq = (SELECT COUNT(*) FROM messages m2 WHERE m2.room = messages.room AND m2.id <= messages.id);
         ALTER TABLE members ADD COLUMN tenant TEXT NOT NULL DEFAULT 'local';
         ALTER TABLE members ADD COLUMN last_seq INTEGER DEFAULT 0;
         UPDATE members SET last_seq = (SELECT COUNT(*) FROM messages WHERE messages.room = members.room AND messages.id <= members.last_id);
         ALTER TABLE claims ADD COLUMN tenant TEXT NOT NULL DEFAULT 'local';
         ALTER TABLE claims ADD COLUMN member TEXT;
         UPDATE claims SET member = owner;",
    )
    .map_err(be)?;
    let rows: Vec<(i64, i64)> = {
        let mut st = conn
            .prepare("SELECT id, ts FROM messages ORDER BY id")
            .map_err(be)?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(be)?
            .collect::<std::result::Result<_, _>>()
            .map_err(be)?
    };
    for (id, ts) in rows {
        conn.execute(
            "UPDATE messages SET mid = ?1 WHERE id = ?2",
            params![new_id((ts.max(0) as u64) * 1000), id],
        )
        .map_err(be)?;
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
         CREATE UNIQUE INDEX messages_mid ON messages(mid);",
    )
    .map_err(be)?;
    Ok(())
}

/// Version 1 to 2: messages record whether the sender was proven and the label they were posted under.
/// History written before this is reported as unattributed and public, which is the truthful reading
/// because nothing recorded otherwise.
pub(super) fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE messages ADD COLUMN attribution TEXT NOT NULL DEFAULT 'unattributed';
         ALTER TABLE messages ADD COLUMN label TEXT NOT NULL DEFAULT 'public';",
    )
    .map_err(be)
}

/// Version 2 to 3: what each member has been shown of other members' claims, so a hook reports only what
/// changed. Nothing to backfill: a member that has seen nothing is shown every claim once.
pub(super) fn migrate_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE claims_seen(room TEXT NOT NULL, member TEXT NOT NULL, owner TEXT NOT NULL, path TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,owner,path));",
    )
    .map_err(be)
}

/// Version 3 to 4: how far a hook has shown each member's messages, apart from how far the member has read.
/// Existing members start at 0, which the hook treats as "no further than the read cursor".
pub(super) fn migrate_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE members ADD COLUMN hook_seq INTEGER NOT NULL DEFAULT 0;")
        .map_err(be)
}

/// Version 4 to 5: how far a `wait` has looked at each member's room, apart from the read cursor, so a filtered
/// wait does not wake again for a message it already skipped or returned while the message stays unread.
pub(super) fn migrate_v4(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE members ADD COLUMN wait_seq INTEGER NOT NULL DEFAULT 0;")
        .map_err(be)
}

/// Version 5 to 6: which stale asks were already reported to their asker, so each is reported once.
pub(super) fn migrate_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE asks_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));",
    )
    .map_err(be)
}

/// Version 6 to 7: the status a member sets for itself in a room (`done`, `busy`, ...), shown by `who` and the board.
pub(super) fn migrate_v6(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE members ADD COLUMN status TEXT;")
        .map_err(be)
}

/// Version 7 to 8: which asks were already reported to their asker as undoable by anyone here, so each is told once.
pub(super) fn migrate_v7(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE asks_escalated(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));",
    )
    .map_err(be)
}

/// Version 8 to 9: the latest run of each hook event per member (time, count, size printed), so a user can see
/// the hooks are alive without asking the session. Never any content.
pub(super) fn migrate_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE hook_runs(member TEXT NOT NULL, event TEXT NOT NULL, last_ts INTEGER NOT NULL, runs INTEGER NOT NULL, last_bytes INTEGER NOT NULL, PRIMARY KEY(member,event));",
    )
    .map_err(be)
}

/// Version 9 to 10: the claims members recently tried and could not take, so a conflict that closes a cycle of
/// waits can be named a deadlock. Short lived, never any content.
pub(super) fn migrate_v9(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE claim_waits(room TEXT NOT NULL, member TEXT NOT NULL, path TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,member,path));",
    )
    .map_err(be)
}

/// A member that tried to claim something and was refused is treated as still waiting for this long.
pub(super) const WAIT_WINDOW_SECS: i64 = 120;

/// Version 10 to 11: which member each harness session is, so a hook that runs after the harness dropped its own
/// record (the SessionEnd hook) can still find its member. A session id and a member name only.
pub(super) fn migrate_v10(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE sessions(session_id TEXT PRIMARY KEY, member TEXT NOT NULL, ts INTEGER NOT NULL);",
    )
    .map_err(be)
}

/// Version 11 to 12: the scope a claim lives in and the scope a member works in (DESIGN section 27), so two
/// repositories, or two bots, can claim the same path without colliding. Existing claims keep the empty scope.
pub(super) fn migrate_v11(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE claims ADD COLUMN scope TEXT NOT NULL DEFAULT '';
         ALTER TABLE claim_waits ADD COLUMN scope TEXT NOT NULL DEFAULT '';
         CREATE TABLE member_scope(member TEXT PRIMARY KEY, scope TEXT NOT NULL, ts INTEGER NOT NULL);",
    )
    .map_err(be)
}

/// Version 12 to 13: the last lease taken on each ask, and which lapses were already reported to the asker, so a
/// helper that failed or vanished after claiming an ask can be told to the asker once (DESIGN section 28).
pub(super) fn migrate_v12(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE ask_leases(room TEXT NOT NULL, seq INTEGER NOT NULL, owner TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,seq));
         CREATE TABLE asks_lapse_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,seq,expires));",
    )
    .map_err(be)
}

pub(super) fn migrate_v13(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE claim_notes(room TEXT NOT NULL, path TEXT NOT NULL, owner TEXT NOT NULL, note TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,path));",
    )
    .map_err(be)
}

pub(super) fn migrate_v14(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE room_state(room TEXT PRIMARY KEY, state TEXT NOT NULL, ts INTEGER NOT NULL);
         CREATE TABLE member_blocks(room TEXT NOT NULL, member TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,member));
         CREATE TABLE admin_audit(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, detail TEXT NOT NULL);",
    )
    .map_err(be)
}

/// Version 15 to 16: whether a member muted a room (ticket #20).
pub(super) fn migrate_v15(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE members ADD COLUMN muted INTEGER NOT NULL DEFAULT 0;")
        .map_err(be)
}

/// Version 16 to 17: which member's session started a member's (ticket #24).
pub(super) fn migrate_v16(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE member_parent(member TEXT PRIMARY KEY, parent TEXT NOT NULL, ts INTEGER NOT NULL);",
    )
    .map_err(be)
}

/// Version 17 to 18: the administrator's policy and the record of refused calls (epic #9).
pub(super) fn migrate_v17(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE policy(key TEXT PRIMARY KEY, value TEXT NOT NULL, ts INTEGER NOT NULL);
         CREATE TABLE refusals(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, member TEXT NOT NULL, room TEXT NOT NULL, operation TEXT NOT NULL, rule TEXT NOT NULL);",
    )
    .map_err(be)
}

type Step = fn(&Connection) -> Result<()>;

/// Every migration, indexed by the version it upgrades from: `MIGRATIONS[n]` takes a database at version `n` (or
/// older) to version `n + 1`.
const MIGRATIONS: [Step; 18] = [
    migrate_v0,
    migrate_v1,
    migrate_v2,
    migrate_v3,
    migrate_v4,
    migrate_v5,
    migrate_v6,
    migrate_v7,
    migrate_v8,
    migrate_v9,
    migrate_v10,
    migrate_v11,
    migrate_v12,
    migrate_v13,
    migrate_v14,
    migrate_v15,
    migrate_v16,
    migrate_v17,
];

/// Bring a database that is at `version` up to the current schema, one step at a time.
pub(super) fn migrate(conn: &Connection, version: i64) -> Result<()> {
    for (from, step) in MIGRATIONS.iter().enumerate() {
        if version <= from as i64 {
            step(conn)?;
        }
    }
    Ok(())
}

const _: () = assert!(MIGRATIONS.len() as i64 == SCHEMA_VERSION);
