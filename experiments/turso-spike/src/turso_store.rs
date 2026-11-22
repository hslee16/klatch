//! `TursoStore`: klatch's `Store` over Turso Database. The seam is synchronous and Turso's API is async, so each store
//! owns a current-thread tokio runtime and blocks on it (one connection, one thread, like the SQLite store).

use klatch::domain::{
    Attribution, AuditRecord, Claim, Error, MemberInfo, Message, Posted, RefusalRecord, Result,
    overlaps,
};
use klatch::store::{
    AskStore, ClaimRequest, ClaimStore, CredentialRecord, CredentialStore, Feature, Features,
    MemberStore, MessageStore, NewAudit, NewCredential, NewMessage, NewRefusal, PolicyStore,
    ReviewStore, RoomStore, Seat, Seq,
};
use std::sync::atomic::{AtomicU32, Ordering};
use turso::{Builder, EncryptionOpts, Value};

/// How the file is encrypted: Turso's native page encryption (experimental), a cipher name and a hex key.
#[derive(Clone)]
pub struct Encryption {
    pub cipher: String,
    pub hexkey: String,
}

pub struct TursoStore {
    rt: tokio::runtime::Runtime,
    conn: turso::Connection,
    _db: turso::Database,
}

fn be(e: impl std::fmt::Display) -> Error {
    Error::Backend(e.to_string())
}

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS members(tenant TEXT NOT NULL DEFAULT 'local', room TEXT, name TEXT, joined INTEGER, last_seen INTEGER, last_seq INTEGER DEFAULT 0, hook_seq INTEGER NOT NULL DEFAULT 0, wait_seq INTEGER NOT NULL DEFAULT 0, wake_seq INTEGER NOT NULL DEFAULT 0, status TEXT, status_ts INTEGER NOT NULL DEFAULT 0, muted INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(room,name))",
    "CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT NOT NULL, seq INTEGER NOT NULL, mid TEXT NOT NULL, sender TEXT, member TEXT, body TEXT, ts INTEGER, attribution TEXT NOT NULL DEFAULT 'unattributed', label TEXT NOT NULL DEFAULT 'public', kind TEXT, payload TEXT)",
    "CREATE UNIQUE INDEX IF NOT EXISTS messages_room_seq ON messages(room, seq)",
    "CREATE UNIQUE INDEX IF NOT EXISTS messages_mid ON messages(mid)",
    "CREATE INDEX IF NOT EXISTS messages_sender_ts ON messages(sender, ts)",
    "CREATE TABLE IF NOT EXISTS admin_audit(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, action TEXT NOT NULL, target TEXT NOT NULL, detail TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS admin_audit_action_target_ts ON admin_audit(action, target, ts)",
    "CREATE TABLE IF NOT EXISTS policy(key TEXT PRIMARY KEY, value TEXT NOT NULL, ts INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS refusals(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, member TEXT NOT NULL, room TEXT NOT NULL, operation TEXT NOT NULL, rule TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS credentials(member TEXT PRIMARY KEY, token_hash BLOB NOT NULL, clearance TEXT NOT NULL, parent TEXT NOT NULL, issued INTEGER NOT NULL, expires INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0, idle INTEGER NOT NULL DEFAULT 0, cap INTEGER NOT NULL DEFAULT 0)",
    "CREATE TABLE IF NOT EXISTS claims(id INTEGER PRIMARY KEY AUTOINCREMENT, tenant TEXT NOT NULL DEFAULT 'local', room TEXT, path TEXT, owner TEXT, member TEXT, reason TEXT, expires INTEGER, scope TEXT NOT NULL DEFAULT '', anchor TEXT NOT NULL DEFAULT '', ttl INTEGER NOT NULL DEFAULT 0, since INTEGER NOT NULL DEFAULT 0, age_noted INTEGER NOT NULL DEFAULT 0)",
];

const MESSAGE_COLUMNS: &str = "mid,seq,sender,attribution,label,body,ts,kind,payload";

fn int(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        _ => 0,
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        _ => String::new(),
    }
}

fn opt_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    }
}

type Row = Vec<Value>;

fn message(r: &Row) -> Message {
    Message {
        id: text(&r[0]),
        seq: int(&r[1]),
        sender: text(&r[2]),
        attribution: if text(&r[3]) == "verified" {
            Attribution::Verified
        } else {
            Attribution::Unattributed
        },
        label: text(&r[4]),
        body: text(&r[5]),
        ts: int(&r[6]),
        kind: opt_text(&r[7]),
        payload: opt_text(&r[8]),
        advisory: false,
    }
}

