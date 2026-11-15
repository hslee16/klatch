//! Anchored claims (#125): a claim may name the work item it is for, and is released when that item is reported done.

use super::*;

#[test]
fn a_database_from_before_anchors_gains_the_column_and_its_claims_have_none() {
    let path = temp_db("anchor-migrate");
    drop(SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap());
    {
        let c = keyed(&path);
        c.execute_batch(
            "INSERT INTO claims(room,path,owner,member,reason,expires,scope) VALUES('r','p','w','w','',9999999999,'');
             ALTER TABLE claims DROP COLUMN anchor; PRAGMA user_version = 21;",
        )
        .unwrap();
    }
    let s = SqliteStore::open(&path, &crate::store::insecure_test_key()).unwrap();
    assert_eq!(s.claims("r", 1).unwrap().len(), 1);
    assert!(s.anchors_in_use(1).unwrap().is_empty());
}
