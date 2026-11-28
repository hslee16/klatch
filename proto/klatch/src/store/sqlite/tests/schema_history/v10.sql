-- The SCHEMA a fresh store was created from at store schema version 10: the constant as it stood in main at commit 04df1550e8da (file proto/klatch-store/src/sqlite.rs). Frozen; do not edit (#1785).
CREATE TABLE members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, hook_seq INTEGER NOT NULL DEFAULT 0, wait_seq INTEGER NOT NULL DEFAULT 0, status TEXT, PRIMARY KEY(room,name));
 CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER, attribution TEXT NOT NULL DEFAULT 'unattributed', label TEXT NOT NULL DEFAULT 'public');
 CREATE UNIQUE INDEX messages_room_seq ON messages(room, seq);
 CREATE UNIQUE INDEX messages_mid ON messages(mid);
 CREATE TABLE claims(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT, path TEXT, owner TEXT, member TEXT, reason TEXT, expires INTEGER);
 CREATE TABLE claims_seen(room TEXT NOT NULL, member TEXT NOT NULL, owner TEXT NOT NULL, path TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(room,member,owner,path));
 CREATE TABLE asks_reported(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));
 CREATE TABLE asks_escalated(room TEXT NOT NULL, member TEXT NOT NULL, seq INTEGER NOT NULL, PRIMARY KEY(room,member,seq));
 CREATE TABLE hook_runs(member TEXT NOT NULL, event TEXT NOT NULL, last_ts INTEGER NOT NULL, runs INTEGER NOT NULL, last_bytes INTEGER NOT NULL, PRIMARY KEY(member,event));
 CREATE TABLE claim_waits(room TEXT NOT NULL, member TEXT NOT NULL, path TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(room,member,path));