/// A message id: 26 characters of Crockford base32 (48-bit millisecond time, then process bits and a counter), the
/// same shape as klatch's own ids.
fn new_id(now_ms: u64) -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    const A: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let bits = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
        & 0xFFFF_FFFF_FFFF;
    let low = ((bits as u128) << 32) | COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    let enc = |mut v: u128, n: usize| {
        let mut out = vec![b'0'; n];
        for s in out.iter_mut().rev() {
            *s = A[(v & 31) as usize];
            v >>= 5;
        }
        String::from_utf8(out).unwrap()
    };
    format!(
        "{}{}",
        enc((now_ms & 0xFFFF_FFFF_FFFF) as u128, 10),
        enc(low, 16)
    )
}

impl TursoStore {
    /// Open (creating when new) the database at `path`, encrypted when `enc` is given.
    pub fn open(path: &str, enc: Option<Encryption>) -> Result<Self> {
        Self::open_with(path, enc, false)
    }

    /// Open with the key klatch's key custody holds (32 bytes, hex encoded for Turso) and AEGIS-256.
    pub fn open_with_key(
        path: &str,
        key: &klatch::keys::Key,
        multiprocess_wal: bool,
    ) -> Result<Self> {
        let hexkey = key.expose().iter().map(|b| format!("{b:02x}")).collect();
        let enc = Encryption {
            cipher: "aegis256".into(),
            hexkey,
        };
        Self::open_with(path, Some(enc), multiprocess_wal)
    }

    /// Like `open`; `multiprocess_wal` turns on Turso's experimental multi-process WAL coordination.
    pub fn open_with(path: &str, enc: Option<Encryption>, multiprocess_wal: bool) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(be)?;
        let mut b = Builder::new_local(path);
        if let Some(e) = enc {
            b = b
                .experimental_encryption(true)
                .with_encryption(EncryptionOpts {
                    cipher: e.cipher,
                    hexkey: e.hexkey,
                });
        }
        if multiprocess_wal {
            b = b.experimental_multiprocess_wal(true);
        }
        let db = rt.block_on(b.build()).map_err(be)?;
        let conn = db.connect().map_err(be)?;
        let s = Self { rt, conn, _db: db };
        // a busy database waits for the other writer instead of failing at once
        s.conn
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(be)?;
        for ddl in SCHEMA {
            s.exec(ddl, vec![])?;
        }
        Ok(s)
    }

    /// A fresh store for the conformance suite: a new file in the temp directory, encrypted unless
    /// `TURSO_SPIKE_PLAIN` is set.
    pub fn for_test() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::var("TURSO_SPIKE_DIR")
            .unwrap_or_else(|_| std::env::temp_dir().display().to_string());
        let path = format!(
            "{dir}/turso-spike-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        );
        let enc = std::env::var("TURSO_SPIKE_PLAIN")
            .is_err()
            .then(|| Encryption {
                cipher: "aegis256".into(),
                hexkey: "5a".repeat(32),
            });
        Self::open(&path, enc).expect("open turso store")
    }

    pub fn exec(&self, sql: &str, p: Vec<Value>) -> Result<u64> {
        self.rt
            .block_on(self.conn.execute(sql, turso::params_from_iter(p)))
            .map_err(be)
    }

    pub fn rows(&self, sql: &str, p: Vec<Value>) -> Result<Vec<Row>> {
        self.rt
            .block_on(async {
                let mut rows = self.conn.query(sql, turso::params_from_iter(p)).await?;
                let mut out = Vec::new();
                while let Some(r) = rows.next().await? {
                    let n = r.column_count();
                    out.push(
                        (0..n)
                            .map(|i| r.get_value(i))
                            .collect::<turso::Result<Row>>()?,
                    );
                }
                Ok::<_, turso::Error>(out)
            })
            .map_err(be)
    }

    fn one_int(&self, sql: &str, p: Vec<Value>) -> Result<Option<i64>> {
        Ok(self.rows(sql, p)?.first().map(|r| int(&r[0])))
    }

    /// Run `f` in a write transaction (`BEGIN IMMEDIATE`, as the SQLite store does).
    fn immediate<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.exec("BEGIN IMMEDIATE", vec![])?;
        match f() {
            Ok(v) => {
                self.exec("COMMIT", vec![])?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.exec("ROLLBACK", vec![]);
                Err(e)
            }
        }
    }

    fn touch(&self, room: &str, me: &str, now: i64) -> Result<()> {
        self.exec(
            "INSERT INTO members(room,name,joined,last_seen) VALUES(?1,?2,?3,?3)
             ON CONFLICT(room,name) DO UPDATE SET last_seen=?3",
            vec![room.into(), me.into(), now.into()],
        )?;
        Ok(())
    }

    fn others_after(&self, room: &str, after: i64, member: &str) -> Result<Vec<Message>> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 AND sender<>?3 AND COALESCE(kind,'')<>'archived' ORDER BY seq"
        );
        Ok(self
            .rows(&sql, vec![room.into(), after.into(), member.into()])?
            .iter()
            .map(message)
            .collect())
    }

    fn read_cursor(&self, room: &str, member: &str) -> Result<i64> {
        Ok(self
            .one_int(
                "SELECT last_seq FROM members WHERE room=?1 AND name=?2",
                vec![room.into(), member.into()],
            )?
            .unwrap_or(0))
    }

    fn poll_past(&self, column: &str, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.touch(at.room, at.member, now)?;
        let sql =
            format!("SELECT COALESCE(last_seq,0), {column} FROM members WHERE room=?1 AND name=?2");
        let r = self.rows(&sql, vec![at.room.into(), at.member.into()])?;
        let (read, seen) = r.first().map_or((0, 0), |r| (int(&r[0]), int(&r[1])));
        self.others_after(at.room, read.max(seen), at.member)
    }

    fn move_forward(&self, column: &str, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.touch(at.room, at.member, now)?;
        let sql = format!("UPDATE members SET {column}=MAX({column},?3) WHERE room=?1 AND name=?2");
        self.exec(&sql, vec![at.room.into(), at.member.into(), seq.0.into()])?;
        Ok(())
    }
}

