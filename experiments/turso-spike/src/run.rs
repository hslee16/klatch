//! The SQLite store's conformance wiring (`proto/klatch/src/store/sqlite/tests/conformance.rs`), one `#[test]` per
//! behaviour, included unchanged and pointed at `TursoStore`.
use crate::TursoStore;
#[allow(unused_imports)]
use klatch::store::areas::*;

fn store() -> TursoStore {
    TursoStore::for_test()
}

#[path = "../../../proto/klatch/src/store/sqlite/tests/conformance.rs"]
mod conformance_tests;
