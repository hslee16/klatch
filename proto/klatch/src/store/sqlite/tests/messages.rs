use super::*;

#[test]
fn data_persists_across_connections_on_disk() {
    let path = temp_db("persist");
    {
        let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
        s.post(Seat::new("r", "bob"), "kept", 1).unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    s.join("r", "alice", 2).unwrap();
    assert_eq!(
        s.read(Seat::new("r", "alice"), true, 3).unwrap()[0].body,
        "kept"
    );
}
#[test]
fn look_returns_the_unread_messages_and_touches_neither_presence_nor_membership() {
    let s = store();
    s.join("r", "alice", 2).unwrap();
    s.post(Seat::new("r", "bob"), "hi", 3).unwrap();
    assert_eq!(s.look(Seat::new("r", "lib"), 50).unwrap().len(), 1);
    assert!(
        s.members("r").unwrap().iter().all(|m| m.name != "lib"),
        "a look made the reader a member"
    );
    assert_eq!(s.look(Seat::new("r", "alice"), 60).unwrap().len(), 1);
    assert_eq!(s.look(Seat::new("r", "alice"), 61).unwrap().len(), 1);
    let alice = s.members("r").unwrap();
    let alice = alice.iter().find(|m| m.name == "alice").unwrap();
    assert_eq!(alice.last_seen, 2, "a look refreshed presence");
    assert_eq!(
        s.read(Seat::new("r", "alice"), false, 62).unwrap().len(),
        1,
        "the cursor did not move"
    );
}
#[test]
fn every_row_carries_tenant_local_and_a_member() {
    let s = store();
    s.post(Seat::new("r", "bob"), "x", 1).unwrap();
    s.claim(&ClaimRequest::new(("r", "bob"), "src", 10, ""), 1)
        .unwrap();
    let count = |sql: &str| -> i64 { s.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT COUNT(*) FROM messages WHERE tenant='local' AND member=sender"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM claims WHERE tenant='local' AND member=owner"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM members WHERE tenant='local'"),
        1
    );
}
#[test]
fn the_cursor_advances_to_the_last_message_returned_never_past_it() {
    // A message that arrives between the select and the cursor update would be lost if the cursor
    // jumped to the room's current maximum. The cursor must stop at the last message returned.
    let s = store();
    s.join("r", "alice", 1).unwrap();
    s.post(Seat::new("r", "bob"), "one", 2).unwrap(); // seq 1
    s.post(Seat::new("r", "alice"), "mine", 3).unwrap(); // seq 2, her own
    let got = s.read(Seat::new("r", "alice"), true, 4).unwrap();
    assert_eq!(got.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1]);
    let cursor: i64 = s
        .conn
        .query_row(
            "SELECT last_seq FROM members WHERE room='r' AND name='alice'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cursor, 1, "cursor must equal the last returned seq");
    s.post(Seat::new("r", "bob"), "two", 5).unwrap(); // seq 3
    assert_eq!(s.read(Seat::new("r", "alice"), true, 6).unwrap()[0].seq, 3);
}
#[test]
fn a_database_from_before_handoffs_gains_the_columns_and_keeps_its_messages() {
    let path = temp_db("handoff-migrate");
    drop(SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch(
            "INSERT INTO messages(room,seq,mid,sender,member,body,ts,attribution,label) VALUES('r',1,'m1','a','a','old',1,'unattributed','public');
             ALTER TABLE messages DROP COLUMN kind; ALTER TABLE messages DROP COLUMN payload; ALTER TABLE claims DROP COLUMN anchor; PRAGMA user_version = 20;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    let m = &s.tail("r", 0, 10).unwrap()[0];
    assert_eq!(
        (m.body.as_str(), m.kind.as_deref(), m.payload.as_deref()),
        ("old", None, None)
    );
}