impl MessageStore for TursoStore {
    fn join(&self, room: &str, member: &str, now: i64) -> Result<()> {
        self.touch(room, member, now)
    }

    fn post_as(&self, m: &NewMessage, now: i64) -> Result<Posted> {
        self.immediate(|| {
            let seq = self
                .one_int(
                    "SELECT COALESCE(MAX(seq),0)+1 FROM messages WHERE room=?1",
                    vec![m.room.into()],
                )?
                .unwrap_or(1);
            let id = new_id((now.max(0) as u64) * 1000);
            let attribution = match m.attribution {
                Attribution::Verified => "verified",
                Attribution::Unattributed => "unattributed",
            };
            self.exec(
                "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label,kind,payload) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10)",
                vec![
                    m.room.into(),
                    seq.into(),
                    id.clone().into(),
                    m.sender.into(),
                    m.body.into(),
                    now.into(),
                    attribution.into(),
                    m.label.into(),
                    m.kind.into(),
                    m.payload.into(),
                ],
            )?;
            self.touch(m.room, m.sender, now)?;
            Ok(Posted { id, seq })
        })
    }

    fn read(&self, at: Seat, advance: bool, now: i64) -> Result<Vec<Message>> {
        self.touch(at.room, at.member, now)?;
        let last = self.read_cursor(at.room, at.member)?;
        let rows = self.others_after(at.room, last, at.member)?;
        if advance {
            let cursor = rows.last().map_or(last, |m| m.seq.max(last));
            self.exec(
                "UPDATE members SET last_seq=?3, last_seen=?4 WHERE room=?1 AND name=?2",
                vec![at.room.into(), at.member.into(), cursor.into(), now.into()],
            )?;
        }
        Ok(rows)
    }

    fn look(&self, at: Seat, _now: i64) -> Result<Vec<Message>> {
        let last = self.read_cursor(at.room, at.member)?;
        self.others_after(at.room, last, at.member)
    }

    fn posts_since(&self, member: &str, since: i64) -> Result<Vec<i64>> {
        Ok(self
            .rows(
                "SELECT ts FROM messages WHERE sender=?1 AND ts>=?2 ORDER BY ts",
                vec![member.into(), since.into()],
            )?
            .iter()
            .map(|r| int(&r[0]))
            .collect())
    }

    fn wait_poll(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.poll_past("wait_seq", at, now)
    }

    fn wait_seen(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.move_forward("wait_seq", at, seq, now)
    }

    fn wake_poll(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.poll_past("wake_seq", at, now)
    }

    fn wake_seen(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.move_forward("wake_seq", at, seq, now)
    }

