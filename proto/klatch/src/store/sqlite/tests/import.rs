//! The storage-level import (#1217): a message comes in with its id, seq and time, and only a holder of the concrete
//! store can do it.

use super::*;
use crate::domain::Attribution;
use crate::store::ImportStore;

fn source_message(id: &str, seq: i64, ts: i64) -> Message {
    Message {
        id: id.to_string(),
        seq,
        sender: "bob".to_string(),
        attribution: Attribution::Verified,
        label: "internal".to_string(),
        body: format!("body {seq}"),
        ts,
        kind: Some("handoff".to_string()),
        payload: Some("opaque".to_string()),
        advisory: false,
        scope: None,
        refs: Vec::new(),
    }
}

#[test]
fn an_imported_message_keeps_its_id_seq_time_sender_label_and_handoff() {
    let s = store();
    let posted = s
        .import_message("r", &source_message("OLDID1", 1, 777))
        .unwrap();
    assert_eq!((posted.id.as_str(), posted.seq), ("OLDID1", 1));
    assert_eq!(
        s.tail("r", 0, 10).unwrap(),
        vec![source_message("OLDID1", 1, 777)]
    );
}

#[test]
fn imports_must_arrive_in_dense_seq_order() {
    let s = store();
    assert!(matches!(
        s.import_message("r", &source_message("A1", 2, 1)),
        Err(Error::Invalid(_))
    ));
    s.import_message("r", &source_message("A1", 1, 1)).unwrap();
    assert!(matches!(
        s.import_message("r", &source_message("A2", 1, 2)),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        s.import_message("r", &source_message("A3", 3, 2)),
        Err(Error::Invalid(_))
    ));
    s.import_message("r", &source_message("A2", 2, 2)).unwrap();
    assert_eq!(s.head("r").unwrap(), 2);
}

#[test]
fn a_duplicate_id_is_refused_even_in_another_room_and_nothing_is_written() {
    let s = store();
    s.import_message("r", &source_message("DUP", 1, 1)).unwrap();
    assert!(matches!(
        s.import_message("other", &source_message("DUP", 1, 2)),
        Err(Error::Invalid(_))
    ));
    assert_eq!(s.head("other").unwrap(), 0);
    let minted = s.post(Seat::new("r", "bob"), "x", 3).unwrap();
    assert!(matches!(
        s.import_message("q", &source_message(&minted.id, 1, 4)),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn a_bad_id_or_label_is_refused() {
    let s = store();
    for id in ["", "has space", "semi;colon"] {
        assert!(matches!(
            s.import_message("r", &source_message(id, 1, 1)),
            Err(Error::Invalid(_))
        ));
    }
    let mut m = source_message("OK", 1, 1);
    m.label = String::new();
    assert!(matches!(s.import_message("r", &m), Err(Error::Invalid(_))));
    assert_eq!(s.head("r").unwrap(), 0);
}

#[test]
fn an_import_does_not_make_the_sender_a_member_and_a_normal_post_continues_the_seq() {
    let s = store();
    s.import_message("r", &source_message("A1", 1, 5)).unwrap();
    assert!(s.members("r").unwrap().is_empty());
    let next = s.post(Seat::new("r", "carol"), "after", 9).unwrap();
    assert_eq!(next.seq, 2);
}

#[test]
fn the_import_is_not_part_of_the_store_bound_so_the_gateway_and_server_cannot_call_it() {
    // Compile-time shape: `Store` does not require `ImportStore`; the memory store is a `Store` without it.
    fn only_store<S: crate::store::Store>(_: &S) {}
    only_store(&crate::store::memory_spike::MemoryStore::default());
    // Source-level: the method is named only in its trait (every crate under proto/ is scanned), its SQLite implementation and these tests.
    let proto = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    let mut stack: Vec<_> = std::fs::read_dir(&proto)
        .unwrap()
        .flatten()
        .map(|e| e.path().join("src"))
        .filter(|p| p.is_dir())
        .collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs")
                && std::fs::read_to_string(&path)
                    .unwrap()
                    .contains("import_message")
            {
                hits.push(
                    path.strip_prefix(&proto)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    hits.sort();
    assert_eq!(
        hits,
        [
            "klatch/src/store/area/import.rs",
            "klatch/src/store/sqlite/api/import.rs",
            "klatch/src/store/sqlite/tests/import.rs"
        ],
        "{hits:?}"
    );
}

#[test]
fn the_id_limit_is_64_characters_inclusive() {
    let s = store();
    assert!(matches!(
        s.import_message("r", &source_message(&"A".repeat(65), 1, 1)),
        Err(Error::Invalid(_))
    ));
    s.import_message("r", &source_message(&"A".repeat(64), 1, 1))
        .unwrap();
}

#[test]
fn a_sealed_label_is_sealed_at_rest_on_import_and_opens_back_to_the_original() {
    let s = store();
    s.policy_set("sealed:secret", Some("1"), 1).unwrap();
    s.set_label_keys(
        crate::keys::LabelKeys::new().with("secret", crate::keys::Key::from_bytes([0x33; 32])),
    );
    let mut m = source_message("SEALED1", 1, 5);
    m.label = "secret".to_string();
    m.body = "the-sealed-launch-code".to_string();
    m.payload = Some("the-sealed-payload".to_string());
    s.import_message("r", &m).unwrap();
    let (body, payload): (String, String) = s
        .conn
        .query_row(
            "SELECT body,payload FROM messages WHERE mid='SEALED1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        body.starts_with("klatch-sealed:1:") && !body.contains("launch-code"),
        "{body}"
    );
    assert!(
        payload.starts_with("klatch-sealed:1:") && !payload.contains("sealed-payload"),
        "{payload}"
    );
    assert_eq!(s.tail("r", 0, 10).unwrap(), vec![m]);
}
