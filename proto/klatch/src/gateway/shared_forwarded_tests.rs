//! The server's note of ignored forwarded headers through the shared gateway (#1490): the backend of the listener hands
//! the owner thread the text, and the owner's answer, a refusal included, comes back to the caller.

use super::shared::SharedGateway;
use crate::domain::Principal;
use crate::gateway::areas::Admin;
use crate::gateway::{AllowAll, Gateway, SystemClock};
use crate::session::Backend;
use crate::store::SqliteStore;
use crate::store::test_dir::temp_db;

#[test]
fn a_note_the_owner_refuses_is_an_error_to_the_caller_and_a_good_one_is_kept() {
    let db = temp_db("fwd-shared");
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        Ok(Gateway::new(store, SystemClock, AllowAll))
    })
    .unwrap();
    let backend = gw.session_backend();
    assert!(
        backend
            .note_forwarded_ignored("requests=1 x-forwarded-for=203.0.113.9")
            .is_err(),
        "a refusal must reach the caller, so the counts stay unkept"
    );
    backend
        .note_forwarded_ignored("requests=2 x-real-ip=2")
        .unwrap();
    let rows = gw
        .transport(Principal::verified("human"))
        .admin_audit(10)
        .unwrap();
    let kept: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "forwarded-ignored")
        .collect();
    assert_eq!(kept.len(), 1, "{rows:?}");
    assert_eq!(kept[0].detail, "requests=2 x-real-ip=2");
}

#[test]
fn the_audit_page_reaches_the_shared_transport_with_the_note_left_out_and_counted() {
    let db = temp_db("fwd-shared-page");
    let path = db.to_path_buf();
    let gw = SharedGateway::spawn(move || {
        let store = SqliteStore::open(&path, &crate::store::insecure_test_key())?;
        Ok(Gateway::new(store, SystemClock, AllowAll))
    })
    .unwrap();
    let t = gw.transport(Principal::verified("human"));
    t.admin_pause(true).unwrap();
    gw.session_backend()
        .note_forwarded_ignored("requests=2 x-real-ip=2")
        .unwrap();
    let page = t.admin_audit_page(10, None).unwrap();
    assert_eq!(page.hidden, 1);
    assert!(page.rows.iter().all(|r| r.action != "forwarded-ignored"));
    assert_eq!(page.rows.len(), 1, "{:?}", page.rows);
    let only = t.admin_audit_page(10, Some("forwarded-ignored")).unwrap();
    assert_eq!((only.rows.len(), only.hidden), (1, 0));
}