    fn hook_read(&self, at: Seat, now: i64) -> Result<Vec<Message>> {
        self.immediate(|| {
            self.touch(at.room, at.member, now)?;
            let r = self.rows(
                "SELECT COALESCE(last_seq,0), hook_seq FROM members WHERE room=?1 AND name=?2",
                vec![at.room.into(), at.member.into()],
            )?;
            let (read, shown) = r.first().map_or((0, 0), |r| (int(&r[0]), int(&r[1])));
            let rows = self.others_after(at.room, read.max(shown), at.member)?;
            if let Some(last) = rows.last() {
                self.exec(
                    "UPDATE members SET hook_seq=?3 WHERE room=?1 AND name=?2",
                    vec![at.room.into(), at.member.into(), last.seq.into()],
                )?;
            }
            Ok(rows)
        })
    }

    fn tail(&self, room: &str, after_seq: i64, limit: usize) -> Result<Vec<Message>> {
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages WHERE room=?1 AND seq>?2 ORDER BY seq LIMIT ?3"
        );
        Ok(self
            .rows(
                &sql,
                vec![room.into(), after_seq.into(), (limit as i64).into()],
            )?
            .iter()
            .map(message)
            .collect())
    }

    fn head(&self, room: &str) -> Result<i64> {
        Ok(self
            .one_int(
                "SELECT COALESCE(MAX(seq),0) FROM messages WHERE room=?1",
                vec![room.into()],
            )?
            .unwrap_or(0))
    }

    fn advance_cursor(&self, at: Seat, seq: Seq, now: i64) -> Result<()> {
        self.immediate(|| {
            let head = self.head(at.room)?;
            let target = seq.0.min(head).max(0);
            self.exec(
                "INSERT INTO members(room,name,joined,last_seen,last_seq) VALUES(?1,?2,?4,?4,?3)
                 ON CONFLICT(room,name) DO UPDATE SET last_seq=MAX(last_seq,?3), last_seen=?4",
                vec![at.room.into(), at.member.into(), target.into(), now.into()],
            )?;
            Ok(())
        })
    }
}

impl MemberStore for TursoStore {
    fn members(&self, room: &str) -> Result<Vec<MemberInfo>> {
        Ok(self
            .rows(
                "SELECT name,last_seen,status,joined,status_ts FROM members WHERE room=?1 ORDER BY name",
                vec![room.into()],
            )?
            .iter()
            .map(|r| MemberInfo {
                name: text(&r[0]),
                joined: int(&r[3]),
                last_seen: int(&r[1]),
                status: opt_text(&r[2]),
                status_ts: int(&r[4]),
                parent: None,
                parked: None,
                kind: None,
                place: None,
                mode: None,
                advisory: false,
            })
            .collect())
    }

    fn rooms_of(&self, member: &str) -> Result<Vec<String>> {
        Ok(self
            .rows(
                "SELECT room FROM members WHERE name=?1 ORDER BY room",
                vec![member.into()],
            )?
            .iter()
            .map(|r| text(&r[0]))
            .collect())
    }

    fn set_muted(&self, at: Seat, muted: bool, now: i64) -> Result<()> {
        self.touch(at.room, at.member, now)?;
        self.exec(
            "UPDATE members SET muted=?3 WHERE room=?1 AND name=?2",
            vec![at.room.into(), at.member.into(), i64::from(muted).into()],
        )?;
        Ok(())
    }

    fn is_muted(&self, at: Seat) -> Result<bool> {
        Ok(self
            .one_int(
                "SELECT muted FROM members WHERE room=?1 AND name=?2",
                vec![at.room.into(), at.member.into()],
            )?
            .is_some_and(|m| m != 0))
    }
}

fn scopes_collide(a: &str, b: &str) -> bool {
    a == b || a == "*" || b == "*"
}

impl ClaimStore for TursoStore {
    fn claim(&self, q: &ClaimRequest, now: i64) -> Result<()> {
        self.immediate(|| {
            self.exec("DELETE FROM claims WHERE expires<=?1", vec![now.into()])?;
            let held = self.rows(
                "SELECT path,owner,expires,scope,since FROM claims WHERE room=?1",
                vec![q.room.into()],
            )?;
            for h in &held {
                let (path, owner, scope) = (text(&h[0]), text(&h[1]), text(&h[3]));
                if owner != q.owner && overlaps(&path, q.path) && scopes_collide(&scope, q.scope) {
                    let since = int(&h[4]);
                    return Err(Error::Conflict {
                        path,
                        owner,
                        secs_left: int(&h[2]) - now,
                        held_secs: if since > 0 { (now - since).max(0) } else { 0 },
                        deadlock: false,
                    });
                }
            }
            self.exec(
                "DELETE FROM claims WHERE room=?1 AND owner=?2 AND path=?3",
                vec![q.room.into(), q.owner.into(), q.path.into()],
            )?;
            self.exec(
                "INSERT INTO claims(room,path,owner,member,reason,expires,scope,anchor,ttl,since,age_noted) VALUES(?1,?2,?3,?3,?4,?5,?6,?7,?8,?9,0)",
                vec![
                    q.room.into(),
                    q.path.into(),
                    q.owner.into(),
                    q.reason.into(),
                    (now + q.ttl).into(),
                    q.scope.into(),
                    q.anchor.into(),
                    q.ttl.into(),
                    now.into(),
                ],
            )?;
            Ok(())
        })
    }

