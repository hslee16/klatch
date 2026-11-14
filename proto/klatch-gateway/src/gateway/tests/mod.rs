//! Tests of the gateway, by topic.

use super::*;
use crate::test_support::{SpyGuard, SpyStore};
use crate::{AllowAll, FixedClock, Operation};
use klatch_core::Error;
use std::rc::Rc;

mod admin_acts;
mod asks;
mod board;
mod budget;
mod claims;
mod digest;
mod guarding;
mod members;
mod stream;

pub(super) fn alice() -> Principal {
    Principal::unattributed("alice")
}
pub(super) fn bob() -> Principal {
    Principal::unattributed("bob")
}
pub(super) fn open() -> Gateway<SpyStore> {
    Gateway::new(SpyStore::new(), FixedClock(100), AllowAll)
}

pub(super) struct TestClock(pub(super) Rc<std::cell::Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.get()
    }
}

pub(super) fn timed() -> (Gateway<SpyStore>, Rc<std::cell::Cell<i64>>) {
    let t = Rc::new(std::cell::Cell::new(1_000));
    (
        Gateway::new(SpyStore::new(), TestClock(t.clone()), AllowAll),
        t,
    )
}

pub(super) fn admin() -> Principal {
    Principal::unattributed("admin")
}

pub(super) fn rule_of(e: klatch_core::Error) -> String {
    match e {
        Error::Refused { rule, .. } => rule,
        other => panic!("not a refusal: {other}"),
    }
}
