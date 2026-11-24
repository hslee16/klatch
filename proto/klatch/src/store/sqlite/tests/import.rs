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
    // Source-level: the method is named only in its trait, its SQLite implementation and these tests.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    let mut stack = vec![src.clone()];
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
                    path.strip_prefix(&src)
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
            "store/area/import.rs",
            "store/sqlite/api/import.rs",
            "store/sqlite/tests/import.rs"
        ],
        "{hits:?}"
    );
}