    fn release(&self, room: &str, owner: &str, path: &str) -> Result<()> {
        self.exec(
            "DELETE FROM claims WHERE room=?1 AND owner=?2 AND path=?3",
            vec![room.into(), owner.into(), path.into()],
        )?;
        Ok(())
    }

    fn claims(&self, room: &str, now: i64) -> Result<Vec<Claim>> {
        Ok(self
            .rows(
                "SELECT path,owner,reason,expires,scope FROM claims WHERE room=?1 AND expires>?2 ORDER BY path",
                vec![room.into(), now.into()],
            )?
            .iter()
            .map(|r| Claim {
                path: text(&r[0]),
                owner: text(&r[1]),
                reason: text(&r[2]),
                expires: int(&r[3]),
                scope: text(&r[4]),
            })
            .collect())
    }
}

impl AskStore for TursoStore {}
impl ReviewStore for TursoStore {}

fn credential(r: &Row) -> CredentialRecord {
    CredentialRecord {
        member: text(&r[0]),
        token_hash: match &r[1] {
            Value::Blob(b) => b.clone(),
            _ => Vec::new(),
        },
        clearance: text(&r[2]),
        parent: text(&r[3]),
        issued: int(&r[4]),
        expires: int(&r[5]),
        revoked: int(&r[6]) != 0,
        idle: int(&r[7]),
        cap: int(&r[8]),
    }
}

const CREDENTIAL_COLUMNS: &str =
    "member,token_hash,clearance,parent,issued,expires,revoked,idle,cap";

impl CredentialStore for TursoStore {
    fn issue_credential(&self, c: &NewCredential, now: i64) -> Result<()> {
        self.immediate(|| {
            let held = self.rows(
                "SELECT expires, revoked FROM credentials WHERE member=?1",
                vec![c.member.into()],
            )?;
            if held
                .first()
                .is_some_and(|h| int(&h[1]) == 0 && int(&h[0]) > now)
            {
                return Err(Error::Invalid(format!(
                    "{} already holds a live credential; revoke it first or wait for it to expire",
                    c.member
                )));
            }
            let life = if c.max_life > 0 { c.ttl.min(c.max_life) } else { c.ttl };
            let cap = if c.max_life > 0 { now + c.max_life } else { 0 };
            let idle = if c.max_life > 0 { c.ttl } else { 0 };
            self.exec(
                "INSERT INTO credentials(member,token_hash,clearance,parent,issued,expires,revoked,idle,cap) VALUES(?1,?2,?3,?4,?5,?6,0,?7,?8)
                 ON CONFLICT(member) DO UPDATE SET token_hash=?2, clearance=?3, parent=?4, issued=?5, expires=?6, revoked=0, idle=?7, cap=?8",
                vec![
                    c.member.into(),
                    c.token_hash.into(),
                    c.clearance.into(),
                    c.parent.into(),
                    now.into(),
                    (now + life).into(),
                    idle.into(),
                    cap.into(),
                ],
            )?;
            Ok(())
        })
    }

    fn credential(&self, member: &str) -> Result<Option<CredentialRecord>> {
        let sql = format!("SELECT {CREDENTIAL_COLUMNS} FROM credentials WHERE member=?1");
        Ok(self
            .rows(&sql, vec![member.into()])?
            .first()
            .map(credential))
    }

    fn revoke_credential(&self, member: &str, _now: i64) -> Result<bool> {
        Ok(self.exec(
            "UPDATE credentials SET revoked=1 WHERE member=?1 AND revoked=0",
            vec![member.into()],
        )? > 0)
    }

    fn credentials(&self) -> Result<Vec<CredentialRecord>> {
        let sql = format!("SELECT {CREDENTIAL_COLUMNS} FROM credentials ORDER BY member");
        Ok(self.rows(&sql, vec![])?.iter().map(credential).collect())
    }
}

