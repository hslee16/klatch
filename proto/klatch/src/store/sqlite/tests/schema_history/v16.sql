-- The SCHEMA a fresh store was created from at store schema version 16: the constant as it stood in main at commit abd031e37867 (file proto/klatch-store/src/sqlite/schema.rs). Frozen; do not edit (#1785).
CREATE TABLE members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, hook_seq INTEGER NOT NULL DEFAULT 0, wait_seq INTEGER NOT NULL DEFAULT 0, status TEXT, muted INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(room,name));
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
 CREATE TABLE room_state(room TEXT PRIMARY KEY, state TEXT NOT NULL, ts INTEGER NOT NULL);
 CREATE TABLE member_blocks(room TEXT NOT NULL, member TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,member));
 CREATE TABLE admin_audit(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, detail TEXT NOT NULL);