fn refusal(r: &Row) -> RefusalRecord {
    RefusalRecord {
        ts: int(&r[0]),
        member: text(&r[1]),
        room: text(&r[2]),
        operation: text(&r[3]),
        rule: text(&r[4]),
    }
}

impl PolicyStore for TursoStore {
    fn record_audit(&self, e: &NewAudit, now: i64) -> Result<()> {
        self.exec(
            "INSERT INTO admin_audit(ts,action,target,detail) VALUES(?1,?2,?3,?4)",
            vec![
                now.into(),
                e.action.into(),
                e.target.into(),
                e.detail.into(),
            ],
        )?;
        Ok(())
    }

    fn audit_log(&self, limit: usize) -> Result<Vec<AuditRecord>> {
        Ok(self
            .rows(
                "SELECT ts,action,target,detail FROM admin_audit ORDER BY id DESC LIMIT ?1",
                vec![(limit as i64).into()],
            )?
            .iter()
            .map(|r| AuditRecord {
                ts: int(&r[0]),
                action: text(&r[1]),
                target: text(&r[2]),
                detail: text(&r[3]),
            })
            .collect())
    }

    fn last_audit_ts(&self, action: &str, target: &str, since: i64) -> Result<Option<i64>> {
        Ok(self
            .rows(
                "SELECT MAX(ts) FROM admin_audit WHERE action=?1 AND target=?2 AND ts>=?3",
                vec![action.into(), target.into(), since.into()],
            )?
            .first()
            .and_then(|r| match &r[0] {
                Value::Integer(i) => Some(*i),
                _ => None,
            }))
    }

    fn policy_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .rows("SELECT value FROM policy WHERE key=?1", vec![key.into()])?
            .first()
            .map(|r| text(&r[0])))
    }

    fn policy_set(&self, key: &str, value: Option<&str>, now: i64) -> Result<()> {
        match value {
            Some(v) => self.exec(
                "INSERT INTO policy(key,value,ts) VALUES(?1,?2,?3)
                 ON CONFLICT(key) DO UPDATE SET value=?2, ts=?3",
                vec![key.into(), v.into(), now.into()],
            )?,
            None => self.exec("DELETE FROM policy WHERE key=?1", vec![key.into()])?,
        };
        Ok(())
    }

    fn atomically(&self, act: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        self.immediate(act)
    }

    fn policy_list(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        Ok(self
            .rows(
                "SELECT key,value FROM policy WHERE substr(key,1,length(?1))=?1 ORDER BY key",
                vec![prefix.into()],
            )?
            .iter()
            .map(|r| (text(&r[0]), text(&r[1])))
            .collect())
    }

    fn record_refusal(&self, e: &NewRefusal, now: i64) -> Result<()> {
        self.exec(
            "INSERT INTO refusals(ts,member,room,operation,rule) VALUES(?1,?2,?3,?4,?5)",
            vec![
                now.into(),
                e.member.into(),
                e.room.into(),
                e.operation.into(),
                e.rule.into(),
            ],
        )?;
        Ok(())
    }

    fn refusals(&self, limit: usize) -> Result<Vec<RefusalRecord>> {
        Ok(self
            .rows(
                "SELECT ts,member,room,operation,rule FROM refusals ORDER BY id DESC LIMIT ?1",
                vec![(limit as i64).into()],
            )?
            .iter()
            .map(refusal)
            .collect())
    }

    fn refusals_of(&self, member: &str, limit: usize) -> Result<Vec<RefusalRecord>> {
        Ok(self
            .rows(
                "SELECT ts,member,room,operation,rule FROM refusals WHERE member=?1 ORDER BY id DESC LIMIT ?2",
                vec![member.into(), (limit as i64).into()],
            )?
            .iter()
            .map(refusal)
            .collect())
    }

    fn refusal_rules_since(&self, member: &str, since: i64) -> Result<Vec<String>> {
        Ok(self
            .rows(
                "SELECT rule FROM refusals WHERE member=?1 AND ts>=?2 ORDER BY id",
                vec![member.into(), since.into()],
            )?
            .iter()
            .map(|r| text(&r[0]))
            .collect())
    }
}
impl RoomStore for TursoStore {
    fn set_label_keys(&self, _keys: klatch::keys::LabelKeys) {}

    fn features(&self) -> Features {
        Features::none()
            .with(Feature::WaitPositions)
            .with(Feature::Mute)
    }
}
